//! Shared topic wire encoding/decoding.

use anyhow::{Result, bail};

const SUBJECT_SEPARATOR: char = '.';

fn is_reserved_char(ch: char) -> bool {
    matches!(ch, '.' | '*' | '>' | '%')
}

fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Encode one logical token for wire transport.
pub fn encode_token(token: &str) -> String {
    let mut out = String::with_capacity(token.len());
    for ch in token.chars() {
        if is_reserved_char(ch) {
            out.push('%');
            out.push_str(&format!("{:02X}", ch as u32));
        } else {
            out.push(ch);
        }
    }
    out
}

/// Decode one wire token back to logical form.
pub fn decode_token(token: &str) -> Result<String> {
    let bytes = token.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                bail!(
                    "Invalid percent-encoding: trailing '%' in token '{}'",
                    token
                );
            }
            let hi = from_hex(bytes[i + 1]).ok_or_else(|| {
                anyhow::anyhow!(
                    "Invalid percent-encoding: non-hex '{}' in token '{}'",
                    bytes[i + 1] as char,
                    token
                )
            })?;
            let lo = from_hex(bytes[i + 2]).ok_or_else(|| {
                anyhow::anyhow!(
                    "Invalid percent-encoding: non-hex '{}' in token '{}'",
                    bytes[i + 2] as char,
                    token
                )
            })?;
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }

    String::from_utf8(out)
        .map_err(|e| anyhow::anyhow!("Decoded token is not valid UTF-8 ('{}'): {}", token, e))
}

/// Encode logical topic parts to a wire subject.
pub fn encode_subject(parts: &[String]) -> String {
    let encoded_parts = parts.iter().map(|p| encode_token(p)).collect::<Vec<_>>();
    encoded_parts.join(&SUBJECT_SEPARATOR.to_string())
}

/// Decode wire subject into logical topic parts.
pub fn decode_subject(subject: &str) -> Result<Vec<String>> {
    subject.split(SUBJECT_SEPARATOR).map(decode_token).collect()
}

/// Decode first token of a subject.
pub fn decode_subject_base(subject: &str) -> Result<String> {
    let raw = subject
        .split(SUBJECT_SEPARATOR)
        .next()
        .ok_or_else(|| anyhow::anyhow!("Subject cannot be empty"))?;
    if raw.is_empty() {
        bail!("Subject cannot be empty");
    }
    decode_token(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_roundtrip_reserved_chars() {
        let raw = "1.45*foo>bar%baz";
        let encoded = encode_token(raw);
        assert_eq!(encoded, "1%2E45%2Afoo%3Ebar%25baz");
        let decoded = decode_token(&encoded).unwrap();
        assert_eq!(decoded, raw);
    }

    #[test]
    fn token_roundtrip_examples() {
        assert_eq!(encode_token("1.45"), "1%2E45");
        assert_eq!(decode_token("1%2E45").unwrap(), "1.45");

        assert_eq!(encode_token("1*34"), "1%2A34");
        assert_eq!(decode_token("1%2A34").unwrap(), "1*34");

        assert_eq!(encode_token("1%25"), "1%2525");
        assert_eq!(decode_token("1%2525").unwrap(), "1%25");

        // Raw wire token decodes once as expected.
        assert_eq!(decode_token("1%25").unwrap(), "1%");
    }

    #[test]
    fn subject_roundtrip() {
        let parts = vec![
            "diss".to_string(),
            "FOO".to_string(),
            "1.59342".to_string(),
            "a*b".to_string(),
            "p%q".to_string(),
        ];

        let wire = encode_subject(&parts);
        assert_eq!(wire, "diss.FOO.1%2E59342.a%2Ab.p%25q");

        let decoded = decode_subject(&wire).unwrap();
        assert_eq!(decoded, parts);
    }

    #[test]
    fn rejects_invalid_percent_sequences() {
        assert!(decode_token("%").is_err());
        assert!(decode_token("%2").is_err());
        assert!(decode_token("%2G").is_err());
        assert!(decode_token("abc%ZZ").is_err());
    }

    #[test]
    fn decode_subject_base_decodes_first_token() {
        let subject = "diss%2Ev2.FOO";
        assert_eq!(decode_subject_base(subject).unwrap(), "diss.v2");
    }
}
