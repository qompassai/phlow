//! Credential redaction for model-visible and retained text.
//!
//! Contract: a provider-issued token — one of [`CREDENTIAL_PREFIXES`] at a
//! token boundary, followed by at least [`CREDENTIAL_BODY_BYTES_MIN`] token
//! bytes (`[A-Za-z0-9_-]`) — is replaced by [`REDACTED`]. Everything else is
//! preserved byte-for-byte, and clean text is returned borrowed (no copy).
//! Work is linear in input bytes; JSON walks use an explicit stack, never
//! recursion. Object keys are not rewritten: only string values are.
//!
//! This is a pattern scanner, not a broker: it cannot recognize credentials
//! without a known prefix, and it never learns or stores the values it sees.

use std::borrow::Cow;

use serde_json::Value;

/// Replacement for every recognized credential.
pub const REDACTED: &str = "[REDACTED]";

/// Prefixes of provider-issued API tokens (OpenAI/Anthropic, GitHub).
const CREDENTIAL_PREFIXES: [&str; 3] = ["sk-", "ghp_", "github_pat_"];

/// Minimum token bytes after the prefix. Shorter runs (`sk-learn`) are
/// ordinary words, not credentials.
const CREDENTIAL_BODY_BYTES_MIN: usize = 16;

fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'
}

/// Byte length of the credential starting at `start`, if one starts there.
fn credential_len(bytes: &[u8], start: usize) -> Option<usize> {
    assert!(start < bytes.len(), "scan start must be in bounds");
    // `task-...` contains `sk-` but is one word: require a token boundary.
    if start > 0 && is_token_byte(bytes[start - 1]) {
        return None;
    }
    let rest = &bytes[start..];
    let prefix = CREDENTIAL_PREFIXES
        .iter()
        .find(|prefix| rest.starts_with(prefix.as_bytes()))?;
    let body_bytes = rest[prefix.len()..]
        .iter()
        .take_while(|byte| is_token_byte(**byte))
        .count();
    (body_bytes >= CREDENTIAL_BODY_BYTES_MIN).then_some(prefix.len() + body_bytes)
}

/// Replace every recognized credential in `text` with [`REDACTED`].
///
/// Returns `Cow::Borrowed(text)` when nothing matched. Match boundaries are
/// ASCII bytes, so every slice below falls on a UTF-8 char boundary.
pub fn redact_credentials(text: &str) -> Cow<'_, str> {
    let bytes = text.as_bytes();
    let mut redacted: Option<String> = None;
    let mut copied_bytes = 0;
    let mut index = 0;
    while index < bytes.len() {
        match credential_len(bytes, index) {
            Some(len) => {
                let out = redacted.get_or_insert_with(|| String::with_capacity(text.len()));
                out.push_str(&text[copied_bytes..index]);
                out.push_str(REDACTED);
                index += len;
                copied_bytes = index;
            }
            None => index += 1,
        }
    }
    match redacted {
        None => Cow::Borrowed(text),
        Some(mut out) => {
            out.push_str(&text[copied_bytes..]);
            Cow::Owned(out)
        }
    }
}

/// Redact every string value inside `value`, in place, at any depth.
pub fn redact_value(value: &mut Value) {
    let mut pending: Vec<&mut Value> = vec![value];
    while let Some(item) = pending.pop() {
        match item {
            Value::String(text) => {
                let clean = match redact_credentials(text) {
                    Cow::Borrowed(_) => None,
                    Cow::Owned(clean) => Some(clean),
                };
                if let Some(clean) = clean {
                    *text = clean;
                }
            }
            Value::Array(items) => pending.extend(items.iter_mut()),
            Value::Object(fields) => pending.extend(fields.values_mut()),
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TOKEN: &str = "sk-test-0123456789abcdefghij";

    #[test]
    fn clean_text_is_borrowed_unchanged() {
        for text in [
            "",
            "{}",
            "task-list-for-the-quarterly-review",
            "use sk-learn",
            "é sk-",
        ] {
            assert!(matches!(redact_credentials(text), Cow::Borrowed(t) if t == text));
        }
    }

    #[test]
    fn surrounding_text_is_preserved() {
        let text = format!("héllo\nAuthorization: Bearer {TOKEN}\nbye");
        assert_eq!(
            redact_credentials(&text),
            "héllo\nAuthorization: Bearer [REDACTED]\nbye"
        );
    }

    #[test]
    fn every_prefix_and_repeat_is_redacted() {
        let text =
            format!("a={TOKEN} b=ghp_0123456789abcdefXYZ c=github_pat_0123456789abcdef {TOKEN}");
        let clean = redact_credentials(&text);
        assert_eq!(clean, "a=[REDACTED] b=[REDACTED] c=[REDACTED] [REDACTED]");
    }

    #[test]
    fn json_encoded_inside_string_is_redacted() {
        let inner = json!({"headers": {"Authorization": format!("Bearer {TOKEN}")}}).to_string();
        let clean = redact_credentials(&inner);
        assert!(!clean.contains(TOKEN));
        assert!(
            serde_json::from_str::<Value>(&clean).is_ok(),
            "JSON must stay valid"
        );
    }

    #[test]
    fn nested_value_strings_are_redacted_keys_and_scalars_kept() {
        let mut value = json!({"a": [1, true, null, {"k": TOKEN}], "path": "README.md"});
        redact_value(&mut value);
        assert_eq!(
            value,
            json!({"a": [1, true, null, {"k": REDACTED}], "path": "README.md"})
        );
    }

    #[test]
    fn deep_nesting_does_not_recurse() {
        let mut value = Value::String(TOKEN.to_owned());
        for _ in 0..10_000 {
            value = Value::Array(vec![value]);
        }
        redact_value(&mut value);
        let mut cursor = &value;
        while let Value::Array(items) = cursor {
            cursor = &items[0];
        }
        assert_eq!(cursor, &Value::String(REDACTED.to_owned()));
        // Drop iteratively too: serde_json's recursive Drop would overflow.
        let mut next = Some(value);
        while let Some(Value::Array(mut items)) = next.take() {
            next = items.pop();
        }
    }
}
