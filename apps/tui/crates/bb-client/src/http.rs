use std::sync::{Arc, RwLock};
use std::time::Duration;

use bb_wire::routes::{API_PREFIX, HEALTH_PATH, Method, Route};
use bb_wire::system::HealthResponse;
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderName, HeaderValue};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::watch;

use crate::auth::Auth;
use crate::endpoint::ServerEndpoint;
use crate::error::{ClientError, HttpError};

pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(75);
pub const APP_SURFACE_HEADER: &str = "x-bb-app-surface";
pub const APP_SURFACE: &str = "api";
const JSON_CONTENT_TYPE: &str = "application/json";

#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub timeout: Duration,
    pub app_surface: String,
    pub user_agent: String,
    pub follow_server_moves: bool,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_REQUEST_TIMEOUT,
            app_surface: APP_SURFACE.into(),
            user_agent: concat!("bb-tui/", env!("CARGO_PKG_VERSION")).into(),
            follow_server_moves: true,
        }
    }
}

#[derive(Debug, Clone)]
pub enum RequestBody {
    Empty,
    Json(Vec<u8>),
    Multipart(MultipartFile),
}

#[derive(Debug, Clone)]
pub struct MultipartFile {
    pub field: String,
    pub file_name: String,
    pub mime_type: Option<String>,
    pub bytes: Vec<u8>,
}

impl MultipartFile {
    fn form(&self) -> Result<reqwest::multipart::Form, ClientError> {
        let mut part =
            reqwest::multipart::Part::bytes(self.bytes.clone()).file_name(self.file_name.clone());
        if let Some(mime) = &self.mime_type {
            part = part
                .mime_str(mime)
                .map_err(|_| ClientError::InvalidHeader(format!("invalid mime type {mime}")))?;
        }
        Ok(reqwest::multipart::Form::new().part(self.field.clone(), part))
    }
}

#[derive(Debug, Clone)]
pub struct RawRequest {
    pub method: Method,
    pub path: String,
    pub query: String,
    pub body: RequestBody,
}

impl RawRequest {
    pub fn api(route: &Route) -> Self {
        Self {
            method: route.method,
            path: route.api_path(),
            query: String::new(),
            body: RequestBody::Empty,
        }
    }

    pub fn with_query<Q: Serialize + ?Sized>(mut self, query: &Q) -> Result<Self, ClientError> {
        self.query = serde_urlencoded::to_string(query)
            .map_err(|error| ClientError::InvalidUrl(format!("invalid query: {error}")))?;
        Ok(self)
    }

    pub fn with_json<B: Serialize + ?Sized>(mut self, body: &B) -> Result<Self, ClientError> {
        let bytes = serde_json::to_vec(body).map_err(|source| ClientError::Decode {
            route: self.path.clone(),
            source,
        })?;
        self.body = RequestBody::Json(bytes);
        Ok(self)
    }

    fn label(&self) -> String {
        format!("{} {}", self.method, self.path)
    }
}

struct Inner {
    http: reqwest::Client,
    config: ClientConfig,
    endpoint: RwLock<ServerEndpoint>,
    endpoint_tx: watch::Sender<ServerEndpoint>,
    auth: Auth,
}

#[derive(Clone)]
pub struct BbClient {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for BbClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BbClient")
            .field("endpoint", &self.endpoint().to_string())
            .field("auth", &self.inner.auth)
            .finish()
    }
}

impl BbClient {
    pub fn new(endpoint: ServerEndpoint, auth: Auth) -> Result<Self, ClientError> {
        Self::with_config(endpoint, auth, ClientConfig::default())
    }

    pub fn local(endpoint: ServerEndpoint) -> Result<Self, ClientError> {
        Self::new(endpoint, Auth::None)
    }

    pub fn with_config(
        endpoint: ServerEndpoint,
        auth: Auth,
        config: ClientConfig,
    ) -> Result<Self, ClientError> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            HeaderName::from_static(APP_SURFACE_HEADER),
            HeaderValue::from_str(&config.app_surface)
                .map_err(|_| ClientError::InvalidHeader(config.app_surface.clone()))?,
        );
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .user_agent(config.user_agent.clone())
            .timeout(config.timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(ClientError::Transport)?;
        let (endpoint_tx, _) = watch::channel(endpoint.clone());
        Ok(Self {
            inner: Arc::new(Inner {
                http,
                config,
                endpoint: RwLock::new(endpoint),
                endpoint_tx,
                auth,
            }),
        })
    }

    pub fn endpoint(&self) -> ServerEndpoint {
        self.inner
            .endpoint
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn set_endpoint(&self, endpoint: ServerEndpoint) {
        *self
            .inner
            .endpoint
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = endpoint.clone();
        self.inner.endpoint_tx.send_replace(endpoint);
    }

    pub fn watch_endpoint(&self) -> watch::Receiver<ServerEndpoint> {
        self.inner.endpoint_tx.subscribe()
    }

    pub fn auth(&self) -> &Auth {
        &self.inner.auth
    }

    pub fn config(&self) -> &ClientConfig {
        &self.inner.config
    }

    pub async fn execute(&self, request: &RawRequest) -> Result<reqwest::Response, ClientError> {
        let mut moved = false;
        let mut reauthorized = false;
        loop {
            let response = self.send_once(request).await?;
            let status = response.status().as_u16();
            if response.status().is_success() {
                return Ok(response);
            }
            if (status == 401 || status == 403)
                && !reauthorized
                && self.inner.auth.reject(&request.path).await
            {
                reauthorized = true;
                continue;
            }
            let error = read_http_error(response).await;
            if let Some(details) = error.server_moved() {
                if !moved
                    && self.inner.config.follow_server_moves
                    && let Ok(next) = ServerEndpoint::parse(&details.server_url)
                    && next != self.endpoint()
                {
                    tracing::info!(to = %next, "bb server moved; switching endpoint");
                    self.set_endpoint(next);
                    moved = true;
                    continue;
                }
                return Err(ClientError::ServerMoved(Box::new(details)));
            }
            return Err(ClientError::Http(Box::new(error)));
        }
    }

    async fn send_once(&self, request: &RawRequest) -> Result<reqwest::Response, ClientError> {
        let endpoint = self.endpoint();
        let mut url = endpoint.url_for(&request.path);
        if !request.query.is_empty() {
            url.set_query(Some(&request.query));
        }
        let method = match request.method {
            Method::Get => reqwest::Method::GET,
            Method::Post => reqwest::Method::POST,
            Method::Put => reqwest::Method::PUT,
            Method::Patch => reqwest::Method::PATCH,
            Method::Delete => reqwest::Method::DELETE,
        };
        let mut builder = self
            .inner
            .http
            .request(method, url.clone())
            .header(ACCEPT, JSON_CONTENT_TYPE)
            .headers(self.inner.auth.http_headers(&request.path).await?);
        builder = match &request.body {
            RequestBody::Multipart(file) => builder.multipart(file.form()?),
            RequestBody::Json(bytes) => builder
                .header(CONTENT_TYPE, JSON_CONTENT_TYPE)
                .body(bytes.clone()),
            RequestBody::Empty if request.method.is_mutation() => {
                builder.header(CONTENT_TYPE, JSON_CONTENT_TYPE)
            }
            RequestBody::Empty => builder,
        };
        builder.send().await.map_err(|error| {
            ClientError::from_reqwest(&endpoint.to_string(), self.inner.config.timeout, error)
        })
    }

    pub async fn execute_json<T: DeserializeOwned>(
        &self,
        request: &RawRequest,
    ) -> Result<T, ClientError> {
        let response = self.execute(request).await?;
        let bytes = response.bytes().await.map_err(|error| {
            ClientError::from_reqwest(&request.path, self.inner.config.timeout, error)
        })?;
        let slice: &[u8] = if bytes.iter().all(u8::is_ascii_whitespace) {
            b"null"
        } else {
            &bytes
        };
        serde_json::from_slice(slice).map_err(|source| ClientError::Decode {
            route: request.label(),
            source,
        })
    }

    pub async fn execute_bytes(&self, request: &RawRequest) -> Result<Vec<u8>, ClientError> {
        let response = self.execute(request).await?;
        response
            .bytes()
            .await
            .map(|bytes| bytes.to_vec())
            .map_err(|error| {
                ClientError::from_reqwest(&request.path, self.inner.config.timeout, error)
            })
    }

    pub async fn execute_text(&self, request: &RawRequest) -> Result<String, ClientError> {
        let response = self.execute(request).await?;
        response.text().await.map_err(|error| {
            ClientError::from_reqwest(&request.path, self.inner.config.timeout, error)
        })
    }

    pub async fn get<T: DeserializeOwned>(&self, route: Route) -> Result<T, ClientError> {
        self.execute_json(&RawRequest::api(&route)).await
    }

    pub async fn get_with<T, Q>(&self, route: Route, query: &Q) -> Result<T, ClientError>
    where
        T: DeserializeOwned,
        Q: Serialize + ?Sized,
    {
        self.execute_json(&RawRequest::api(&route).with_query(query)?)
            .await
    }

    pub async fn call<T: DeserializeOwned>(&self, route: Route) -> Result<T, ClientError> {
        self.execute_json(&RawRequest::api(&route)).await
    }

    pub async fn call_with<T, B>(&self, route: Route, body: &B) -> Result<T, ClientError>
    where
        T: DeserializeOwned,
        B: Serialize + ?Sized,
    {
        self.execute_json(&RawRequest::api(&route).with_json(body)?)
            .await
    }

    pub async fn get_bytes(&self, route: Route) -> Result<Vec<u8>, ClientError> {
        self.execute_bytes(&RawRequest::api(&route)).await
    }

    pub async fn health(&self) -> Result<HealthResponse, ClientError> {
        let request = RawRequest {
            method: Method::Get,
            path: HEALTH_PATH.into(),
            query: String::new(),
            body: RequestBody::Empty,
        };
        self.execute_json(&request).await
    }

    pub fn api_url(&self, route: &Route) -> url::Url {
        self.endpoint()
            .url_for(&format!("{API_PREFIX}{}", route.path))
    }
}

async fn read_http_error(response: reqwest::Response) -> HttpError {
    let status = response.status();
    let status_text = status.canonical_reason().unwrap_or_default().to_owned();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let raw = response.text().await.unwrap_or_default();
    HttpError::parse(status.as_u16(), &status_text, content_type.as_deref(), &raw)
}
