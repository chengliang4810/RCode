//! Full attachment identity shared by upload storage and thumbnail aliases.

use sha2::{Digest, Sha256};

/// UUID suffix understood by new hosts as prohibiting short-id fallback.
/// Appending preserves the first eight UUID characters for legacy hosts.
pub const UPLOAD_ID_V2_SUFFIX: &str = "-v2";

pub fn is_full_identity_upload(upload_id: &str) -> bool {
    upload_id.ends_with(UPLOAD_ID_V2_SUFFIX)
}

/// Stable identity for the full upload id AND the original, unsanitized name.
pub fn upload_identity_hash(upload_id: &str, file_name: &str) -> String {
    let mut identity = Sha256::new();
    // Frame both inputs so ("ab", "c") and ("a", "bc") cannot alias.
    identity.update((upload_id.len() as u64).to_le_bytes());
    identity.update(upload_id.as_bytes());
    identity.update(file_name.as_bytes());
    format!("{:x}", identity.finalize())
}

/// Read the full identity from a committed basename without guessing a short
/// legacy id. Both chip display and thumbnail aliases use this exact shape.
pub fn split_full_upload_name(name: &str) -> Option<(&str, &str)> {
    let (hash, file_name) = name.strip_prefix("upload-")?.split_once('-')?;
    (hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()) && !file_name.is_empty())
        .then_some((hash, file_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_identity_has_a_wire_compatible_suffix_without_losing_uuid_entropy() {
        let uuid = "12345678-1234-4567-89ab-123456789abc";
        let full = format!("{uuid}{UPLOAD_ID_V2_SUFFIX}");
        assert_eq!(full.len(), 39);
        assert_eq!(&full[..8], &uuid[..8]);
        assert!(full.len() <= 64);
        assert!(
            full.bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        );
        assert!(is_full_identity_upload(&full));
        assert!(!is_full_identity_upload(uuid));
    }

    #[test]
    fn hash_frames_inputs_and_keeps_every_original_identity_component() {
        assert_ne!(
            upload_identity_hash("ab", "c"),
            upload_identity_hash("a", "bc")
        );
        assert_ne!(
            upload_identity_hash("abcdefgh-first", "image.png"),
            upload_identity_hash("abcdefgh-second", "image.png")
        );
        assert_ne!(
            upload_identity_hash("same", "im age.png"),
            upload_identity_hash("same", "im_age.png")
        );
        let hash = upload_identity_hash("12345678-1234-4567-89ab-123456789abc-v2", "image.png");
        assert_eq!(hash.len(), 64);
        assert!(hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(
            hash,
            upload_identity_hash("12345678-1234-4567-89ab-123456789abc-v2", "image.png")
        );
    }

    #[test]
    fn full_upload_name_parser_does_not_guess_short_or_malformed_aliases() {
        let hash = upload_identity_hash("upload-v2", "notes.zip");
        let name = format!("upload-{hash}-notes.zip");
        assert_eq!(
            split_full_upload_name(&name),
            Some((hash.as_str(), "notes.zip"))
        );
        for name in [
            "abcd1234-notes.zip",
            "upload-short-notes.zip",
            "upload-notes.zip",
        ] {
            assert_eq!(split_full_upload_name(name), None);
        }
        assert!(split_full_upload_name(&format!("upload-{}-notes.zip", "g".repeat(64))).is_none());
        assert!(split_full_upload_name(&format!("upload-{hash}-")).is_none());
    }
}
