//! Stable conversation links shared by sidebar copy actions and inbound URL routing.

use rcode_proto::{AuthState, Chat, HarnessId};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationDeepLink {
    pub chat_id: String,
    pub workspace: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessConversationLink {
    pub label: &'static str,
    pub url: String,
}

/// Opaque locator: enough to reject links for a different workspace without
/// putting user or organization ids in the URL.
///
/// `None` until the identity is known, which is the signal a caller must not
/// consume the pending link yet: rejecting a link because the engine has not
/// reported its identity would tell the user their link belongs to someone
/// else's workspace.
pub fn workspace_locator(auth: Option<&AuthState>) -> Option<String> {
    let AuthState::SignedIn { user, org_id } = auth? else {
        return None;
    };
    let identity = format!(
        "user:{}:org:{}",
        user.id,
        org_id.as_deref().unwrap_or("personal")
    );
    let mut hash = Sha256::new();
    hash.update(identity);
    Some(format!("{:x}", hash.finalize())[..16].to_string())
}

pub fn rcode_conversation_link(chat_id: &str, workspace: &str) -> String {
    format!(
        "rcode://open/chat/{}?workspace={}",
        encode_component(chat_id),
        encode_component(workspace)
    )
}

pub fn parse_rcode_conversation_link(url: &str) -> Result<ConversationDeepLink, &'static str> {
    let rest = url
        .strip_prefix("rcode://open/chat/")
        .ok_or("not a RCode conversation link")?;
    let (chat_id, query) = rest.split_once('?').ok_or("missing workspace locator")?;
    if chat_id.is_empty() || chat_id.contains('/') {
        return Err("invalid conversation id");
    }
    let workspace = query
        .split('&')
        .find_map(|part| part.strip_prefix("workspace="))
        .ok_or("missing workspace locator")?;
    Ok(ConversationDeepLink {
        chat_id: decode_component(chat_id)?,
        workspace: decode_component(workspace)?,
    })
}

/// Only return schemes verified against the harness app. Hermes exposes a
/// candidate scheme, but its contract is not stable enough to put on users'
/// clipboards yet.
pub fn harness_conversation_link(chat: &Chat) -> Option<HarnessConversationLink> {
    let id = chat.harness_session_id.as_deref()?.trim();
    if id.is_empty() || chat.config.as_ref()?.harness != HarnessId::Codex {
        return None;
    }
    Some(HarnessConversationLink {
        label: "Codex conversation link",
        url: format!("codex://threads/{}", encode_component(id)),
    })
}

fn encode_component(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            use std::fmt::Write as _;
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

fn decode_component(value: &str) -> Result<String, &'static str> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let encoded = bytes
                .get(index + 1..index + 3)
                .ok_or("invalid URL escape")?;
            let text = std::str::from_utf8(encoded).map_err(|_| "invalid URL escape")?;
            out.push(u8::from_str_radix(text, 16).map_err(|_| "invalid URL escape")?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).map_err(|_| "invalid UTF-8 in URL")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn harness_chat(harness: HarnessId) -> Chat {
        Chat {
            id: "chat".into(),
            device_id: "device".into(),
            title: None,
            archived: false,
            cwd: None,
            branch: None,
            checkout_id: None,
            source_context: None,
            config: Some(rcode_proto::ChatConfig {
                harness,
                model: None,
                reasoning: None,
                model_options: Default::default(),
            }),
            last_message_preview: None,
            last_message_at: None,
            created_at: chrono::DateTime::UNIX_EPOCH,
            harness_session_id: Some("thread/one".into()),
            harness_session_cwd: None,
            parent_chat_id: None,
            space_id: None,
            last_seen_at: None,
            room_gen: None,
        }
    }

    #[test]
    fn conversation_link_round_trips_reserved_characters() {
        let link = rcode_conversation_link("chat/with space", "workspace:one");
        assert_eq!(
            parse_rcode_conversation_link(&link).unwrap(),
            ConversationDeepLink {
                chat_id: "chat/with space".into(),
                workspace: "workspace:one".into(),
            }
        );
    }

    #[test]
    fn malformed_or_foreign_links_are_rejected() {
        assert!(parse_rcode_conversation_link("https://example.com").is_err());
        assert!(parse_rcode_conversation_link("rcode://open/chat/id").is_err());
        assert!(parse_rcode_conversation_link("rcode://open/chat/%GG?workspace=x").is_err());
    }

    /// The locator waits for an identity, then separates workspaces. Both
    /// halves matter: it is what a foreign link is rejected against, and `None`
    /// means "hold the link", not "reject it".
    #[test]
    fn workspace_locator_waits_for_identity_then_separates_workspaces() {
        assert_eq!(workspace_locator(None), None);
        assert_eq!(workspace_locator(Some(&AuthState::SignedOut)), None);

        let user = |id: &str, org: Option<&str>| AuthState::SignedIn {
            user: rcode_proto::UserProfile {
                id: id.into(),
                email: format!("{id}@example.test"),
                name: None,
            },
            org_id: org.map(str::to_owned),
        };
        let first = workspace_locator(Some(&user("alice", Some("acme"))));
        let second = workspace_locator(Some(&user("bob", Some("acme"))));
        let third = workspace_locator(Some(&user("alice", Some("other"))));
        assert!(first.is_some());
        assert_ne!(first, second, "a different user is a different workspace");
        assert_ne!(first, third, "a different org is a different workspace");
        assert_eq!(
            first,
            workspace_locator(Some(&user("alice", Some("acme")))),
            "the same identity is the same workspace"
        );
        // An org-less identity is still locatable ("personal").
        assert!(workspace_locator(Some(&user("alice", None))).is_some());
    }

    #[test]
    fn codex_link_is_exact_and_unverified_harnesses_are_omitted() {
        assert_eq!(
            harness_conversation_link(&harness_chat(HarnessId::Codex))
                .unwrap()
                .url,
            "codex://threads/thread%2Fone"
        );
        assert!(harness_conversation_link(&harness_chat(HarnessId::Hermes)).is_none());
    }
}
