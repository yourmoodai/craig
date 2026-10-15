//! Bounded response framing shared by both worker transports.
use serde_json::{Value, json};

pub const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024; // 16 MiB, including newline

pub fn response_bytes(value: &Value) -> Vec<u8> {
    let mut bytes = value.to_string().into_bytes();
    if bytes.len() >= MAX_RESPONSE_BYTES {
        bytes = json!({"version":1,"status":"error","code":"review_response_too_large"})
            .to_string()
            .into_bytes();
    }
    bytes.push(b'\n');
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framing_limit_includes_newline() {
        for (length, accepted) in [
            (MAX_RESPONSE_BYTES - 3, true),
            (MAX_RESPONSE_BYTES - 2, false),
        ] {
            let value = Value::String("x".repeat(length));
            let bytes = response_bytes(&value);
            assert!(bytes.len() <= MAX_RESPONSE_BYTES);
            assert_eq!(bytes.last(), Some(&b'\n'));
            let decoded: Value = serde_json::from_slice(&bytes).unwrap();
            if accepted {
                assert_eq!(decoded, value);
            } else {
                assert_eq!(decoded["code"], "review_response_too_large");
            }
        }
    }
}
