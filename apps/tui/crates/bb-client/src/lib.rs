use url::Url;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("invalid server url: {0}")]
    InvalidUrl(String),
}

#[derive(Debug, Clone)]
pub struct ServerEndpoint {
    base: Url,
}

impl ServerEndpoint {
    pub fn parse(raw: &str) -> Result<Self, ClientError> {
        let base = Url::parse(raw).map_err(|err| ClientError::InvalidUrl(err.to_string()))?;
        match base.scheme() {
            "http" | "https" => Ok(Self { base }),
            other => Err(ClientError::InvalidUrl(format!(
                "unsupported scheme {other}"
            ))),
        }
    }

    pub fn base(&self) -> &Url {
        &self.base
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
}
