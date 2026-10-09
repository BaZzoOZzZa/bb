use std::sync::Arc;

use reqwest::header::{COOKIE, HeaderMap, HeaderName, HeaderValue};

use crate::ClientError;
use crate::connect::{ConnectCredential, ConnectSession, MACHINE_CREDENTIAL_HEADER};

#[derive(Debug, Clone, Default)]
pub enum Auth {
    #[default]
    None,
    Connect(Arc<ConnectSession>),
}

impl Auth {
    pub fn connect(credential: ConnectCredential) -> Self {
        Self::Connect(Arc::new(ConnectSession::new(credential)))
    }

    pub fn connect_session(&self) -> Option<&Arc<ConnectSession>> {
        match self {
            Self::None => None,
            Self::Connect(session) => Some(session),
        }
    }

    pub(crate) async fn http_headers(&self, path: &str) -> Result<HeaderMap, ClientError> {
        let mut headers = HeaderMap::new();
        let Self::Connect(session) = self else {
            return Ok(headers);
        };
        if is_machine_path(path) {
            headers.insert(
                HeaderName::from_static(MACHINE_CREDENTIAL_HEADER),
                header_value(session.machine_credential())?,
            );
        } else {
            headers.insert(COOKIE, header_value(&session.cookie_header().await?)?);
        }
        Ok(headers)
    }

    pub(crate) async fn websocket_headers(&self) -> Result<HeaderMap, ClientError> {
        let mut headers = HeaderMap::new();
        if let Self::Connect(session) = self {
            headers.insert(COOKIE, header_value(&session.cookie_header().await?)?);
        }
        Ok(headers)
    }

    pub(crate) async fn reject(&self, path: &str) -> bool {
        match self {
            Self::Connect(session) if !is_machine_path(path) => {
                session.invalidate().await;
                true
            }
            _ => false,
        }
    }
}

pub fn is_machine_path(path: &str) -> bool {
    path == "/api/v1" || path.starts_with("/api/v1/")
}

fn header_value(raw: &str) -> Result<HeaderValue, ClientError> {
    let mut value = HeaderValue::from_str(raw).map_err(|_| {
        ClientError::InvalidHeader("credential contains invalid header bytes".into())
    })?;
    value.set_sensitive(true);
    Ok(value)
}
