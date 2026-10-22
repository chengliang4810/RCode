//! Local identity and bearer provider for the engine.
//!
//! [`AuthState`] and [`AuthUser`] carry the local identity on the RPC wire. The
//! engine creates a fixed identity and exposes a constant configured bearer
//! through [`rcode_rpc::TokenSource`].

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use rcode_rpc::TokenError;

// ---------------------------------------------------------------------------
// Wire types (ARCHITECTURE.md AuthRpc)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthUser {
    pub id: String,
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrgMembership {
    pub id: String,
    pub organization_id: String,
    pub name: String,
}

/// AuthStatus stream payload (`SignedOut | NeedsOrganization{user} |
/// SignedIn{user, orgId?}`). Serializes as the canonical [`rcode_proto::AuthState`]
/// wire shape (`{"state": "signedIn", …}`) so every client parses one form.
#[derive(Debug, Clone, PartialEq)]
pub enum AuthState {
    SignedOut,
    NeedsOrganization {
        user: AuthUser,
    },
    SignedIn {
        user: AuthUser,
        org_id: Option<String>,
    },
}

impl AuthState {
    pub fn is_signed_in(&self) -> bool {
        matches!(self, AuthState::SignedIn { .. })
    }

    pub fn org_id(&self) -> Option<&str> {
        match self {
            AuthState::SignedIn { org_id, .. } => org_id.as_deref(),
            _ => None,
        }
    }

    pub fn user(&self) -> Option<&AuthUser> {
        match self {
            AuthState::SignedIn { user, .. } | AuthState::NeedsOrganization { user } => Some(user),
            AuthState::SignedOut => None,
        }
    }

    /// The proto wire twin — the one shape the engine emits over AuthStatus.
    pub fn to_proto(&self) -> rcode_proto::AuthState {
        let profile = |user: &AuthUser| rcode_proto::UserProfile {
            id: user.id.clone(),
            email: user.email.clone(),
            name: user.name.clone(),
        };
        match self {
            AuthState::SignedOut => rcode_proto::AuthState::SignedOut,
            AuthState::NeedsOrganization { user } => rcode_proto::AuthState::NeedsOrganization {
                user: profile(user),
            },
            AuthState::SignedIn { user, org_id } => rcode_proto::AuthState::SignedIn {
                user: profile(user),
                org_id: org_id.clone(),
            },
        }
    }
}

impl Serialize for AuthState {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_proto().serialize(serializer)
    }
}

// ---------------------------------------------------------------------------
// Config + construction
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AuthConfig {
    /// Data dir for any persisted local state.
    pub data_dir: std::path::PathBuf,
    /// The device's local user id (also its bearer — there is no network).
    pub dev_user_id: String,
}

impl AuthConfig {
    pub fn new(data_dir: impl Into<std::path::PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            dev_user_id: "dev-user".into(),
        }
    }
}

/// The auth service — cheap to clone by `Arc`.
#[derive(Clone)]
pub struct Auth {
    inner: Arc<AuthInner>,
}

struct AuthInner {
    config: AuthConfig,
    state_tx: watch::Sender<AuthState>,
    token_tx: watch::Sender<u64>,
}

impl Auth {
    /// Build the local session: always signed in as the configured device user.
    pub fn new(config: AuthConfig) -> Self {
        let user = AuthUser {
            id: config.dev_user_id.clone(),
            email: config.dev_user_id.clone(),
            name: None,
        };
        let (state_tx, _) = watch::channel(AuthState::SignedIn { user, org_id: None });
        let (token_tx, _) = watch::channel(0);
        Self {
            inner: Arc::new(AuthInner {
                config,
                state_tx,
                token_tx,
            }),
        }
    }

    /// The signed-in user id — the identity that scopes local storage. A
    /// `user@org` token yields `user`, matching the old dev-bearer parsing.
    pub fn user_id(&self) -> Option<String> {
        let dev = &self.inner.config.dev_user_id;
        Some(dev.split('@').next().unwrap_or(dev).to_string())
    }

    /// The bearer. Local-only: the configured user id, no network, no refresh.
    pub async fn access_token(&self) -> Result<String, TokenError> {
        Ok(self.inner.config.dev_user_id.clone())
    }

    /// No-op: there is no token to refresh locally.
    pub fn retry_refresh(&self) {}

    /// No-op task: nothing to keep fresh locally.
    pub fn spawn_refresh_loop(&self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async {})
    }

    /// Live auth status (current value + changes).
    pub fn watch_state(&self) -> watch::Receiver<AuthState> {
        self.inner.state_tx.subscribe()
    }

    pub fn state(&self) -> AuthState {
        self.inner.state_tx.borrow().clone()
    }
}

/// The relay/room token seam: `Auth` IS a [`rcode_rpc::TokenSource`], so the
/// host relay and link cache always dial with the current bearer.
#[async_trait::async_trait]
impl rcode_rpc::TokenSource for Auth {
    async fn token(&self) -> Result<String, TokenError> {
        self.access_token().await
    }

    fn subscribe(&self) -> Option<watch::Receiver<u64>> {
        Some(self.inner.token_tx.subscribe())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local() -> Auth {
        Auth::new(AuthConfig::new(std::env::temp_dir()))
    }

    #[tokio::test]
    async fn local_session_is_always_signed_in_as_the_device_user() {
        let auth = local();
        assert!(auth.state().is_signed_in());
        assert_eq!(auth.user_id().as_deref(), Some("dev-user"));
        assert_eq!(auth.access_token().await.unwrap(), "dev-user");
        assert_eq!(
            serde_json::to_value(auth.state()).unwrap(),
            serde_json::json!({
                "state": "signedIn",
                "user": {"id": "dev-user", "email": "dev-user", "name": null},
                "orgId": null,
            })
        );
    }

    #[test]
    fn user_id_splits_a_user_at_org_token() {
        let auth = Auth::new(AuthConfig::new(std::env::temp_dir()));
        // `dev_user_id` is the configured bearer; an explicit one carries `@org`.
        let mut config = AuthConfig::new(std::env::temp_dir());
        config.dev_user_id = "alice@acme".into();
        let auth2 = Auth::new(config);
        assert_eq!(auth2.user_id().as_deref(), Some("alice"));
        // The plain case is unchanged.
        assert_eq!(auth.user_id().as_deref(), Some("dev-user"));
    }

    #[test]
    fn proto_round_trips_the_emitted_value() {
        let user = AuthUser {
            id: "u1".into(),
            email: "u@x".into(),
            name: None,
        };
        let signed_in = AuthState::SignedIn {
            user: user.clone(),
            org_id: Some("org_1".into()),
        };
        let value = serde_json::to_value(&signed_in).expect("json");
        assert_eq!(
            value,
            serde_json::json!({
                "state": "signedIn",
                "user": {"id": "u1", "email": "u@x", "name": null},
                "orgId": "org_1",
            })
        );
        let parsed: rcode_proto::AuthState = serde_json::from_value(value).expect("proto parse");
        assert!(matches!(parsed, rcode_proto::AuthState::SignedIn { .. }));
        assert_eq!(
            serde_json::to_value(AuthState::SignedOut).expect("json"),
            serde_json::json!({"state": "signedOut"})
        );
        assert_eq!(
            serde_json::to_value(AuthState::NeedsOrganization { user }).expect("json"),
            serde_json::json!({
                "state": "needsOrganization",
                "user": {"id": "u1", "email": "u@x", "name": null},
            })
        );
    }
}
