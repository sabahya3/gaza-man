//! Auth schemes that can't be a fixed header: Digest and NTLM answer a server challenge,
//! and AWS Signature V4 signs the final request, body included.

use crate::crypto::{hex, hmac_md5, hmac_sha256, md4, md5, sha256};
use crate::http::base64;

pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    };
    let clean: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=').collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.chunks(4) {
        let mut n = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            n |= val(*c)? << (18 - 6 * i);
        }
        let bytes = n.to_be_bytes();
        out.extend_from_slice(&bytes[1..chunk.len()]);
    }
    Some(out)
}

/// `key="value", key2=value2` parameters, as in `WWW-Authenticate` headers.
fn auth_params(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = s.trim();
    while !rest.is_empty() {
        let Some(eq) = rest.find('=') else { break };
        let key = rest[..eq].trim().trim_start_matches(',').trim().to_ascii_lowercase();
        rest = rest[eq + 1..].trim_start();
        let value;
        if let Some(r) = rest.strip_prefix('"') {
            let mut v = String::new();
            let mut chars = r.char_indices();
            let mut end = r.len();
            while let Some((i, c)) = chars.next() {
                match c {
                    '\\' => {
                        if let Some((_, n)) = chars.next() {
                            v.push(n);
                        }
                    }
                    '"' => {
                        end = i + 1;
                        break;
                    }
                    c => v.push(c),
                }
            }
            value = v;
            rest = &r[end.min(r.len())..];
        } else {
            let end = rest.find(',').unwrap_or(rest.len());
            value = rest[..end].trim().to_string();
            rest = &rest[end..];
        }
        rest = rest.trim_start().trim_start_matches(',').trim_start();
        out.push((key, value));
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Digest (RFC 7616 / 2617)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct DigestChallenge {
    pub realm: String,
    pub nonce: String,
    pub qop: Option<String>,
    pub opaque: Option<String>,
    pub algorithm: String,
}

/// Finds a Digest challenge among `WWW-Authenticate` header values.
pub fn parse_digest_challenge<'a>(values: impl IntoIterator<Item = &'a str>) -> Option<DigestChallenge> {
    let v = values.into_iter().find(|v| v.trim_start().to_ascii_lowercase().starts_with("digest "))?;
    let params = auth_params(&v.trim_start()[7..]);
    let get = |k: &str| params.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
    Some(DigestChallenge {
        realm: get("realm").unwrap_or_default(),
        nonce: get("nonce")?,
        qop: get("qop"),
        opaque: get("opaque"),
        algorithm: get("algorithm").unwrap_or_else(|| "MD5".into()),
    })
}

/// The `Authorization` header answering a Digest challenge. `uri` is the path and query.
pub fn digest_authorization(ch: &DigestChallenge, user: &str, pass: &str, method: &str, uri: &str, cnonce: &str) -> String {
    let alg = ch.algorithm.to_ascii_uppercase();
    let h = |s: &str| if alg.starts_with("SHA-256") { hex(&sha256(s.as_bytes())) } else { hex(&md5(s.as_bytes())) };
    let mut ha1 = h(&format!("{user}:{}:{pass}", ch.realm));
    if alg.ends_with("-SESS") {
        ha1 = h(&format!("{ha1}:{}:{cnonce}", ch.nonce));
    }
    let ha2 = h(&format!("{method}:{uri}"));
    let qop = ch.qop.as_deref().and_then(|q| q.split(',').map(str::trim).find(|q| *q == "auth"));
    let nc = "00000001";
    let response = match qop {
        Some(q) => h(&format!("{ha1}:{}:{nc}:{cnonce}:{q}:{ha2}", ch.nonce)),
        None => h(&format!("{ha1}:{}:{ha2}", ch.nonce)),
    };
    let mut header = format!(
        "Digest username=\"{user}\", realm=\"{}\", nonce=\"{}\", uri=\"{uri}\", algorithm={}, response=\"{response}\"",
        ch.realm, ch.nonce, ch.algorithm
    );
    if let Some(q) = qop {
        header.push_str(&format!(", qop={q}, nc={nc}, cnonce=\"{cnonce}\""));
    }
    if let Some(o) = &ch.opaque {
        header.push_str(&format!(", opaque=\"{o}\""));
    }
    header
}

// ---------------------------------------------------------------------------------------------
// AWS Signature Version 4
// ---------------------------------------------------------------------------------------------

pub struct AwsCreds<'a> {
    pub access_key: &'a str,
    pub secret_key: &'a str,
    pub region: &'a str,
    pub service: &'a str,
    pub session_token: &'a str,
}

fn uri_encode(s: &str, keep_slash: bool) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b'/' if keep_slash => "/".into(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn aws_signing_key(secret: &str, date: &str, region: &str, service: &str) -> Vec<u8> {
    let k = hmac_sha256(format!("AWS4{secret}").as_bytes(), date.as_bytes());
    let k = hmac_sha256(&k, region.as_bytes());
    let k = hmac_sha256(&k, service.as_bytes());
    hmac_sha256(&k, b"aws4_request")
}

/// Adds the `X-Amz-*` and `Authorization` headers. `amz_date` is `YYYYMMDDTHHMMSSZ`.
pub fn aws_sign(method: &str, url: &str, headers: &mut Vec<(String, String)>, body: &[u8], c: &AwsCreds, amz_date: &str) {
    let (scheme, rest) = url.split_once("://").unwrap_or(("https", url));
    let rest = &rest[..rest.find('#').unwrap_or(rest.len())];
    let (authority, path_query) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let host = match (scheme, authority.rsplit_once(':')) {
        ("https", Some((h, "443"))) | ("http", Some((h, "80"))) => h,
        _ => authority,
    };
    let (path, query) = path_query.split_once('?').unwrap_or((path_query, ""));
    let s3 = c.service.eq_ignore_ascii_case("s3");
    // S3 signs the path as sent; other services encode it once more.
    let canonical_uri = if s3 { path.to_string() } else { uri_encode(path, true) };
    let mut pairs: Vec<(String, String)> = query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (uri_encode(&percent_decode(k), false), uri_encode(&percent_decode(v), false))
        })
        .collect();
    pairs.sort();
    let canonical_query = pairs.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");

    let payload_hash = hex(&sha256(body));
    headers.retain(|(k, _)| !k.eq_ignore_ascii_case("authorization") && !k.to_ascii_lowercase().starts_with("x-amz-"));
    headers.push(("X-Amz-Date".into(), amz_date.into()));
    if s3 {
        headers.push(("X-Amz-Content-Sha256".into(), payload_hash.clone()));
    }
    if !c.session_token.is_empty() {
        headers.push(("X-Amz-Security-Token".into(), c.session_token.into()));
    }
    let mut signed: Vec<(String, String)> = vec![("host".into(), host.to_string())];
    for (k, v) in headers.iter() {
        let lk = k.to_ascii_lowercase();
        if lk.starts_with("x-amz-") || lk == "content-type" {
            signed.push((lk, v.split_whitespace().collect::<Vec<_>>().join(" ")));
        }
    }
    signed.sort();
    let canonical_headers: String = signed.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    let signed_headers = signed.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(";");
    let canonical_request = format!("{method}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}");

    let date = &amz_date[..8];
    let scope = format!("{date}/{}/{}/aws4_request", c.region, c.service);
    let string_to_sign = format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}", hex(&sha256(canonical_request.as_bytes())));
    let signature = hex(&hmac_sha256(&aws_signing_key(c.secret_key, date, c.region, c.service), string_to_sign.as_bytes()));
    headers.push((
        "Authorization".into(),
        format!("AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}", c.access_key),
    ));
}

/// `20150830T123600Z` for a Unix time.
pub fn amz_date(secs: u64) -> String {
    let iso = crate::http::iso_time(secs, 0);
    format!("{}{}{}T{}{}{}Z", &iso[0..4], &iso[5..7], &iso[8..10], &iso[11..13], &iso[14..16], &iso[17..19])
}

// ---------------------------------------------------------------------------------------------
// NTLM v2
// ---------------------------------------------------------------------------------------------

const NTLM_FLAGS: u32 = 0x0000_0001 // unicode
    | 0x0000_0004 // request target
    | 0x0000_0200 // NTLM
    | 0x0000_8000 // always sign
    | 0x0008_0000; // extended session security

fn utf16le(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
}

/// The first (negotiate) message, as an `Authorization` header value.
pub fn ntlm_negotiate() -> String {
    let mut m = b"NTLMSSP\0".to_vec();
    m.extend_from_slice(&1u32.to_le_bytes());
    m.extend_from_slice(&NTLM_FLAGS.to_le_bytes());
    m.extend_from_slice(&[0; 16]); // empty domain and workstation
    format!("NTLM {}", base64(&m))
}

/// The server's challenge message, from `WWW-Authenticate: NTLM <base64>` values.
pub fn ntlm_challenge<'a>(values: impl IntoIterator<Item = &'a str>) -> Option<Vec<u8>> {
    let v = values.into_iter().find_map(|v| v.trim().strip_prefix("NTLM ").map(str::trim))?;
    let m = base64_decode(v)?;
    (m.len() >= 32 && m.starts_with(b"NTLMSSP\0") && m[8] == 2).then_some(m)
}

pub fn ntowf_v2(user: &str, pass: &str, domain: &str) -> Vec<u8> {
    let nt = md4(&utf16le(pass));
    hmac_md5(&nt, &utf16le(&format!("{}{domain}", user.to_uppercase())))
}

/// NTProofStr and the blob it signs.
pub fn ntlm_v2_response(v2hash: &[u8], server_challenge: &[u8], client_challenge: &[u8; 8], filetime: u64, target_info: &[u8]) -> Vec<u8> {
    let mut blob = vec![1, 1, 0, 0, 0, 0, 0, 0];
    blob.extend_from_slice(&filetime.to_le_bytes());
    blob.extend_from_slice(client_challenge);
    blob.extend_from_slice(&[0; 4]);
    blob.extend_from_slice(target_info);
    blob.extend_from_slice(&[0; 4]);
    let mut data = server_challenge.to_vec();
    data.extend_from_slice(&blob);
    let mut out = hmac_md5(v2hash, &data);
    out.extend(blob);
    out
}

/// The final (authenticate) message, as an `Authorization` header value. A `DOMAIN\user`
/// user name sets the domain too.
pub fn ntlm_authenticate(challenge: &[u8], user: &str, pass: &str, domain: &str, workstation: &str, client_challenge: [u8; 8], unix_secs: u64) -> Option<String> {
    let (domain, user) = match user.split_once('\\') {
        Some((d, u)) if domain.is_empty() => (d.to_string(), u.to_string()),
        _ => (domain.to_string(), user.to_string()),
    };
    let server_challenge = challenge.get(24..32)?;
    let target_info = if challenge.len() >= 48 {
        let len = u16::from_le_bytes([challenge[40], challenge[41]]) as usize;
        let off = u32::from_le_bytes([challenge[44], challenge[45], challenge[46], challenge[47]]) as usize;
        challenge.get(off..off + len)?.to_vec()
    } else {
        vec![]
    };
    let filetime = (unix_secs + 11_644_473_600) * 10_000_000;
    let v2 = ntowf_v2(&user, pass, &domain);
    let nt = ntlm_v2_response(&v2, server_challenge, &client_challenge, filetime, &target_info);
    let mut lm = hmac_md5(&v2, &[server_challenge, &client_challenge[..]].concat());
    lm.extend_from_slice(&client_challenge);

    let fields = [lm, nt, utf16le(&domain), utf16le(&user), utf16le(workstation), vec![]];
    let mut header = b"NTLMSSP\0".to_vec();
    header.extend_from_slice(&3u32.to_le_bytes());
    let mut payload = Vec::new();
    let mut offset = 64u32;
    let mut secbufs = Vec::new();
    for f in &fields {
        let len = f.len() as u16;
        secbufs.extend_from_slice(&len.to_le_bytes());
        secbufs.extend_from_slice(&len.to_le_bytes());
        secbufs.extend_from_slice(&offset.to_le_bytes());
        offset += f.len() as u32;
        payload.extend_from_slice(f);
    }
    header.extend_from_slice(&secbufs);
    header.extend_from_slice(&NTLM_FLAGS.to_le_bytes());
    header.extend(payload);
    Some(format!("NTLM {}", base64(&header)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_rfc2617() {
        let ch = parse_digest_challenge([
            "Basic realm=\"x\"",
            "Digest realm=\"testrealm@host.com\", qop=\"auth,auth-int\", nonce=\"dcd98b7102dd2f0e8b11d0f600bfb0c093\", opaque=\"5ccc069c403ebaf9f0171e9517f40e41\"",
        ])
        .unwrap();
        assert_eq!(ch.realm, "testrealm@host.com");
        let h = digest_authorization(&ch, "Mufasa", "Circle Of Life", "GET", "/dir/index.html", "0a4f113b");
        assert!(h.contains("response=\"6629fae49393a05397450978507c4ef1\""), "{h}");
        assert!(h.contains("opaque=\"5ccc069c403ebaf9f0171e9517f40e41\""));
    }

    #[test]
    fn aws_vectors() {
        let key = aws_signing_key("wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY", "20120215", "us-east-1", "iam");
        assert_eq!(hex(&key), "f4780e2d9f65fa895f9c67b32ce1baf0b0d8a43505a000a1a9e090d414db404d");
        // "get-vanilla" from the AWS SigV4 test suite.
        let c = AwsCreds {
            access_key: "AKIDEXAMPLE",
            secret_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            region: "us-east-1",
            service: "service",
            session_token: "",
        };
        let mut headers = vec![];
        aws_sign("GET", "https://example.amazonaws.com/", &mut headers, b"", &c, "20150830T123600Z");
        let auth = &headers.iter().find(|(k, _)| k == "Authorization").unwrap().1;
        assert_eq!(
            auth,
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, SignedHeaders=host;x-amz-date, Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
        );
        assert_eq!(amz_date(1_440_938_160), "20150830T123600Z");
    }

    #[test]
    fn ntlm_ms_nlmp_vectors() {
        let v2 = ntowf_v2("User", "Password", "Domain");
        assert_eq!(hex(&v2), "0c868a403bfd7a93a3001ef22ef02e3f");
        let server = [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef];
        let info = [
            0x02, 0x00, 0x0c, 0x00, 0x44, 0x00, 0x6f, 0x00, 0x6d, 0x00, 0x61, 0x00, 0x69, 0x00, 0x6e, 0x00, 0x01, 0x00, 0x0c, 0x00, 0x53,
            0x00, 0x65, 0x00, 0x72, 0x00, 0x76, 0x00, 0x65, 0x00, 0x72, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let resp = ntlm_v2_response(&v2, &server, &[0xaa; 8], 0, &info);
        assert_eq!(hex(&resp[..16]), "68cd0ab851e51c96aabc927bebef6a1c");
        // A full exchange produces a well-formed type 3 message.
        let mut challenge = b"NTLMSSP\0".to_vec();
        challenge.extend_from_slice(&2u32.to_le_bytes());
        challenge.extend_from_slice(&[0; 8]);
        challenge.extend_from_slice(&NTLM_FLAGS.to_le_bytes());
        challenge.extend_from_slice(&server);
        challenge.extend_from_slice(&[0; 8]);
        challenge.extend_from_slice(&(info.len() as u16).to_le_bytes());
        challenge.extend_from_slice(&(info.len() as u16).to_le_bytes());
        challenge.extend_from_slice(&48u32.to_le_bytes());
        challenge.extend_from_slice(&info);
        let header = format!("NTLM {}", base64(&challenge));
        let parsed = ntlm_challenge([header.as_str()]).unwrap();
        let t3 = ntlm_authenticate(&parsed, "Domain\\User", "Password", "", "WS", [0xaa; 8], 0).unwrap();
        let bytes = base64_decode(t3.trim_start_matches("NTLM ")).unwrap();
        assert_eq!(&bytes[..8], b"NTLMSSP\0");
        assert_eq!(bytes[8], 3);
        assert!(ntlm_negotiate().starts_with("NTLM TlRMTVNTUAAB"));
    }

    #[test]
    fn decodes_base64() {
        assert_eq!(base64_decode("dXNlcjpwYXNz").unwrap(), b"user:pass");
        assert_eq!(base64_decode("YQ==").unwrap(), b"a");
        assert_eq!(base64_decode(&base64(&[0, 255, 7, 9])).unwrap(), vec![0, 255, 7, 9]);
    }
}
