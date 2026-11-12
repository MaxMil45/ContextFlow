//! Deterministic snapshot hashing.
//!
//! The hash is computed over a canonical JSON encoding: object keys sorted
//! lexicographically at every level, no insignificant whitespace. The same
//! payload therefore always produces the same bytes and the same hash,
//! independent of struct field order or map insertion order.

use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::Result;

/// Encode `value` as canonical JSON.
pub fn canonical_json<T: Serialize>(value: &T) -> Result<String> {
    let value = canonicalize(serde_json::to_value(value)?);
    Ok(serde_json::to_string(&value)?)
}

fn canonicalize(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            let mut out = Map::new();
            for (k, v) in entries {
                out.insert(k, canonicalize(v));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(canonicalize).collect()),
        other => other,
    }
}

/// Lower-case hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Canonical hash of any serialisable value.
pub fn content_hash<T: Serialize>(value: &T) -> Result<String> {
    Ok(sha256_hex(canonical_json(value)?.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_order_does_not_change_hash() {
        let a = json!({"b": 1, "a": {"y": [1, 2], "x": null}});
        let b = json!({"a": {"x": null, "y": [1, 2]}, "b": 1});
        assert_eq!(content_hash(&a).unwrap(), content_hash(&b).unwrap());
    }

    #[test]
    fn array_order_does_change_hash() {
        let a = json!([1, 2]);
        let b = json!([2, 1]);
        assert_ne!(content_hash(&a).unwrap(), content_hash(&b).unwrap());
    }

    #[test]
    fn canonical_form_is_compact_and_sorted() {
        let v = json!({"z": 1, "a": {"c": true, "b": "x"}});
        assert_eq!(
            canonical_json(&v).unwrap(),
            r#"{"a":{"b":"x","c":true},"z":1}"#
        );
    }

    #[test]
    fn known_sha256_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
