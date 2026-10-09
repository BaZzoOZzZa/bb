use std::time::Duration;

use reqwest::header::{HeaderValue, USER_AGENT};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::{self, Error as WsError};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use url::Url;

use crate::error::ClientError;
use crate::http::{APP_SURFACE_HEADER, BbClient};

pub(crate) type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Debug)]
pub(crate) enum WsOpenError {
    Rejected { status: u16, retried_auth: bool },
    Failed(String),
}

impl WsOpenError {
    pub(crate) fn describe(&self) -> String {
        match self {
            Self::Rejected { status, .. } => {
                format!("websocket upgrade rejected with HTTP {status}")
            }
            Self::Failed(message) => message.clone(),
        }
    }
}

impl From<ClientError> for WsOpenError {
    fn from(error: ClientError) -> Self {
        Self::Failed(error.to_string())
    }
}

pub(crate) async fn open_websocket(
    client: &BbClient,
    url: &Url,
    timeout: Duration,
) -> Result<WsStream, WsOpenError> {
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|error| WsOpenError::Failed(error.to_string()))?;
    let headers = request.headers_mut();
    if let Ok(value) = HeaderValue::from_str(&client.config().app_surface) {
        headers.insert(APP_SURFACE_HEADER, value);
    }
    if let Ok(value) = HeaderValue::from_str(&client.config().user_agent) {
        headers.insert(USER_AGENT, value);
    }
    for (name, value) in client.auth().websocket_headers().await?.iter() {
        headers.insert(name, value.clone());
    }
    let connect = tokio_tungstenite::connect_async(request);
    match tokio::time::timeout(timeout, connect).await {
        Err(_) => Err(WsOpenError::Failed(format!(
            "websocket connect timed out after {} seconds",
            timeout.as_secs()
        ))),
        Ok(Ok((stream, _))) => Ok(stream),
        Ok(Err(WsError::Http(response))) => {
            let status = response.status().as_u16();
            let retried_auth =
                (status == 401 || status == 403) && client.auth().reject(url.path()).await;
            Err(WsOpenError::Rejected {
                status,
                retried_auth,
            })
        }
        Ok(Err(error)) => Err(WsOpenError::Failed(error.to_string())),
    }
}

pub(crate) fn text_message(json: String) -> tungstenite::Message {
    tungstenite::Message::text(json)
}
