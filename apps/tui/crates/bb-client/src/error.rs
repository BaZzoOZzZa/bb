use std::fmt;
use std::time::Duration;

use bb_wire::ApiError;
use bb_wire::JsonValue;
use bb_wire::error::ServerMovedDetails;

use crate::connect::ConnectError;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("invalid server url: {0}")]
    InvalidUrl(String),
    #[error(
        "cannot connect to bb server at {url}; ensure it is running and the server url is correct"
    )]
    Unreachable {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("bb request timed out after {} seconds", .0.as_secs())]
    Timeout(Duration),
    #[error("{0}")]
    Http(Box<HttpError>),
    #[error("bb server moved to {}", .0.server_url)]
    ServerMoved(Box<ServerMovedDetails>),
    #[error("invalid response from {route}: {source}")]
    Decode {
        route: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid header: {0}")]
    InvalidHeader(String),
    #[error("plugin error: {0}")]
    Plugin(String),
    #[error("bb request failed: {0}")]
    Transport(#[source] reqwest::Error),
    #[error("websocket error: {0}")]
    WebSocket(String),
    #[error(transparent)]
    Connect(#[from] ConnectError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl ClientError {
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Http(error) => Some(error.status),
            _ => None,
        }
    }

    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Http(error) => error.code.as_deref(),
            _ => None,
        }
    }

    pub fn api_error(&self) -> Option<&ApiError> {
        match self {
            Self::Http(error) => error.api.as_ref(),
            _ => None,
        }
    }

    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Http(error) => error.api.as_ref().is_some_and(ApiError::is_retryable),
            Self::Unreachable { .. } | Self::Timeout(_) | Self::WebSocket(_) => true,
            _ => false,
        }
    }

    pub(crate) fn from_reqwest(url: &str, timeout: Duration, error: reqwest::Error) -> Self {
        if error.is_timeout() {
            Self::Timeout(timeout)
        } else if error.is_connect() {
            Self::Unreachable {
                url: url.to_owned(),
                source: error,
            }
        } else {
            Self::Transport(error)
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HttpError {
    pub status: u16,
    pub code: Option<String>,
    pub message: String,
    pub api: Option<ApiError>,
    pub body: Option<JsonValue>,
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HTTP {}: {}", self.status, self.message)
    }
}

impl HttpError {
    pub fn parse(status: u16, status_text: &str, content_type: Option<&str>, raw: &str) -> Self {
        let normalized = normalize_whitespace(raw);
        if normalized.is_empty() {
            return Self::plain(status, status_text.to_owned());
        }
        let looks_json = content_type.is_some_and(|value| value.contains("application/json"))
            || normalized.starts_with('{')
            || normalized.starts_with('[');
        if !looks_json {
            let message = if normalized.starts_with('<') {
                if status_text.is_empty() {
                    format!("Request failed with status {status}")
                } else {
                    status_text.to_owned()
                }
            } else {
                normalized
            };
            return Self::plain(status, message);
        }
        let Ok(parsed) = serde_json::from_str::<JsonValue>(&normalized) else {
            return Self::plain(status, normalized);
        };
        let code = parsed
            .get("code")
            .and_then(JsonValue::as_str)
            .map(str::to_owned);
        let message = extract_error_message(&parsed).unwrap_or_else(|| normalized.clone());
        let api = code
            .as_ref()
            .and_then(|_| serde_json::from_value::<ApiError>(parsed.clone()).ok());
        Self {
            status,
            code,
            message,
            api,
            body: Some(parsed),
        }
    }

    fn plain(status: u16, message: String) -> Self {
        Self {
            status,
            code: None,
            message,
            api: None,
            body: None,
        }
    }

    pub fn server_moved(&self) -> Option<ServerMovedDetails> {
        if self.status != 410 {
            return None;
        }
        self.api.as_ref()?.server_moved_details()
    }
}

fn normalize_whitespace(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn extract_error_message(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::String(text) => {
            let normalized = normalize_whitespace(text);
            (!normalized.is_empty()).then_some(normalized)
        }
        JsonValue::Array(items) => items.iter().find_map(extract_error_message),
        JsonValue::Object(record) => {
            if let Some(message @ JsonValue::String(_)) = record.get("message")
                && let Some(found) = extract_error_message(message)
            {
                return Some(found);
            }
            ["detail", "error"]
                .iter()
                .filter_map(|key| record.get(*key))
                .find_map(extract_error_message)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_api_error_envelope() {
        let body = r#"{"code":"thread_not_writable","message":"Thread is archived","details":{"reason":"archived","threadStatus":"idle"},"retryable":false}"#;
        let error = HttpError::parse(409, "Conflict", Some("application/json"), body);
        assert_eq!(error.code.as_deref(), Some("thread_not_writable"));
        assert_eq!(error.message, "Thread is archived");
        let api = error.api.expect("api error");
        assert!(!api.is_retryable());
        assert!(api.lifecycle_details().is_some());
    }

    #[test]
    fn falls_back_to_legacy_error_keys() {
        let error = HttpError::parse(
            404,
            "Not Found",
            None,
            r#"{"ok":false,"error":{"code":"unknown_method","message":"no rpc method"}}"#,
        );
        assert_eq!(error.message, "no rpc method");
        assert_eq!(error.code, None);
        let error = HttpError::parse(400, "Bad Request", None, r#"{"detail":"  bad\n input "}"#);
        assert_eq!(error.message, "bad input");
    }

    #[test]
    fn handles_empty_html_and_text_bodies() {
        assert_eq!(
            HttpError::parse(502, "Bad Gateway", None, "  ").message,
            "Bad Gateway"
        );
        assert_eq!(
            HttpError::parse(502, "Bad Gateway", Some("text/html"), "<html>oops</html>").message,
            "Bad Gateway"
        );
        assert_eq!(
            HttpError::parse(503, "", Some("text/html"), "<html></html>").message,
            "Request failed with status 503"
        );
        assert_eq!(
            HttpError::parse(
                403,
                "Forbidden",
                Some("text/plain"),
                "bb connect: machine not authorized\n"
            )
            .message,
            "bb connect: machine not authorized"
        );
        assert_eq!(
            HttpError::parse(500, "Error", None, "{not json").message,
            "{not json"
        );
    }

    #[test]
    fn recognizes_server_moved() {
        let body = r#"{"code":"server_moved","message":"moved","details":{"serverUrl":"http://10.0.0.2:38886","toHostName":"studio","movedAt":5}}"#;
        let error = HttpError::parse(410, "Gone", Some("application/json"), body);
        let details = error.server_moved().expect("server moved");
        assert_eq!(details.server_url, "http://10.0.0.2:38886");
        assert_eq!(details.to_host_name, "studio");
        assert!(
            HttpError::parse(409, "", None, body)
                .server_moved()
                .is_none()
        );
    }
}
