//! Turns a request into a concrete HTTP request (variables, auth, body) and sends it.

use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::collection::{Auth, BodyType, KeyValue, Request};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub timeout_secs: u64,
    pub follow_redirects: bool,
    pub max_redirects: u32,
    pub verify_ssl: bool,
    pub use_cookies: bool,
    pub send_user_agent: bool,
    pub history_limit: usize,
    /// Use the proxy in `proxy_url` (http://, https://, socks4:// or socks5://).
    pub proxy_enabled: bool,
    pub proxy_url: String,
    /// Comma-separated hosts that skip the proxy, e.g. `localhost, *.internal`.
    pub proxy_bypass: String,
    /// Otherwise follow the HTTP_PROXY / HTTPS_PROXY environment variables.
    pub use_system_proxy: bool,
    /// Extra CA certificate (PEM), trusted when a server's certificate isn't publicly signed.
    pub ca_cert: String,
    pub client_certs: Vec<ClientCertEntry>,
}

/// A client certificate (mutual TLS) sent to one host.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClientCertEntry {
    pub enabled: bool,
    /// `api.example.com`, `*.example.com` or `host:port`.
    pub host: String,
    /// PEM certificate (chain) and PEM private key.
    pub cert: String,
    pub key: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            timeout_secs: 30,
            follow_redirects: true,
            max_redirects: 10,
            verify_ssl: true,
            use_cookies: true,
            send_user_agent: true,
            history_limit: 200,
            proxy_enabled: false,
            proxy_url: String::new(),
            proxy_bypass: "localhost, 127.0.0.1".into(),
            use_system_proxy: true,
            ca_cert: String::new(),
            client_certs: vec![],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub name: String,
    pub value: String,
    /// `value` is a path to a file to upload.
    pub file: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PreparedBody {
    Raw(String),
    UrlEncoded(Vec<(String, String)>),
    Multipart(Vec<Part>),
    File(String),
}

/// Auth that's worked out while sending: a challenge to answer, or a signature over the body.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum AuthFlow {
    #[default]
    None,
    Digest {
        username: String,
        password: String,
    },
    Ntlm {
        username: String,
        password: String,
        domain: String,
        workstation: String,
    },
    Aws {
        access_key: String,
        secret_key: String,
        region: String,
        service: String,
        session_token: String,
    },
}

/// A request with variables, auth and default headers applied.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Prepared {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<PreparedBody>,
    pub flow: AuthFlow,
}

#[derive(Clone, Debug)]
pub enum Outcome {
    Response {
        status: u16,
        status_text: String,
        headers: Vec<(String, String)>,
        body: String,
        bytes: Vec<u8>,
        time_ms: u128,
    },
    Error {
        message: String,
        time_ms: u128,
    },
}

impl Outcome {
    pub fn status(&self) -> Option<u16> {
        match self {
            Outcome::Response { status, .. } => Some(*status),
            Outcome::Error { .. } => None,
        }
    }

    pub fn time_ms(&self) -> u128 {
        match self {
            Outcome::Response { time_ms, .. } | Outcome::Error { time_ms, .. } => *time_ms,
        }
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        match self {
            Outcome::Response { headers, .. } => headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str()),
            Outcome::Error { .. } => None,
        }
    }
}

/// The standard reason phrase for a status code, e.g. "Created" for 201.
pub fn reason(code: u16) -> &'static str {
    ureq::http::StatusCode::from_u16(code).ok().and_then(|s| s.canonical_reason()).unwrap_or("")
}

pub fn random_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    );
    h.finish()
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn uuid_v4() -> String {
    let (a, b) = (random_u64(), random_u64());
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&a.to_le_bytes());
    bytes[8..].copy_from_slice(&b.to_le_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

fn pick<'a>(list: &[&'a str]) -> &'a str {
    list[(random_u64() % list.len() as u64) as usize]
}

/// Days since 1970-01-01 to (year, month, day). From Howard Hinnant's `civil_from_days`.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

pub fn iso_time(secs: u64, millis: u32) -> String {
    let (y, m, d) = civil((secs / 86_400) as i64);
    let rem = secs % 86_400;
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

/// Postman's `{{$dynamic}}` variables that Gaza Man understands.
pub const DYNAMIC_VARS: [(&str, &str); 16] = [
    ("$guid", "A v4 UUID"),
    ("$randomUUID", "A v4 UUID"),
    ("$timestamp", "Unix time in seconds"),
    ("$isoTimestamp", "Current time, ISO 8601"),
    ("$randomInt", "A number from 0 to 1000"),
    ("$randomBoolean", "true or false"),
    ("$randomFirstName", "A first name"),
    ("$randomLastName", "A last name"),
    ("$randomFullName", "A full name"),
    ("$randomUserName", "A user name"),
    ("$randomEmail", "An email address"),
    ("$randomPhoneNumber", "A phone number"),
    ("$randomCity", "A city"),
    ("$randomColor", "A color name"),
    ("$randomAlphaNumeric", "One letter or digit"),
    ("$randomPrice", "A price like 42.50"),
];

const FIRST: [&str; 12] = [
    "Ahmed", "Lina", "Omar", "Sara", "Yousef", "Mariam", "Khaled", "Nour", "Adam", "Huda", "Sami",
    "Rana",
];
const LAST: [&str; 10] = [
    "Haddad", "Nasser", "Khalil", "Saleh", "Barakat", "Awad", "Mansour", "Darwish", "Qasem",
    "Hamdan",
];

fn dynamic(name: &str) -> Option<String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Some(match name {
        "$guid" | "$randomUUID" => uuid_v4(),
        "$timestamp" => now.as_secs().to_string(),
        "$isoTimestamp" => iso_time(now.as_secs(), now.subsec_millis()),
        "$randomInt" => (random_u64() % 1001).to_string(),
        "$randomBoolean" => (random_u64() % 2 == 0).to_string(),
        "$randomFirstName" => pick(&FIRST).to_string(),
        "$randomLastName" => pick(&LAST).to_string(),
        "$randomFullName" => format!("{} {}", pick(&FIRST), pick(&LAST)),
        "$randomUserName" => format!(
            "{}.{}{}",
            pick(&FIRST).to_lowercase(),
            pick(&LAST).to_lowercase(),
            random_u64() % 100
        ),
        "$randomEmail" => format!(
            "{}{}@example.com",
            pick(&FIRST).to_lowercase(),
            random_u64() % 1000
        ),
        "$randomPhoneNumber" => format!(
            "+970-59-{:03}-{:04}",
            random_u64() % 1000,
            random_u64() % 10_000
        ),
        "$randomCity" => pick(&[
            "Gaza",
            "Jerusalem",
            "Nablus",
            "Hebron",
            "Jenin",
            "Ramallah",
            "Khan Younis",
            "Rafah",
        ])
        .to_string(),
        "$randomColor" => pick(&[
            "red", "green", "black", "white", "olive", "gold", "teal", "navy",
        ])
        .to_string(),
        "$randomAlphaNumeric" => {
            let chars = b"abcdefghijklmnopqrstuvwxyz0123456789";
            (chars[(random_u64() % chars.len() as u64) as usize] as char).to_string()
        }
        "$randomPrice" => format!("{}.{:02}", random_u64() % 1000, random_u64() % 100),
        _ => return None,
    })
}

fn resolve_once(input: &str, vars: &[KeyValue]) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("{{") {
        let Some(len) = rest[start + 2..].find("}}") else {
            break;
        };
        out.push_str(&rest[..start]);
        let name = rest[start + 2..start + 2 + len].trim();
        let value = dynamic(name).or_else(|| {
            vars.iter()
                .rev()
                .find(|v| v.enabled && v.key == name)
                .map(|v| v.value.clone())
        });
        match value {
            Some(v) => out.push_str(&v),
            None => out.push_str(&rest[start..start + 4 + len]),
        }
        rest = &rest[start + 4 + len..];
    }
    out.push_str(rest);
    out
}

/// Replaces `{{name}}` with variables (later entries win) and Postman's dynamic variables.
/// Values may themselves use variables, a few levels deep.
pub fn resolve(input: &str, vars: &[KeyValue]) -> String {
    let mut s = resolve_once(input, vars);
    for _ in 0..4 {
        if !s.contains("{{") {
            break;
        }
        let next = resolve_once(&s, vars);
        if next == s {
            break;
        }
        s = next;
    }
    s
}

pub fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub fn base64(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn has_header(headers: &[(String, String)], name: &str) -> bool {
    headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name))
}

/// Replaces `:name` path segments with their values.
fn apply_path_vars(url: &str, vars: &[(String, String)]) -> String {
    if vars.is_empty() {
        return url.to_string();
    }
    let split = url.find(['?', '#']).unwrap_or(url.len());
    let (base, tail) = url.split_at(split);
    let (scheme, rest) = match base.split_once("://") {
        Some((s, r)) => (format!("{s}://"), r),
        None => (String::new(), base),
    };
    let path: Vec<String> = rest
        .split('/')
        .enumerate()
        .map(|(i, seg)| match seg.strip_prefix(':') {
            Some(name) if i > 0 => vars
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| seg.to_string()),
            _ => seg.to_string(),
        })
        .collect();
    format!("{scheme}{}{tail}", path.join("/"))
}

/// `auth` is the request's effective auth (already resolved through its folders).
pub fn prepare(req: &Request, auth: &Auth, vars: &[KeyValue]) -> Prepared {
    let r = |s: &str| resolve(s, vars);

    let path_vars: Vec<(String, String)> = req
        .path_vars
        .iter()
        .map(|v| (v.key.clone(), r(&v.value)))
        .collect();
    let mut url = apply_path_vars(&r(&req.url), &path_vars).trim().to_string();
    if !url.is_empty() && !url.contains("://") {
        url = format!("http://{url}");
    }

    let mut headers: Vec<(String, String)> = req
        .headers
        .iter()
        .filter(|h| h.enabled && !h.key.is_empty())
        .map(|h| (r(&h.key), r(&h.value)))
        .collect();

    let add_query = |url: &mut String, k: &str, v: &str| {
        let sep = if url.contains('?') { '&' } else { '?' };
        *url = format!("{url}{sep}{}={}", percent_encode(k), percent_encode(v));
    };
    match auth {
        Auth::Bearer { token } if !token.is_empty() => {
            headers.push(("Authorization".into(), format!("Bearer {}", r(token))))
        }
        Auth::Basic { username, password } if !(username.is_empty() && password.is_empty()) => {
            let creds = format!("{}:{}", r(username), r(password));
            headers.push((
                "Authorization".into(),
                format!("Basic {}", base64(creds.as_bytes())),
            ));
        }
        Auth::ApiKey {
            key,
            value,
            in_query,
        } if !key.is_empty() => {
            if *in_query {
                add_query(&mut url, &r(key), &r(value));
            } else {
                headers.push((r(key), r(value)));
            }
        }
        Auth::OAuth2 {
            access_token,
            header_prefix,
            ..
        } if !access_token.is_empty() => {
            let prefix = r(header_prefix);
            let value = if prefix.is_empty() {
                r(access_token)
            } else {
                format!("{prefix} {}", r(access_token))
            };
            headers.push(("Authorization".into(), value));
        }
        _ => {}
    }
    let flow = match auth {
        Auth::Digest { username, password } => AuthFlow::Digest {
            username: r(username),
            password: r(password),
        },
        Auth::Ntlm {
            username,
            password,
            domain,
            workstation,
        } => AuthFlow::Ntlm {
            username: r(username),
            password: r(password),
            domain: r(domain),
            workstation: r(workstation),
        },
        Auth::Aws {
            access_key,
            secret_key,
            region,
            service,
            session_token,
        } => AuthFlow::Aws {
            access_key: r(access_key),
            secret_key: r(secret_key),
            region: r(region),
            service: r(service),
            session_token: r(session_token),
        },
        _ => AuthFlow::None,
    };

    let set_type = |headers: &mut Vec<(String, String)>, ct: &str| {
        if !has_header(headers, "content-type") {
            headers.push(("Content-Type".into(), ct.into()));
        }
    };
    let body = if matches!(req.method.as_str(), "GET" | "HEAD") {
        None
    } else {
        match req.body_type {
            BodyType::None => None,
            BodyType::Raw => {
                let raw = r(&req.raw);
                if !raw.trim().is_empty() {
                    set_type(&mut headers, req.raw_lang.content_type());
                }
                Some(PreparedBody::Raw(raw))
            }
            BodyType::UrlEncoded => {
                set_type(&mut headers, "application/x-www-form-urlencoded");
                let fields = req
                    .urlencoded
                    .iter()
                    .filter(|f| f.enabled && !f.key.is_empty())
                    .map(|f| (r(&f.key), r(&f.value)));
                Some(PreparedBody::UrlEncoded(fields.collect()))
            }
            BodyType::FormData => {
                let parts = req.form.iter().filter(|f| f.enabled && !f.key.is_empty());
                Some(PreparedBody::Multipart(
                    parts
                        .map(|f| Part {
                            name: r(&f.key),
                            value: r(&f.value),
                            file: f.file,
                        })
                        .collect(),
                ))
            }
            BodyType::Binary if !req.binary_path.is_empty() => {
                set_type(&mut headers, mime_for(&req.binary_path));
                Some(PreparedBody::File(r(&req.binary_path)))
            }
            BodyType::Binary => None,
            BodyType::GraphQl => {
                set_type(&mut headers, "application/json");
                let variables = r(&req.gql_variables);
                let variables: serde_json::Value = if variables.trim().is_empty() {
                    serde_json::json!({})
                } else {
                    serde_json::from_str(&variables).unwrap_or(serde_json::json!({}))
                };
                let payload =
                    serde_json::json!({ "query": r(&req.gql_query), "variables": variables });
                Some(PreparedBody::Raw(payload.to_string()))
            }
        }
    };

    Prepared {
        method: req.method.clone(),
        url,
        headers,
        body,
        flow,
    }
}

pub fn encode_urlencoded(fields: &[(String, String)]) -> String {
    fields
        .iter()
        .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

pub fn mime_for(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "json" => "application/json",
        "txt" | "md" | "csv" => "text/plain",
        "html" | "htm" => "text/html",
        "xml" => "application/xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

pub fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

// ---------------------------------------------------------------------------------------------
// Cookies
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cookie {
    pub domain: String,
    pub path: String,
    pub name: String,
    pub value: String,
    /// Unix seconds, or `None` for a session cookie.
    pub expires: Option<u64>,
    pub secure: bool,
    pub http_only: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CookieJar {
    pub cookies: Vec<Cookie>,
}

/// `(scheme, host, path)` of a URL, without a port.
pub fn split_url(url: &str) -> (String, String, String) {
    let (scheme, rest) = url.split_once("://").unwrap_or(("http", url));
    let rest = &rest[..rest.find(['?', '#']).unwrap_or(rest.len())];
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let host = host.rsplit('@').next().unwrap_or(host);
    let host = match host.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => h,
        _ => host,
    };
    (
        scheme.to_ascii_lowercase(),
        host.to_ascii_lowercase(),
        path.to_string(),
    )
}

pub(crate) fn domain_matches(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{domain}"))
}

impl CookieJar {
    fn expire(&mut self) {
        let now = now_secs();
        self.cookies.retain(|c| c.expires.is_none_or(|e| e > now));
    }

    /// The `Cookie` header value for a request to `url`.
    pub fn header_for(&mut self, url: &str) -> Option<String> {
        self.expire();
        let (scheme, host, path) = split_url(url);
        let pairs: Vec<String> = self
            .cookies
            .iter()
            .filter(|c| {
                domain_matches(&host, &c.domain)
                    && path.starts_with(&c.path)
                    && (!c.secure || scheme == "https")
            })
            .map(|c| format!("{}={}", c.name, c.value))
            .collect();
        (!pairs.is_empty()).then(|| pairs.join("; "))
    }

    /// Stores the `Set-Cookie` headers of a response from `url`.
    pub fn store(&mut self, url: &str, headers: &[(String, String)]) {
        let (_, host, path) = split_url(url);
        for (_, value) in headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
        {
            let Some(c) = parse_set_cookie(value, &host, &path) else {
                continue;
            };
            self.cookies
                .retain(|o| !(o.domain == c.domain && o.path == c.path && o.name == c.name));
            if c.expires.is_none_or(|e| e > now_secs()) {
                self.cookies.push(c);
            }
        }
    }
}

pub fn parse_set_cookie(header: &str, host: &str, request_path: &str) -> Option<Cookie> {
    let mut parts = header.split(';');
    let (name, value) = parts.next()?.trim().split_once('=')?;
    let default_path = match request_path.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(i) => request_path[..i].to_string(),
    };
    let mut c = Cookie {
        domain: host.to_string(),
        path: default_path,
        name: name.trim().to_string(),
        value: value.trim().trim_matches('"').to_string(),
        ..Default::default()
    };
    for attr in parts {
        let (k, v) = attr.trim().split_once('=').unwrap_or((attr.trim(), ""));
        match k.to_ascii_lowercase().as_str() {
            "domain" if !v.is_empty() => c.domain = v.trim_start_matches('.').to_ascii_lowercase(),
            "path" if v.starts_with('/') => c.path = v.to_string(),
            "max-age" => {
                if let Ok(secs) = v.parse::<i64>() {
                    c.expires = Some(if secs <= 0 {
                        0
                    } else {
                        now_secs() + secs as u64
                    });
                }
            }
            "expires" if c.expires.is_none() => c.expires = parse_http_date(v),
            "secure" => c.secure = true,
            "httponly" => c.http_only = true,
            _ => {}
        }
    }
    Some(c)
}

/// Parses dates like `Wed, 21 Oct 2026 07:28:00 GMT`.
fn parse_http_date(s: &str) -> Option<u64> {
    let parts: Vec<&str> = s
        .split([' ', ',', '-', ':'])
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() < 7 {
        return None;
    }
    let day: i64 = parts[1].parse().ok()?;
    let month = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ]
    .iter()
    .position(|m| parts[2].to_ascii_lowercase().starts_with(m))? as i64
        + 1;
    let mut year: i64 = parts[3].parse().ok()?;
    if year < 100 {
        year += if year < 70 { 2000 } else { 1900 };
    }
    let (h, mi, sec): (i64, i64, i64) = (
        parts[4].parse().ok()?,
        parts[5].parse().ok()?,
        parts[6].parse().ok()?,
    );
    // Days from civil, the inverse of `civil` above.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + h * 3600 + mi * 60 + sec).ok()
}

// ---------------------------------------------------------------------------------------------
// Sending
// ---------------------------------------------------------------------------------------------

/// Does `host` match a pattern like `example.com`, `*.example.com` or `host:port`?
pub(crate) fn host_matches(pattern: &str, host: &str, port: &str) -> bool {
    let pattern = pattern.trim().to_ascii_lowercase();
    let (pat_host, pat_port) = match pattern.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => (h.to_string(), Some(p.to_string())),
        _ => (pattern.clone(), None),
    };
    if pat_port.is_some_and(|p| p != port) {
        return false;
    }
    match pat_host.strip_prefix("*.") {
        Some(domain) => host == domain || host.ends_with(&format!(".{domain}")),
        None => !pat_host.is_empty() && host == pat_host,
    }
}

/// The port of a URL, defaulting by scheme.
fn url_port(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("http", url));
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    match authority.rsplit_once(':') {
        Some((_, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => p.to_string(),
        _ => if scheme.eq_ignore_ascii_case("https") { "443" } else { "80" }.to_string(),
    }
}

fn pem_blocks(text: &str, kind: &str) -> Vec<String> {
    let begin = format!("-----BEGIN {kind}-----");
    let end = format!("-----END {kind}-----");
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find(&begin) {
        let Some(j) = rest[i..].find(&end) else { break };
        out.push(rest[i..i + j + end.len()].to_string());
        rest = &rest[i + j + end.len()..];
    }
    out
}

fn load_certs(path: &str) -> Result<Vec<ureq::tls::Certificate<'static>>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("Could not read certificate \"{path}\": {e}"))?;
    let certs: Vec<_> = pem_blocks(&text, "CERTIFICATE")
        .iter()
        .filter_map(|b| ureq::tls::Certificate::from_pem(b.as_bytes()).ok())
        .collect();
    if certs.is_empty() {
        return Err(format!("No PEM certificate found in \"{path}\""));
    }
    Ok(certs)
}

/// An agent for one request: timeouts, redirects, TLS (client certificate, extra CA) and proxy.
fn agent(settings: &Settings, url: &str, with_ca: bool) -> Result<ureq::Agent, String> {
    let (_, host, _) = split_url(url);
    let port = url_port(url);
    let mut tls = ureq::tls::TlsConfig::builder().disable_verification(!settings.verify_ssl);
    if let Some(c) = settings
        .client_certs
        .iter()
        .find(|c| c.enabled && host_matches(&c.host, &host, &port))
    {
        let chain = load_certs(&c.cert)?;
        let key_pem = std::fs::read(&c.key).map_err(|e| format!("Could not read key \"{}\": {e}", c.key))?;
        let key = ureq::tls::PrivateKey::from_pem(&key_pem).map_err(|e| format!("Bad private key \"{}\": {e}", c.key))?;
        tls = tls.client_cert(Some(ureq::tls::ClientCert::new_with_certs(&chain, key)));
    }
    if with_ca && !settings.ca_cert.trim().is_empty() {
        tls = tls.root_certs(ureq::tls::RootCerts::new_with_certs(&load_certs(settings.ca_cert.trim())?));
    }
    let bypass = settings.proxy_bypass.split(',').any(|p| host_matches(p, &host, &port));
    let proxy = if settings.proxy_enabled && !settings.proxy_url.trim().is_empty() && !bypass {
        Some(ureq::Proxy::new(settings.proxy_url.trim()).map_err(|e| format!("Bad proxy URL: {e}"))?)
    } else if settings.use_system_proxy && !bypass {
        ureq::Proxy::try_from_env()
    } else {
        None
    };
    let mut config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .allow_non_standard_methods(true)
        .timeout_global(Some(Duration::from_secs(settings.timeout_secs.max(1))))
        .max_redirects(if settings.follow_redirects { settings.max_redirects } else { 0 })
        .max_redirects_will_error(false)
        .proxy(proxy)
        .tls_config(tls.build());
    if settings.send_user_agent {
        config = config.user_agent("GazaMan/1.0");
    }
    Ok(config.build().into())
}

struct RawResponse {
    status: u16,
    reason: String,
    headers: Vec<(String, String)>,
    bytes: Vec<u8>,
}

impl RawResponse {
    fn values(&self, name: &str) -> Vec<&str> {
        self.headers.iter().filter(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str()).collect()
    }
}

/// One HTTP round trip, reading the whole body (so the connection can be reused).
fn exchange(agent: &ureq::Agent, sent: &Prepared, body: Option<&[u8]>) -> Result<RawResponse, String> {
    let mut builder = ureq::http::Request::builder().method(sent.method.as_str()).uri(sent.url.as_str());
    for (k, v) in &sent.headers {
        builder = builder.header(k.as_str(), v.as_str());
    }
    let response = match body {
        Some(bytes) => builder.body(bytes.to_vec()).map_err(|e| e.to_string()).and_then(|r| agent.run(r).map_err(|e| e.to_string())),
        None => builder.body(()).map_err(|e| e.to_string()).and_then(|r| agent.run(r).map_err(|e| e.to_string())),
    }?;
    let status = response.status();
    let headers = response
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned()))
        .collect();
    let bytes = response.into_body().with_config().limit(100 * 1024 * 1024).read_to_vec().unwrap_or_default();
    Ok(RawResponse { status: status.as_u16(), reason: status.canonical_reason().unwrap_or("").to_string(), headers, bytes })
}

fn set_header(headers: &mut Vec<(String, String)>, name: &str, value: String) {
    headers.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
    headers.push((name.to_string(), value));
}

/// Path and query of a URL, as Digest auth signs it.
fn request_target(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let rest = &rest[..rest.find('#').unwrap_or(rest.len())];
    rest.find('/').map(|i| rest[i..].to_string()).unwrap_or_else(|| "/".into())
}

fn multipart(parts: &[Part]) -> Result<(String, Vec<u8>), String> {
    let boundary = format!("----GazaManBoundary{:016x}", random_u64());
    let mut out: Vec<u8> = Vec::new();
    for p in parts {
        out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        if p.file {
            let data = std::fs::read(&p.value)
                .map_err(|e| format!("Could not read file \"{}\": {e}", p.value))?;
            out.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: {}\r\n\r\n",
                    p.name,
                    file_name(&p.value),
                    mime_for(&p.value)
                )
                .as_bytes(),
            );
            out.extend_from_slice(&data);
        } else {
            out.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"{}\"\r\n\r\n{}",
                    p.name, p.value
                )
                .as_bytes(),
            );
        }
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    Ok((format!("multipart/form-data; boundary={boundary}"), out))
}

/// Sends the request on the current thread (call it from a worker thread). Returns the result
/// and the request as actually sent, with the cookie and multipart headers added.
pub fn send(p: &Prepared, settings: &Settings, jar: &Mutex<CookieJar>) -> (Outcome, Prepared) {
    let started = Instant::now();
    let mut sent = p.clone();
    let fail = |message: String, sent: Prepared| {
        (
            Outcome::Error {
                message,
                time_ms: started.elapsed().as_millis(),
            },
            sent,
        )
    };

    if p.url.is_empty() {
        return fail("The URL is empty".into(), sent);
    }
    if settings.use_cookies && !has_header(&sent.headers, "cookie") {
        if let Some(cookie) = jar.lock().ok().and_then(|mut j| j.header_for(&p.url)) {
            sent.headers.push(("Cookie".into(), cookie));
        }
    }

    let body: Option<Vec<u8>> = match &p.body {
        None => None,
        Some(PreparedBody::Raw(s)) => Some(s.clone().into_bytes()),
        Some(PreparedBody::UrlEncoded(f)) => Some(encode_urlencoded(f).into_bytes()),
        Some(PreparedBody::File(path)) => match std::fs::read(path) {
            Ok(b) => Some(b),
            Err(e) => return fail(format!("Could not read file \"{path}\": {e}"), sent),
        },
        Some(PreparedBody::Multipart(parts)) => match multipart(parts) {
            Ok((ct, bytes)) => {
                sent.headers
                    .retain(|(k, _)| !k.eq_ignore_ascii_case("content-type"));
                sent.headers.push(("Content-Type".into(), ct));
                Some(bytes)
            }
            Err(e) => return fail(e, sent),
        },
    };

    let mut agent = match agent(settings, &sent.url, false) {
        Ok(a) => a,
        Err(e) => return fail(e, sent),
    };
    match &p.flow {
        AuthFlow::Aws { access_key, secret_key, region, service, session_token } => {
            let creds = crate::auth::AwsCreds { access_key, secret_key, region, service, session_token };
            let date = crate::auth::amz_date(now_secs());
            crate::auth::aws_sign(&sent.method, &sent.url, &mut sent.headers, body.as_deref().unwrap_or(&[]), &creds, &date);
        }
        AuthFlow::Ntlm { .. } => set_header(&mut sent.headers, "Authorization", crate::auth::ntlm_negotiate()),
        _ => {}
    }

    let mut result = exchange(&agent, &sent, body.as_deref());
    // A server with a private certificate: try again trusting the extra CA.
    if let Err(e) = &result {
        if !settings.ca_cert.trim().is_empty() && e.to_lowercase().contains("certificate") {
            match self::agent(settings, &sent.url, true) {
                Ok(a) => {
                    agent = a;
                    result = exchange(&agent, &sent, body.as_deref());
                }
                Err(e) => return fail(e, sent),
            }
        }
    }
    // Challenge / response schemes answer the server's 401 and try again.
    if let Ok(first) = &result {
        if first.status == 401 {
            let www = first.values("www-authenticate");
            let answer = match &p.flow {
                AuthFlow::Digest { username, password } => crate::auth::parse_digest_challenge(www.iter().copied()).map(|ch| {
                    let cnonce = format!("{:016x}", random_u64());
                    crate::auth::digest_authorization(&ch, username, password, &sent.method, &request_target(&sent.url), &cnonce)
                }),
                AuthFlow::Ntlm { username, password, domain, workstation } => {
                    crate::auth::ntlm_challenge(www.iter().copied()).and_then(|ch| {
                        let client: [u8; 8] = random_u64().to_le_bytes();
                        crate::auth::ntlm_authenticate(&ch, username, password, domain, workstation, client, now_secs())
                    })
                }
                _ => None,
            };
            if let Some(header) = answer {
                set_header(&mut sent.headers, "Authorization", header);
                result = exchange(&agent, &sent, body.as_deref());
            }
        }
    }

    match result {
        Ok(r) => {
            if settings.use_cookies {
                if let Ok(mut j) = jar.lock() {
                    j.store(&sent.url, &r.headers);
                }
            }
            let outcome = Outcome::Response {
                status: r.status,
                status_text: r.reason,
                headers: r.headers,
                body: String::from_utf8_lossy(&r.bytes).into_owned(),
                bytes: r.bytes,
                time_ms: started.elapsed().as_millis(),
            };
            (outcome, sent)
        }
        Err(e) => fail(e, sent),
    }
}

/// Runs an OAuth 2.0 client-credentials or password grant and returns the access token.
pub fn fetch_oauth2_token(
    auth: &Auth,
    vars: &[KeyValue],
    settings: &Settings,
) -> Result<String, String> {
    let Auth::OAuth2 {
        token_url,
        client_id,
        client_secret,
        scope,
        password_grant,
        username,
        password,
        ..
    } = auth
    else {
        return Err("Not an OAuth 2.0 auth".into());
    };
    let r = |s: &str| resolve(s, vars);
    if token_url.trim().is_empty() {
        return Err("Enter the Access Token URL first".into());
    }
    let mut fields = vec![(
        "grant_type".to_string(),
        if *password_grant {
            "password"
        } else {
            "client_credentials"
        }
        .to_string(),
    )];
    if *password_grant {
        fields.push(("username".into(), r(username)));
        fields.push(("password".into(), r(password)));
    }
    if !scope.is_empty() {
        fields.push(("scope".into(), r(scope)));
    }
    fields.push(("client_id".into(), r(client_id)));
    if !client_secret.is_empty() {
        fields.push(("client_secret".into(), r(client_secret)));
    }
    let p = Prepared {
        method: "POST".into(),
        url: r(token_url),
        headers: vec![
            (
                "Content-Type".into(),
                "application/x-www-form-urlencoded".into(),
            ),
            ("Accept".into(), "application/json".into()),
        ],
        body: Some(PreparedBody::UrlEncoded(fields)),
        flow: AuthFlow::None,
    };
    let (outcome, _) = send(&p, settings, &Mutex::new(CookieJar::default()));
    match outcome {
        Outcome::Response { status, body, .. } => {
            let json: serde_json::Value =
                serde_json::from_str(&body).map_err(|_| format!("{status}: {body}"))?;
            json["access_token"]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("{status}: no access_token in the response: {}", json))
        }
        Outcome::Error { message, .. } => Err(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_variables() {
        let vars = vec![
            KeyValue::new("baseUrl", "http://localhost:3000"),
            KeyValue::new("api", "{{baseUrl}}/api"),
        ];
        assert_eq!(
            resolve("{{baseUrl}}/students", &vars),
            "http://localhost:3000/students"
        );
        assert_eq!(
            resolve("{{ baseUrl }}/x/{{missing}}", &vars),
            "http://localhost:3000/x/{{missing}}"
        );
        assert_eq!(resolve("no vars {{", &vars), "no vars {{");
        assert_eq!(
            resolve("{{api}}/v1", &vars),
            "http://localhost:3000/api/v1",
            "nested variables"
        );
    }

    #[test]
    fn encodes_base64() {
        assert_eq!(base64(b"user:pass"), "dXNlcjpwYXNz");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"a"), "YQ==");
    }

    #[test]
    fn path_variables() {
        let mut r = Request {
            url: "{{base}}/users/:id/posts?x=:id".into(),
            ..Default::default()
        };
        r.sync_from_url();
        r.path_vars[0].value = "7".into();
        let p = prepare(&r, &Auth::None, &[KeyValue::new("base", "https://a.io")]);
        assert_eq!(p.url, "https://a.io/users/7/posts?x=:id");
    }

    #[test]
    fn cookie_jar() {
        let mut jar = CookieJar::default();
        jar.store(
            "https://api.example.com/auth/login",
            &[
                ("Set-Cookie".into(), "sid=abc; Path=/; HttpOnly".into()),
                (
                    "set-cookie".into(),
                    "theme=dark; Domain=example.com; Path=/".into(),
                ),
                ("set-cookie".into(), "old=1; Max-Age=0".into()),
            ],
        );
        assert_eq!(jar.cookies.len(), 2);
        assert_eq!(
            jar.header_for("https://api.example.com/users").as_deref(),
            Some("sid=abc; theme=dark")
        );
        assert_eq!(
            jar.header_for("https://www.example.com/").as_deref(),
            Some("theme=dark")
        );
        assert_eq!(jar.header_for("https://other.com/"), None);
    }

    /// Live check against httpbin.org: `cargo test -- --ignored digest`.
    #[test]
    #[ignore]
    fn digest_live() {
        for alg in ["MD5", "SHA-256"] {
            let mut r = Request { url: format!("https://httpbin.org/digest-auth/auth/user/passwd/{alg}"), ..Default::default() };
            r.sync_from_url();
            let auth = Auth::Digest { username: "user".into(), password: "passwd".into() };
            let p = prepare(&r, &auth, &[]);
            let (outcome, _) = send(&p, &Settings::default(), &Mutex::new(CookieJar::default()));
            assert_eq!(outcome.status(), Some(200), "{alg}: {outcome:?}");
        }
    }

    #[test]
    fn host_patterns() {
        assert!(host_matches("*.example.com", "api.example.com", "443"));
        assert!(host_matches("example.com:8443", "example.com", "8443"));
        assert!(!host_matches("example.com:8443", "example.com", "443"));
        assert!(!host_matches("", "example.com", "443"));
        assert_eq!(url_port("https://a.io/x"), "443");
        assert_eq!(url_port("http://a.io:3000"), "3000");
        assert_eq!(request_target("https://a.io/dir/x?y=1#f"), "/dir/x?y=1");
    }

    #[test]
    fn http_dates() {
        assert_eq!(parse_http_date("Thu, 01 Jan 1970 00:00:10 GMT"), Some(10));
        assert_eq!(
            parse_http_date("Wed, 21 Oct 2015 07:28:00 GMT"),
            Some(1_445_412_480)
        );
        assert_eq!(iso_time(1_445_412_480, 5), "2015-10-21T07:28:00.005Z");
    }
}
