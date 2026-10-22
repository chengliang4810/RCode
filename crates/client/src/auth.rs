//! The configured per-client bearer and the shared transport HTTP client.
//!
//! A paired client's bearer is `userId@orgId`. The caller supplies the host and
//! pairing configuration.

use std::sync::OnceLock;
use std::time::Duration;

use crate::error::{ClientError, Result};

pub(crate) fn http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest client")
    })
}

/// How this client authenticates. There is no account: a client is pointed at
/// a host and carries the bearer that host was told to expect.
enum Mode {
    Demo,
    Paired { bearer: String },
}

/// Per-client bearer source: a value the caller supplied.
pub(crate) struct TokenProvider {
    mode: Mode,
}

impl TokenProvider {
    pub(crate) fn new(credentials: &crate::config::Credentials) -> Self {
        use crate::config::Credentials;
        let mode = match credentials {
            Credentials::Demo(_) => Mode::Demo,
            Credentials::Direct { api_key } => Mode::Paired { bearer: api_key.clone() },
            Credentials::Paired { user_id, org_id } => Mode::Paired {
                bearer: if org_id.is_empty() {
                    user_id.clone()
                } else {
                    format!("{user_id}@{org_id}")
                },
            },
        };
        Self { mode }
    }

    /// Current bearer. It is whatever the caller configured — there is no
    /// account and no token to rotate.
    pub(crate) async fn bearer(&self) -> Result<String> {
        match &self.mode {
            Mode::Demo => Err(ClientError::Auth(
                "demo mode has no host credentials".into(),
            )),
            Mode::Paired { bearer } => Ok(bearer.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_bearer_is_the_user_at_org_the_host_expects() {
        let credentials = crate::config::Credentials::Paired {
            user_id: "wing".into(),
            org_id: "acme".into(),
        };
        let tokens = TokenProvider::new(&credentials);
        assert_eq!(tokens.bearer().await.unwrap(), "wing@acme");
    }

    #[tokio::test]
    async fn direct_credentials_keep_the_opaque_key_and_redact_debug_output() {
        let credentials = crate::config::Credentials::Direct { api_key: "FixtureKey12".into() };
        assert_eq!(TokenProvider::new(&credentials).bearer().await.unwrap(), "FixtureKey12");
        assert!(!format!("{credentials:?}").contains("FixtureKey12"));
        assert_eq!(credentials.user_id(), "direct");
        assert_eq!(credentials.org_id(), "direct");
    }
}
