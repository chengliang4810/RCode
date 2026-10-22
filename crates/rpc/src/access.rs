//! Connection credentials for the optional external RPC listener.

use crate::RpcError;

/// Validate a user-supplied connection key without including it in errors.
pub fn validate_api_key(value: &str) -> Result<(), RpcError> {
    if value.len() < 8
        || !value.bytes().all(|byte| byte.is_ascii_alphanumeric())
        || !value.bytes().any(|byte| byte.is_ascii_alphabetic())
        || !value.bytes().any(|byte| byte.is_ascii_digit())
    {
        return Err(RpcError::BadParams(
            "API key must contain at least 8 ASCII letters and digits, including both a letter and a digit".into(),
        ));
    }
    Ok(())
}

/// Generate twelve characters from OS-random UUID bytes. Rejection sampling
/// gives every character the same probability; both character classes occur.
pub fn generate_api_key() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    loop {
        let mut key = String::with_capacity(12);
        while key.len() < 12 {
            let random = uuid::Uuid::new_v4();
            // Skip UUID's version/variant bytes, which contain fixed bits.
            for &byte in random.as_bytes()[..6].iter().chain(&random.as_bytes()[9..]) {
                if byte < 248 {
                    key.push(ALPHABET[usize::from(byte % 62)] as char);
                    if key.len() == 12 {
                        break;
                    }
                }
            }
        }
        if validate_api_key(&key).is_ok() {
            return key;
        }
    }
}

/// Compare equal-length credential values without early exits on their bytes.
pub(crate) fn key_matches(actual: &[u8], expected: &[u8]) -> bool {
    if actual.len() != expected.len() {
        return false;
    }
    actual
        .iter()
        .zip(expected)
        .fold(0_u8, |diff, (left, right)| diff | (left ^ right))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_keys_have_the_requested_length_and_character_classes() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..256 {
            let key = generate_api_key();
            assert_eq!(key.len(), 12);
            validate_api_key(&key).unwrap();
            assert!(seen.insert(key));
        }
    }

    #[test]
    fn rejects_short_single_class_and_non_ascii_keys_without_echoing_them() {
        for value in [
            "abc1234",
            "abcdefgh",
            "12345678",
            "abc12345!",
            "密钥abc12345",
        ] {
            let error = validate_api_key(value).unwrap_err().to_string();
            assert!(!error.contains(value));
        }
        validate_api_key("Abcd1234").unwrap();
    }
}
