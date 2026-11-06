//! Signed, expiring URLs (spec sections 3.3 and 8.2).
//!
//! `sig = HMAC-SHA256(server_secret, canonical)`, where the canonical string
//! is the path, then `?`, then every query parameter except `sig`, sorted by
//! key, as `k=v` joined with `&`. `exp` is a Unix timestamp. Anyone holding
//! the URL can fetch it until it expires; it renders the item as it is when
//! fetched, not when signed.

use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

fn canonical(path: &str, params: &BTreeMap<String, String>) -> String {
    let query: Vec<String> = params
        .iter()
        .filter(|(k, _)| k.as_str() != "sig")
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    format!("{path}?{}", query.join("&"))
}

fn mac(secret: &[u8], text: &str) -> HmacSha256 {
    let mut m = HmacSha256::new_from_slice(secret).expect("HMAC takes any key length");
    m.update(text.as_bytes());
    m
}

/// The signature for `path` with `params` (which should include `exp`).
pub fn sign(secret: &[u8], path: &str, params: &BTreeMap<String, String>) -> String {
    URL_SAFE_NO_PAD.encode(
        mac(secret, &canonical(path, params))
            .finalize()
            .into_bytes(),
    )
}

/// A full signed path: `path?…&exp=…&sig=…`.
pub fn signed_path(
    secret: &[u8],
    path: &str,
    mut params: BTreeMap<String, String>,
    exp: i64,
) -> String {
    params.insert("exp".into(), exp.to_string());
    let sig = sign(secret, path, &params);
    params.insert("sig".into(), sig);
    let query: Vec<String> = params
        .iter()
        .map(|(k, v)| format!("{k}={}", urlencode(v)))
        .collect();
    format!("{path}?{}", query.join("&"))
}

#[derive(Debug, PartialEq, Eq)]
pub enum Invalid {
    Missing,
    Expired,
    BadSignature,
}

/// Check a request's signature (constant-time) and expiry.
pub fn verify(
    secret: &[u8],
    path: &str,
    params: &BTreeMap<String, String>,
    now: i64,
) -> Result<(), Invalid> {
    let sig = params.get("sig").ok_or(Invalid::Missing)?;
    let exp: i64 = params
        .get("exp")
        .and_then(|e| e.parse().ok())
        .ok_or(Invalid::Missing)?;
    let given = URL_SAFE_NO_PAD
        .decode(sig)
        .map_err(|_| Invalid::BadSignature)?;
    mac(secret, &canonical(path, params))
        .verify_slice(&given)
        .map_err(|_| Invalid::BadSignature)?;
    if exp <= now {
        return Err(Invalid::Expired);
    }
    Ok(())
}

fn urlencode(v: &str) -> String {
    v.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn signed_urls_verify_until_they_expire() {
        let secret = b"0123456789abcdef0123456789abcdef";
        let url = signed_path(secret, "/i/abc.png", params(&[("scale", "2")]), 1_000);
        let (path, query) = url.split_once('?').unwrap();
        let got: BTreeMap<String, String> = query
            .split('&')
            .map(|kv| kv.split_once('=').unwrap())
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        assert_eq!(verify(secret, path, &got, 999), Ok(()));
        assert_eq!(verify(secret, path, &got, 1_000), Err(Invalid::Expired));

        let mut tampered = got.clone();
        tampered.insert("scale".into(), "4".into());
        assert_eq!(
            verify(secret, path, &tampered, 999),
            Err(Invalid::BadSignature)
        );
        assert_eq!(
            verify(secret, "/i/other.png", &got, 999),
            Err(Invalid::BadSignature)
        );
        assert_eq!(
            verify(b"another secret, also 32 bytes!!", path, &got, 999),
            Err(Invalid::BadSignature)
        );

        let mut unsigned = got.clone();
        unsigned.remove("sig");
        assert_eq!(verify(secret, path, &unsigned, 999), Err(Invalid::Missing));
    }
}
