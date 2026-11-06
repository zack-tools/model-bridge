//! Redacts credential material before it can reach a log, diff, or report.

const KEYS: &[&str] = &[
    "access_token",
    "refresh_token",
    "id_token",
    "auth_code",
    "authorization",
    "api-key",
    "api_key",
    "secret-key",
    "secret_key",
    "password",
    "set-cookie",
    "cookie",
];

#[derive(Clone, Debug)]
pub struct Redactor {
    secrets: Vec<String>,
}

impl Redactor {
    pub fn new(secrets: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let secrets = secrets
            .into_iter()
            .map(Into::into)
            .filter(|s| s.len() >= 8)
            .collect();
        Self { secrets }
    }

    pub fn redact(&self, input: &str) -> String {
        let mut out = input.to_string();
        for secret in &self.secrets {
            if out.contains(secret) {
                out = out.replace(secret, "<redacted>");
            }
        }
        redact_query_values(&redact_keyed_values(&out))
    }
}

pub fn redact_text(input: &str) -> String {
    Redactor::new(Vec::<String>::new()).redact(input)
}

fn redact_keyed_values(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        if let Some((consumed_key, value_end)) = keyed_span(bytes, i) {
            out.push_str(&input[i..i + consumed_key]);
            out.push_str("<redacted>");
            i = value_end;
            continue;
        }
        let ch = input[i..].chars().next().unwrap_or('\u{fffd}');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn keyed_span(bytes: &[u8], i: usize) -> Option<(usize, usize)> {
    let rest = &bytes[i..];
    for key in KEYS {
        let key_bytes = key.as_bytes();
        if rest.len() < key_bytes.len() {
            continue;
        }
        if !rest[..key_bytes.len()].eq_ignore_ascii_case(key_bytes) {
            continue;
        }
        let after = i + key_bytes.len();
        if i > 0 && is_ident(bytes[i - 1]) {
            continue;
        }
        if after < bytes.len() && is_ident(bytes[after]) {
            continue;
        }
        let mut j = after;
        while j < bytes.len() && matches!(bytes[j], b' ' | b'\t' | b':' | b'=' | b'"' | b'\'') {
            j += 1;
        }
        if j == after {
            continue;
        }
        let line_bound = key.eq_ignore_ascii_case("authorization")
            || key.eq_ignore_ascii_case("cookie")
            || key.eq_ignore_ascii_case("set-cookie");
        let mut k = j;
        while k < bytes.len() {
            let b = bytes[k];
            if b == b'\n' || b == b'\r' {
                break;
            }
            if !line_bound && matches!(b, b',' | b'}' | b'&' | b' ' | b'\t' | b'"' | b'\'') {
                break;
            }
            k += 1;
        }
        if k == j {
            continue;
        }
        return Some((key_bytes.len(), k));
    }
    None
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

fn redact_query_values(input: &str) -> String {
    let mut out = input.to_string();
    for key in ["code", "state", "access_token", "refresh_token"] {
        out = redact_query_key(&out, key);
    }
    out
}

fn redact_query_key(input: &str, key: &str) -> String {
    let needle_eq = format!("{key}=");
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    let n = needle_eq.as_bytes();
    while i < bytes.len() {
        if bytes[i..].len() >= n.len() && bytes[i..i + n.len()].eq_ignore_ascii_case(n) {
            let boundary_ok = i == 0 || matches!(bytes[i - 1], b'?' | b'&' | b' ' | b'\t');
            if boundary_ok {
                out.push_str(&input[i..i + n.len()]);
                out.push_str("<redacted>");
                i += n.len();
                while i < bytes.len() && !matches!(bytes[i], b'&' | b' ' | b'\n' | b'\r' | b'"') {
                    i += 1;
                }
                continue;
            }
        }
        let ch = input[i..].chars().next().unwrap_or('\u{fffd}');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_tokens_headers_and_exact_secrets() {
        let r = Redactor::new(["gateway-key-0123456789"]);
        let raw = concat!(
            "access_token=supersecretvalue\n",
            "\"refresh_token\": \"another-secret\"\n",
            "Authorization: Bearer abc.def.ghi\n",
            "X-Api-Key: gateway-key-0123456789\n",
            "https://auth.example/callback?code=abc123&state=xyz789\n",
            "cookie: session=keep-out\n"
        );
        let clean = r.redact(raw);
        assert!(!clean.contains("supersecretvalue"));
        assert!(!clean.contains("another-secret"));
        assert!(!clean.contains("abc.def.ghi"));
        assert!(!clean.contains("gateway-key-0123456789"));
        assert!(!clean.contains("abc123"));
        assert!(!clean.contains("xyz789"));
        assert!(!clean.contains("session=keep-out"));
        assert!(clean.contains("<redacted>"));
    }
}
