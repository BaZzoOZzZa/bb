use std::fmt;

use url::Url;

use crate::ClientError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerEndpoint {
    base: Url,
}

impl ServerEndpoint {
    pub fn parse(raw: &str) -> Result<Self, ClientError> {
        let base = Url::parse(raw.trim())
            .map_err(|err| ClientError::InvalidUrl(format!("{raw}: {err}")))?;
        Self::from_url(base)
    }

    pub fn from_url(mut base: Url) -> Result<Self, ClientError> {
        match base.scheme() {
            "http" | "https" => {}
            other => {
                return Err(ClientError::InvalidUrl(format!(
                    "unsupported scheme {other}"
                )));
            }
        }
        if base.host_str().is_none_or(str::is_empty) {
            return Err(ClientError::InvalidUrl(format!("{base}: missing host")));
        }
        base.set_query(None);
        base.set_fragment(None);
        Ok(Self { base })
    }

    pub fn base(&self) -> &Url {
        &self.base
    }

    pub fn url_for(&self, path: &str) -> Url {
        let mut url = self.base.clone();
        let prefix = self.base.path().trim_end_matches('/');
        url.set_path(&format!("{prefix}{path}"));
        url
    }

    pub fn websocket_url(&self, path: &str) -> Url {
        let mut url = self.url_for(path);
        let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
        url.set_scheme(scheme)
            .unwrap_or_else(|()| unreachable!("http(s) urls accept ws schemes"));
        url
    }

    pub fn is_https(&self) -> bool {
        self.base.scheme() == "https"
    }
}

impl fmt::Display for ServerEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.base.as_str().trim_end_matches('/'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_http_schemes() {
        assert!(ServerEndpoint::parse("ftp://localhost").is_err());
        assert!(ServerEndpoint::parse("not a url").is_err());
        assert!(ServerEndpoint::parse("http://127.0.0.1:3000").is_ok());
    }

    #[test]
    fn joins_paths_and_preserves_prefix() {
        let endpoint = ServerEndpoint::parse("https://bee.getbb.app/prefix/?x=1#y").unwrap();
        assert_eq!(
            endpoint.url_for("/api/v1/threads").as_str(),
            "https://bee.getbb.app/prefix/api/v1/threads"
        );
        assert_eq!(
            endpoint.websocket_url("/ws").as_str(),
            "wss://bee.getbb.app/prefix/ws"
        );
        let local = ServerEndpoint::parse("http://127.0.0.1:38886").unwrap();
        assert_eq!(
            local.websocket_url("/ws/terminals/t%201").as_str(),
            "ws://127.0.0.1:38886/ws/terminals/t%201"
        );
        assert_eq!(local.to_string(), "http://127.0.0.1:38886");
    }
}
