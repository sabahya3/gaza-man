//! WebSocket (RFC 6455) and Socket.IO v4 clients.
//!
//! Each connection runs on its own thread with a short read timeout, so the same thread can
//! both read frames and write what the UI queues, over plain TCP or TLS alike.

use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use eframe::egui;
use serde_json::Value;

use crate::http::{base64, random_u64};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    In,
    Out,
    /// Something about the connection itself (opened, closed, Socket.IO handshake).
    Info,
}

#[derive(Clone, Debug)]
pub struct Message {
    pub dir: Dir,
    /// Milliseconds since connecting.
    pub at_ms: u128,
    pub text: String,
    /// The Socket.IO event name.
    pub event: Option<String>,
    pub binary: bool,
}

#[derive(Debug)]
pub enum Event {
    Open { status: String, headers: Vec<(String, String)> },
    Message(Message),
    Closed(String),
    Failed(String),
}

pub enum Command {
    /// A raw text message (WebSocket), or the data of an event (Socket.IO).
    Send { event: Option<String>, text: String },
    Close,
}

pub struct Session {
    pub commands: Sender<Command>,
    pub events: Receiver<Event>,
}

// ---------------------------------------------------------------------------------------------
// URLs
// ---------------------------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
pub struct Target {
    pub tls: bool,
    pub host: String,
    pub port: u16,
    /// Path and query sent in the handshake.
    pub resource: String,
    /// Socket.IO namespace, like `/` or `/chat`.
    pub namespace: String,
    /// Engine.IO protocol: 4 for Socket.IO v3 and v4, 3 for Socket.IO v2.
    pub eio: u8,
}

/// Accepts ws://, wss://, http:// and https:// (or no scheme). For Socket.IO the path is the
/// namespace and the Engine.IO endpoint is added, unless the URL already points at it.
pub fn target(url: &str, socket_io: bool) -> Result<Target, String> {
    let url = url.trim();
    let (scheme, rest) = url.split_once("://").unwrap_or(("ws", url));
    let tls = match scheme.to_ascii_lowercase().as_str() {
        "wss" | "https" => true,
        "ws" | "http" => false,
        other => return Err(format!("Unsupported scheme \"{other}\": use ws://, wss://, http:// or https://")),
    };
    let split = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, mut path_query) = (&rest[..split], rest[split..].to_string());
    if authority.is_empty() {
        return Err("The URL has no host".into());
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !h.ends_with(']') || authority.starts_with('[') => {
            (h.trim_matches(['[', ']']).to_string(), p.parse().map_err(|_| format!("Bad port \"{p}\""))?)
        }
        _ => (authority.trim_matches(['[', ']']).to_string(), if tls { 443 } else { 80 }),
    };
    if !path_query.starts_with('/') {
        path_query.insert(0, '/');
    }
    let mut namespace = "/".to_string();
    let mut eio = 4;
    if socket_io {
        let (path, query) = path_query.split_once('?').unwrap_or((&path_query, ""));
        let (path, query) = (path.to_string(), query.to_string());
        if query.split('&').any(|q| q == "EIO=3") {
            eio = 3;
        }
        let mut params: Vec<String> = query.split('&').filter(|q| !q.is_empty() && !q.starts_with("EIO=") && !q.starts_with("transport=")).map(str::to_string).collect();
        params.insert(0, format!("EIO={eio}&transport=websocket"));
        let endpoint = if path.contains("/socket.io") {
            path.clone()
        } else {
            if path != "/" {
                namespace = path.trim_end_matches('/').to_string();
            }
            "/socket.io/".to_string()
        };
        path_query = format!("{endpoint}?{}", params.join("&"));
    }
    Ok(Target { tls, host, port, resource: path_query, namespace, eio })
}

// ---------------------------------------------------------------------------------------------
// Frames
// ---------------------------------------------------------------------------------------------

const TEXT: u8 = 0x1;
const BINARY: u8 = 0x2;
const CLOSE: u8 = 0x8;
const PING: u8 = 0x9;
const PONG: u8 = 0xA;

/// A client frame: always final, always masked (the protocol requires clients to mask).
pub fn encode_frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![0x80 | opcode];
    let len = payload.len();
    if len < 126 {
        out.push(0x80 | len as u8);
    } else if len <= u16::MAX as usize {
        out.push(0x80 | 126);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        out.push(0x80 | 127);
        out.extend_from_slice(&(len as u64).to_be_bytes());
    }
    let mask = (random_u64() as u32).to_be_bytes();
    out.extend_from_slice(&mask);
    out.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    out
}

pub struct Frame {
    pub fin: bool,
    pub opcode: u8,
    pub payload: Vec<u8>,
}

/// Takes one complete frame off the front of `buf`, if there is one.
pub fn decode_frame(buf: &mut Vec<u8>) -> Option<Frame> {
    if buf.len() < 2 {
        return None;
    }
    let fin = buf[0] & 0x80 != 0;
    let opcode = buf[0] & 0x0F;
    let masked = buf[1] & 0x80 != 0;
    let (len, mut pos) = match buf[1] & 0x7F {
        126 if buf.len() >= 4 => (u16::from_be_bytes([buf[2], buf[3]]) as usize, 4),
        127 if buf.len() >= 10 => (u64::from_be_bytes(buf[2..10].try_into().ok()?) as usize, 10),
        126 | 127 => return None,
        n => (n as usize, 2),
    };
    let mask = if masked {
        let m: [u8; 4] = buf.get(pos..pos + 4)?.try_into().ok()?;
        pos += 4;
        Some(m)
    } else {
        None
    };
    if buf.len() < pos + len {
        return None;
    }
    let mut payload = buf[pos..pos + len].to_vec();
    if let Some(m) = mask {
        payload.iter_mut().enumerate().for_each(|(i, b)| *b ^= m[i % 4]);
    }
    buf.drain(..pos + len);
    Some(Frame { fin, opcode, payload })
}

pub fn accept_key(key: &str) -> String {
    base64(&crate::crypto::sha1(format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes()))
}

// ---------------------------------------------------------------------------------------------
// Socket.IO packets (over Engine.IO v4)
// ---------------------------------------------------------------------------------------------

/// `42["event", data]`, with the namespace when it isn't the main one.
pub fn sio_emit(namespace: &str, event: &str, text: &str) -> String {
    let mut args = vec![Value::String(event.to_string())];
    if !text.trim().is_empty() {
        args.push(serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string())));
    }
    let ns = if namespace == "/" { String::new() } else { format!("{namespace},") };
    format!("42{ns}{}", Value::Array(args))
}

/// Splits a Socket.IO packet (without the Engine.IO `4`) into type, namespace and the rest.
fn sio_parts(packet: &str) -> (char, &str, &str) {
    let kind = packet.chars().next().unwrap_or(' ');
    let mut rest = &packet[kind.len_utf8().min(packet.len())..];
    let mut ns = "/";
    if rest.starts_with('/') {
        let end = rest.find(',').unwrap_or(rest.len());
        ns = &rest[..end];
        rest = rest.get(end + 1..).unwrap_or("");
    }
    // An ack id may sit before the JSON.
    let digits = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    (kind, ns, &rest[digits..])
}

/// What the client sees for an incoming Engine.IO text, and what it must answer.
#[derive(Debug, PartialEq)]
pub enum SioIn {
    /// Engine.IO opened: connect to the namespace.
    Open(String),
    Ping,
    /// Engine.IO 3: the server answering our ping.
    Pong,
    Connected(String),
    ConnectError(String),
    Event { event: String, data: String },
    Ack(String),
    Disconnected,
    Other(String),
}

pub fn sio_parse(text: &str) -> SioIn {
    match text.chars().next() {
        Some('0') => SioIn::Open(text[1..].to_string()),
        Some('2') => SioIn::Ping,
        Some('3') => SioIn::Pong,
        Some('4') => {
            let (kind, ns, payload) = sio_parts(&text[1..]);
            match kind {
                '0' => SioIn::Connected(ns.to_string()),
                '1' => SioIn::Disconnected,
                '2' => match serde_json::from_str::<Value>(payload) {
                    Ok(Value::Array(mut items)) if !items.is_empty() => {
                        let event = match items.remove(0) {
                            Value::String(s) => s,
                            other => other.to_string(),
                        };
                        let data = match items.len() {
                            0 => String::new(),
                            1 => pretty(&items[0]),
                            _ => pretty(&Value::Array(items)),
                        };
                        SioIn::Event { event, data }
                    }
                    _ => SioIn::Other(text.to_string()),
                },
                '3' => SioIn::Ack(payload.to_string()),
                '4' => SioIn::ConnectError(payload.to_string()),
                _ => SioIn::Other(text.to_string()),
            }
        }
        _ => SioIn::Other(text.to_string()),
    }
}

fn pretty(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    }
}

// ---------------------------------------------------------------------------------------------
// Connection
// ---------------------------------------------------------------------------------------------

enum Conn {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Conn::Plain(s) => s.read(buf),
            Conn::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Conn::Plain(s) => s.write(buf),
            Conn::Tls(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Conn::Plain(s) => s.flush(),
            Conn::Tls(s) => s.flush(),
        }
    }
}

impl Conn {
    fn tcp(&self) -> &TcpStream {
        match self {
            Conn::Plain(s) => s,
            Conn::Tls(s) => s.get_ref(),
        }
    }
}

/// Accepts any certificate: only used when "Verify SSL certificates" is off in Settings.
#[derive(Debug)]
struct NoVerify(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: &rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// The proxy to use for this host, from Settings (or the environment), like HTTP requests do.
fn proxy_for(t: &Target, settings: &crate::http::Settings) -> Option<String> {
    let port = t.port.to_string();
    if settings.proxy_bypass.split(',').any(|p| crate::http::host_matches(p, &t.host, &port)) {
        return None;
    }
    if settings.proxy_enabled && !settings.proxy_url.trim().is_empty() {
        return Some(settings.proxy_url.trim().to_string());
    }
    if settings.use_system_proxy {
        let names: &[&str] = if t.tls { &["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"] } else { &["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"] };
        return names.iter().find_map(|n| std::env::var(n).ok()).filter(|v| !v.trim().is_empty());
    }
    None
}

fn tcp_to(host: &str, port: u16) -> Result<TcpStream, String> {
    use std::net::ToSocketAddrs;
    let addr = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("Couldn't find {host}: {e}"))?
        .next()
        .ok_or_else(|| format!("Couldn't find {host}"))?;
    let tcp = TcpStream::connect_timeout(&addr, Duration::from_secs(10)).map_err(|e| format!("Couldn't connect to {host}:{port}: {e}"))?;
    tcp.set_nodelay(true).ok();
    tcp.set_read_timeout(Some(Duration::from_secs(10))).ok();
    Ok(tcp)
}

/// `user:p%40ss` from a proxy URL, with the `%` escapes turned back into bytes.
fn pct_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// Opens a tunnel to the target through an HTTP (`CONNECT`) or SOCKS5 proxy.
fn tunnel(proxy: &str, t: &Target) -> Result<TcpStream, String> {
    let (scheme, rest) = proxy.split_once("://").unwrap_or(("http", proxy));
    let rest = rest.trim_end_matches('/');
    let (auth, hostport) = match rest.rsplit_once('@') {
        Some((a, h)) => (Some(a), h),
        None => (None, rest),
    };
    let default_port = match scheme { "https" => 443, "socks5" | "socks5h" | "socks4" => 1080, _ => 80 };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => (h, p.parse().map_err(|_| format!("Bad proxy port in {proxy}"))?),
        None => (hostport, default_port),
    };
    let mut s = tcp_to(host, port).map_err(|e| format!("Proxy: {e}"))?;
    match scheme {
        "http" | "https" => {
            let mut req = format!("CONNECT {0}:{1} HTTP/1.1\r\nHost: {0}:{1}\r\n", t.host, t.port);
            if let Some(a) = auth {
                req.push_str(&format!("Proxy-Authorization: Basic {}\r\n", base64(&pct_decode(a))));
            }
            req.push_str("\r\n");
            s.write_all(req.as_bytes()).map_err(|e| format!("Proxy: {e}"))?;
            let mut buf = Vec::new();
            let mut b = [0u8; 1];
            while !buf.ends_with(b"\r\n\r\n") && buf.len() < 16 * 1024 {
                match s.read(&mut b) {
                    Ok(1) => buf.push(b[0]),
                    _ => return Err("The proxy closed the connection".into()),
                }
            }
            let head = String::from_utf8_lossy(&buf).to_string();
            let status = head.lines().next().unwrap_or("");
            if status.split_whitespace().nth(1) != Some("200") {
                return Err(format!("The proxy refused the tunnel: {status}"));
            }
        }
        "socks5" | "socks5h" => {
            let creds = auth.and_then(|a| a.split_once(':'));
            let methods: &[u8] = if creds.is_some() { &[5, 2, 0, 2] } else { &[5, 1, 0] };
            s.write_all(methods).map_err(|e| format!("Proxy: {e}"))?;
            let mut r = [0u8; 2];
            s.read_exact(&mut r).map_err(|e| format!("Proxy: {e}"))?;
            if r[1] == 2 {
                let (u, p) = creds.ok_or("The SOCKS proxy wants a username and password")?;
                let mut m = vec![1, u.len() as u8];
                m.extend_from_slice(u.as_bytes());
                m.push(p.len() as u8);
                m.extend_from_slice(p.as_bytes());
                s.write_all(&m).map_err(|e| format!("Proxy: {e}"))?;
                s.read_exact(&mut r).map_err(|e| format!("Proxy: {e}"))?;
                if r[1] != 0 {
                    return Err("The SOCKS proxy rejected the username or password".into());
                }
            } else if r[1] != 0 {
                return Err("The SOCKS proxy doesn't accept any login method we support".into());
            }
            let mut m = vec![5, 1, 0, 3, t.host.len() as u8];
            m.extend_from_slice(t.host.as_bytes());
            m.extend_from_slice(&t.port.to_be_bytes());
            s.write_all(&m).map_err(|e| format!("Proxy: {e}"))?;
            let mut head = [0u8; 4];
            s.read_exact(&mut head).map_err(|e| format!("Proxy: {e}"))?;
            if head[1] != 0 {
                return Err(format!("The SOCKS proxy couldn't reach {}:{} (code {})", t.host, t.port, head[1]));
            }
            let skip = match head[3] {
                1 => 4,
                4 => 16,
                _ => {
                    let mut n = [0u8; 1];
                    s.read_exact(&mut n).map_err(|e| format!("Proxy: {e}"))?;
                    n[0] as usize
                }
            } + 2;
            let mut rest = vec![0u8; skip];
            s.read_exact(&mut rest).map_err(|e| format!("Proxy: {e}"))?;
        }
        "socks4" | "socks4a" => {
            // SOCKS4a: IP 0.0.0.1 means "the host name follows", so the proxy resolves it.
            let mut m = vec![4, 1];
            m.extend_from_slice(&t.port.to_be_bytes());
            m.extend_from_slice(&[0, 0, 0, 1]);
            m.extend_from_slice(auth.unwrap_or("").as_bytes());
            m.push(0);
            m.extend_from_slice(t.host.as_bytes());
            m.push(0);
            s.write_all(&m).map_err(|e| format!("Proxy: {e}"))?;
            let mut reply = [0u8; 8];
            s.read_exact(&mut reply).map_err(|e| format!("Proxy: {e}"))?;
            if reply[1] != 0x5A {
                return Err(format!("The SOCKS4 proxy couldn't reach {}:{} (code {})", t.host, t.port, reply[1]));
            }
        }
        other => return Err(format!("WebSocket can't use a {other}:// proxy; use http://, socks4:// or socks5://")),
    }
    Ok(s)
}

fn open(t: &Target, settings: &crate::http::Settings) -> Result<Conn, String> {
    use rustls::pki_types::pem::PemObject;
    let tcp = match proxy_for(t, settings) {
        Some(p) => tunnel(&p, t)?,
        None => tcp_to(&t.host, t.port)?,
    };
    if !t.tls {
        return Ok(Conn::Plain(tcp));
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?;
    let builder = if settings.verify_ssl {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        // The extra CA from Settings, for servers with a private certificate.
        let ca = settings.ca_cert.trim();
        if !ca.is_empty() {
            for cert in rustls::pki_types::CertificateDer::pem_file_iter(ca).map_err(|e| format!("Could not read CA \"{ca}\": {e}"))? {
                let cert = cert.map_err(|e| format!("Bad CA certificate \"{ca}\": {e}"))?;
                roots.add(cert).map_err(|e| format!("Bad CA certificate \"{ca}\": {e}"))?;
            }
        }
        builder.with_root_certificates(roots)
    } else {
        builder.dangerous().with_custom_certificate_verifier(Arc::new(NoVerify(provider)))
    };
    // A client certificate (mutual TLS) for this host, if Settings has one.
    let port = t.port.to_string();
    let client = settings.client_certs.iter().find(|c| c.enabled && crate::http::host_matches(&c.host, &t.host, &port));
    let config = match client {
        Some(c) => {
            let chain = rustls::pki_types::CertificateDer::pem_file_iter(&c.cert)
                .map_err(|e| format!("Could not read certificate \"{}\": {e}", c.cert))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("Bad certificate \"{}\": {e}", c.cert))?;
            let key = rustls::pki_types::PrivateKeyDer::from_pem_file(&c.key).map_err(|e| format!("Bad private key \"{}\": {e}", c.key))?;
            builder.with_client_auth_cert(chain, key).map_err(|e| format!("Client certificate: {e}"))?
        }
        None => builder.with_no_client_auth(),
    };
    let name = rustls::pki_types::ServerName::try_from(t.host.clone()).map_err(|e| format!("Bad host name: {e}"))?;
    let conn = rustls::ClientConnection::new(Arc::new(config), name).map_err(|e| e.to_string())?;
    Ok(Conn::Tls(Box::new(rustls::StreamOwned::new(conn, tcp))))
}

/// Sends the upgrade request and checks the answer. Returns the status line, the response
/// headers and any bytes that arrived after them (the first frames).
fn handshake(conn: &mut Conn, t: &Target, headers: &[(String, String)]) -> Result<(String, Vec<(String, String)>, Vec<u8>), String> {
    let key = base64(&random_u64().to_le_bytes().iter().chain(random_u64().to_le_bytes().iter()).copied().collect::<Vec<u8>>());
    let default_port = if t.tls { 443 } else { 80 };
    let host = if t.port == default_port { t.host.clone() } else { format!("{}:{}", t.host, t.port) };
    let mut req = format!(
        "GET {} HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n",
        t.resource
    );
    const RESERVED: [&str; 6] = ["host", "upgrade", "connection", "sec-websocket-key", "sec-websocket-version", "content-length"];
    for (k, v) in headers {
        if !RESERVED.iter().any(|r| k.eq_ignore_ascii_case(r)) {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
    }
    req.push_str("\r\n");
    conn.write_all(req.as_bytes()).and_then(|_| conn.flush()).map_err(|e| format!("Couldn't send the handshake: {e}"))?;

    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > 64 * 1024 {
            return Err("The server's handshake answer is too long".into());
        }
        match conn.read(&mut chunk) {
            Ok(0) => return Err("The server closed the connection during the handshake".into()),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) => return Err(format!("No handshake answer: {e}")),
        }
    };
    let head = String::from_utf8_lossy(&buf[..end]).to_string();
    let rest = buf[end + 4..].to_vec();
    let mut lines = head.lines();
    let status = lines.next().unwrap_or("").to_string();
    let headers: Vec<(String, String)> = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect();
    if status.split_whitespace().nth(1) != Some("101") {
        return Err(format!("The server didn't switch to WebSocket: {status}"));
    }
    let accept = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("sec-websocket-accept")).map(|(_, v)| v.as_str());
    if accept != Some(accept_key(&key).as_str()) {
        return Err("The server's Sec-WebSocket-Accept doesn't match: not a real WebSocket server?".into());
    }
    Ok((status, headers, rest))
}

/// Connects in the background, with the proxy and TLS settings HTTP requests use.
/// `headers` are the request's headers (auth included).
pub fn connect(url: &str, headers: Vec<(String, String)>, socket_io: bool, settings: &crate::http::Settings, ctx: egui::Context) -> Session {
    let settings = settings.clone();
    let (cmd_tx, cmd_rx) = channel();
    let (evt_tx, evt_rx) = channel();
    let url = url.to_string();
    std::thread::spawn(move || {
        let send = |e: Event| {
            let ok = evt_tx.send(e).is_ok();
            ctx.request_repaint();
            ok
        };
        let result = (|| {
            let mut t = target(&url, socket_io)?;
            let attempt = |t: &Target| -> Result<(Conn, String, Vec<(String, String)>, Vec<u8>), String> {
                let mut conn = open(t, &settings)?;
                conn.tcp().set_read_timeout(Some(Duration::from_secs(10))).ok();
                let (status, headers, rest) = handshake(&mut conn, t, &headers)?;
                Ok((conn, status, headers, rest))
            };
            let (conn, status, resp_headers, rest) = match attempt(&t) {
                // A Socket.IO v2 server refuses Engine.IO 4: try again with version 3.
                Err(e) if socket_io && t.eio == 4 && e.contains(" 400") => {
                    t.eio = 3;
                    t.resource = t.resource.replacen("EIO=4", "EIO=3", 1);
                    let ok = attempt(&t)?;
                    send(Event::Message(Message { dir: Dir::Info, at_ms: 0, text: "This server speaks Socket.IO v2 (Engine.IO 3)".into(), event: None, binary: false }));
                    ok
                }
                other => other?,
            };
            send(Event::Open { status, headers: resp_headers });
            run(conn, rest, &t, socket_io, cmd_rx, &send)
        })();
        match result {
            Ok(reason) => send(Event::Closed(reason)),
            Err(e) => send(Event::Failed(e)),
        };
    });
    Session { commands: cmd_tx, events: evt_rx }
}

fn run(mut conn: Conn, mut buf: Vec<u8>, t: &Target, socket_io: bool, commands: Receiver<Command>, send: &dyn Fn(Event) -> bool) -> Result<String, String> {
    let started = Instant::now();
    let msg = |dir, text: String, event: Option<String>, binary| Event::Message(Message { dir, at_ms: started.elapsed().as_millis(), text, event, binary });
    conn.tcp().set_read_timeout(Some(Duration::from_millis(40))).ok();
    let write = |conn: &mut Conn, opcode: u8, payload: &[u8]| -> Result<(), String> {
        conn.write_all(&encode_frame(opcode, payload)).and_then(|_| conn.flush()).map_err(|e| format!("Couldn't send: {e}"))
    };
    let mut fragments: Vec<u8> = Vec::new();
    let mut fragment_op = TEXT;
    let mut closing: Option<Instant> = None;
    let mut chunk = vec![0u8; 64 * 1024];
    // Engine.IO 3 (Socket.IO v2): the client pings, every `pingInterval` from the open packet.
    let mut ping_interval = Duration::from_secs(25);
    let mut ping_every: Option<Duration> = None;
    let mut last_ping = Instant::now();
    // Socket.IO v2 joins "/" by itself, and some v2 servers accept EIO=4 too. So for "/" we wait
    // a moment: a "40" arriving on its own means v2; otherwise we ask, as v3 and v4 want.
    let mut join_at: Option<Instant> = None;
    loop {
        if let Some(at) = join_at {
            if Instant::now() >= at {
                write(&mut conn, TEXT, b"40")?;
                join_at = None;
            }
        }
        if let Some(every) = ping_every {
            if closing.is_none() && last_ping.elapsed() >= every {
                write(&mut conn, TEXT, b"2")?;
                last_ping = Instant::now();
            }
        }
        // What the UI asked for.
        loop {
            match commands.try_recv() {
                Ok(Command::Send { event, text }) if closing.is_none() => {
                    let wire = match (&event, socket_io) {
                        (Some(ev), true) => sio_emit(&t.namespace, ev, &text),
                        _ => text.clone(),
                    };
                    write(&mut conn, TEXT, wire.as_bytes())?;
                    if !send(msg(Dir::Out, text, event, false)) {
                        return Ok("Closed".into());
                    }
                }
                Ok(Command::Send { .. }) => {}
                Ok(Command::Close) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if closing.is_none() {
                        if socket_io {
                            let ns = if t.namespace == "/" { String::new() } else { format!("{},", t.namespace) };
                            let _ = write(&mut conn, TEXT, format!("41{ns}").as_bytes());
                        }
                        let _ = write(&mut conn, CLOSE, &1000u16.to_be_bytes());
                        closing = Some(Instant::now());
                    }
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
            }
        }
        if closing.is_some_and(|c| c.elapsed() > Duration::from_secs(2)) {
            return Ok("Disconnected".into());
        }

        // What the server sent.
        match conn.read(&mut chunk) {
            Ok(0) => return Ok("The server closed the connection".into()),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(e) => return Err(format!("Connection lost: {e}")),
        }
        while let Some(frame) = decode_frame(&mut buf) {
            match frame.opcode {
                PING => write(&mut conn, PONG, &frame.payload)?,
                PONG => {}
                CLOSE => {
                    let code = frame.payload.get(..2).map(|c| u16::from_be_bytes([c[0], c[1]]));
                    let reason = String::from_utf8_lossy(frame.payload.get(2..).unwrap_or(&[])).to_string();
                    if closing.is_none() {
                        let _ = write(&mut conn, CLOSE, &frame.payload[..frame.payload.len().min(2)]);
                    }
                    let mut why = match code {
                        Some(c) => format!("Closed with code {c}"),
                        None => "Closed".into(),
                    };
                    if !reason.is_empty() {
                        why.push_str(&format!(": {reason}"));
                    }
                    return Ok(why);
                }
                op => {
                    if op != 0 {
                        fragment_op = op;
                    }
                    fragments.extend_from_slice(&frame.payload);
                    if !frame.fin {
                        continue;
                    }
                    let data = std::mem::take(&mut fragments);
                    if fragment_op == BINARY {
                        let preview: String = data.iter().take(64).map(|b| format!("{b:02x} ")).collect();
                        let more = if data.len() > 64 { "…" } else { "" };
                        send(msg(Dir::In, format!("{} bytes: {preview}{more}", data.len()), None, true));
                        continue;
                    }
                    let text = String::from_utf8_lossy(&data).to_string();
                    if !socket_io {
                        send(msg(Dir::In, text, None, false));
                        continue;
                    }
                    let event = match sio_parse(&text) {
                        SioIn::Open(info) => {
                            let open = serde_json::from_str::<Value>(&info).unwrap_or(Value::Null);
                            ping_interval = Duration::from_millis(open["pingInterval"].as_u64().unwrap_or(25_000).max(100));
                            if t.namespace != "/" {
                                write(&mut conn, TEXT, format!("40{},", t.namespace).as_bytes())?;
                            } else if t.eio == 4 {
                                join_at = Some(Instant::now() + Duration::from_millis(250));
                            }
                            if t.eio == 3 {
                                ping_every = Some(ping_interval);
                                last_ping = Instant::now();
                            }
                            let sid = open["sid"].as_str().unwrap_or_default().to_string();
                            msg(Dir::Info, format!("Engine.IO open (sid {sid}), joining namespace {}", t.namespace), None, false)
                        }
                        SioIn::Ping => {
                            write(&mut conn, TEXT, b"3")?;
                            continue;
                        }
                        SioIn::Pong => continue,
                        SioIn::Connected(ns) => {
                            // Joined before we asked: a Socket.IO v2 server, which wants our pings.
                            // (or "/" joined while we asked for another namespace).
                            let unasked = join_at.take().is_some() || t.namespace != "/";
                            if unasked && ns == "/" && ping_every.is_none() {
                                ping_every = Some(ping_interval);
                                last_ping = Instant::now();
                                if !send(msg(Dir::Info, "This server speaks Socket.IO v2".into(), None, false)) {
                                    return Ok("Closed".into());
                                }
                            }
                            msg(Dir::Info, format!("Connected to namespace {ns}"), None, false)
                        }
                        SioIn::ConnectError(e) => msg(Dir::Info, format!("Namespace refused the connection: {e}"), None, false),
                        SioIn::Event { event, data } => msg(Dir::In, data, Some(event), false),
                        SioIn::Ack(data) => msg(Dir::In, data, Some("ack".into()), false),
                        SioIn::Disconnected => msg(Dir::Info, "The server disconnected this socket".into(), None, false),
                        SioIn::Other(raw) => msg(Dir::In, raw, None, false),
                    };
                    if !send(event) {
                        return Ok("Closed".into());
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn parses_targets() {
        let t = target("wss://echo.example.com/chat?x=1", false).unwrap();
        assert_eq!((t.tls, t.host.as_str(), t.port, t.resource.as_str()), (true, "echo.example.com", 443, "/chat?x=1"));
        let t = target("localhost:3000", false).unwrap();
        assert_eq!((t.tls, t.port, t.resource.as_str()), (false, 3000, "/"));
        let t = target("http://localhost:3000/admin?token=abc", true).unwrap();
        assert_eq!(t.resource, "/socket.io/?EIO=4&transport=websocket&token=abc");
        assert_eq!(t.namespace, "/admin");
        let t = target("https://api.io/socket.io/?EIO=3", true).unwrap();
        // `EIO=3` in the URL asks for Socket.IO v2.
        assert_eq!((t.port, t.resource.as_str(), t.namespace.as_str(), t.eio), (443, "/socket.io/?EIO=3&transport=websocket", "/", 3));
        assert_eq!(target("https://api.io/", true).unwrap().eio, 4);
        assert!(target("ftp://x", false).is_err());
    }

    #[test]
    fn frames_round_trip() {
        // The RFC's example: "dGhlIHNhbXBsZSBub25jZQ==" -> "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=".
        assert_eq!(accept_key("dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
        for len in [0usize, 5, 125, 126, 300, 70_000] {
            let payload: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
            let mut buf = encode_frame(TEXT, &payload);
            buf.extend_from_slice(&[0x81]); // the start of a next frame stays in the buffer
            let f = decode_frame(&mut buf).unwrap();
            assert!(f.fin);
            assert_eq!((f.opcode, f.payload.len()), (TEXT, len));
            assert_eq!(f.payload, payload);
            assert_eq!(buf, vec![0x81]);
        }
        let mut partial = encode_frame(TEXT, b"hello")[..4].to_vec();
        assert!(decode_frame(&mut partial).is_none());
    }

    #[test]
    fn socket_io_packets() {
        assert_eq!(sio_emit("/", "chat", "{\"a\":1}"), r#"42["chat",{"a":1}]"#);
        assert_eq!(sio_emit("/admin", "hi", "hello"), r#"42/admin,["hi","hello"]"#);
        assert_eq!(sio_emit("/", "ping", ""), r#"42["ping"]"#);
        assert_eq!(sio_parse("2"), SioIn::Ping);
        assert!(matches!(sio_parse(r#"0{"sid":"x"}"#), SioIn::Open(_)));
        assert_eq!(sio_parse("40/admin,{\"sid\":\"y\"}"), SioIn::Connected("/admin".into()));
        assert_eq!(sio_parse(r#"42["news","hi"]"#), SioIn::Event { event: "news".into(), data: "hi".into() });
        assert_eq!(sio_parse(r#"42/admin,7["n",{"x":1}]"#), SioIn::Event { event: "n".into(), data: "{\n  \"x\": 1\n}".into() });
        assert_eq!(sio_parse("41"), SioIn::Disconnected);
    }

    /// A tiny echo server, to test the client end to end over a real socket.
    fn echo_server() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = s.read(&mut chunk).unwrap();
                buf.extend_from_slice(&chunk[..n]);
            }
            let head = String::from_utf8_lossy(&buf).to_string();
            let key = head.lines().find_map(|l| l.strip_prefix("Sec-WebSocket-Key: ")).unwrap().trim().to_string();
            assert!(head.contains("Authorization: Bearer abc"), "request headers are sent");
            write!(s, "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n", accept_key(&key)).unwrap();
            // A greeting split into two fragments, then echo everything back unmasked.
            s.write_all(&[0x01, 3, b'h', b'e', b'y', 0x80, 1, b'!']).unwrap();
            let mut buf = Vec::new();
            loop {
                let n = s.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    return;
                }
                buf.extend_from_slice(&chunk[..n]);
                while let Some(f) = decode_frame(&mut buf) {
                    let mut out = vec![0x80 | f.opcode, f.payload.len() as u8];
                    out.extend_from_slice(&f.payload);
                    s.write_all(&out).unwrap();
                    if f.opcode == CLOSE {
                        return;
                    }
                }
            }
        });
        port
    }

    #[test]
    fn talks_to_a_server() {
        let port = echo_server();
        let session = connect(&format!("ws://127.0.0.1:{port}/"), vec![("Authorization".into(), "Bearer abc".into())], false, &crate::http::Settings::default(), egui::Context::default());
        let next = || session.events.recv_timeout(Duration::from_secs(5)).expect("an event");
        assert!(matches!(next(), Event::Open { status, .. } if status.contains("101")));
        match next() {
            Event::Message(m) => assert_eq!((m.dir, m.text.as_str()), (Dir::In, "hey!")),
            e => panic!("expected the greeting, got {e:?}"),
        }
        session.commands.send(Command::Send { event: None, text: "مرحبا".into() }).unwrap();
        match next() {
            Event::Message(m) => assert_eq!(m.dir, Dir::Out),
            e => panic!("{e:?}"),
        }
        match next() {
            Event::Message(m) => assert_eq!((m.dir, m.text.as_str()), (Dir::In, "مرحبا")),
            e => panic!("{e:?}"),
        }
        session.commands.send(Command::Close).unwrap();
        assert!(matches!(next(), Event::Closed(reason) if reason.contains("1000")));
    }

    /// Copies bytes both ways between two sockets until one side closes.
    fn pipe(a: TcpStream, b: TcpStream) {
        let (mut a2, mut b2) = (a.try_clone().unwrap(), b.try_clone().unwrap());
        let (mut a, mut b) = (a, b);
        std::thread::spawn(move || { let _ = std::io::copy(&mut a2, &mut b2); });
        std::thread::spawn(move || { let _ = std::io::copy(&mut b, &mut a); });
    }

    fn http_proxy() -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut c, _) = l.accept().unwrap();
            let mut head = Vec::new();
            let mut b = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                c.read_exact(&mut b).unwrap();
                head.push(b[0]);
            }
            let head = String::from_utf8_lossy(&head).to_string();
            let target = head.split_whitespace().nth(1).unwrap().to_string();
            assert!(head.contains("Proxy-Authorization: Basic dXNlcjpwQHNz"), "proxy login is sent: {head}");
            let upstream = TcpStream::connect(target).unwrap();
            c.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n").unwrap();
            pipe(c, upstream);
        });
        port
    }

    fn socks5_proxy() -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut c, _) = l.accept().unwrap();
            let mut greet = [0u8; 4];
            c.read_exact(&mut greet).unwrap();
            assert_eq!(greet, [5, 2, 0, 2], "offers no-auth and user/pass");
            c.write_all(&[5, 2]).unwrap();
            let mut v = [0u8; 2];
            c.read_exact(&mut v).unwrap();
            let mut user = vec![0u8; v[1] as usize];
            c.read_exact(&mut user).unwrap();
            let mut n = [0u8; 1];
            c.read_exact(&mut n).unwrap();
            let mut pass = vec![0u8; n[0] as usize];
            c.read_exact(&mut pass).unwrap();
            assert_eq!((user.as_slice(), pass.as_slice()), (&b"u"[..], &b"p"[..]));
            c.write_all(&[1, 0]).unwrap();
            let mut req = [0u8; 5];
            c.read_exact(&mut req).unwrap();
            let mut host = vec![0u8; req[4] as usize];
            c.read_exact(&mut host).unwrap();
            let mut p = [0u8; 2];
            c.read_exact(&mut p).unwrap();
            let upstream = TcpStream::connect((String::from_utf8(host).unwrap().as_str(), u16::from_be_bytes(p))).unwrap();
            c.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]).unwrap();
            pipe(c, upstream);
        });
        port
    }

    #[test]
    fn connects_through_proxies() {
        for proxy in ["http", "socks5"] {
            let echo = echo_server();
            let proxy_url = match proxy {
                "http" => format!("http://user:p%40ss@127.0.0.1:{}", http_proxy()),
                _ => format!("socks5://u:p@127.0.0.1:{}", socks5_proxy()),
            };
            let settings = crate::http::Settings {
                proxy_enabled: true,
                proxy_url,
                proxy_bypass: String::new(),
                ..Default::default()
            };
            let session = connect(&format!("ws://127.0.0.1:{echo}/"), vec![("Authorization".into(), "Bearer abc".into())], false, &settings, egui::Context::default());
            let next = || session.events.recv_timeout(Duration::from_secs(5)).expect("an event");
            assert!(matches!(next(), Event::Open { .. }), "{proxy}: the handshake goes through the proxy");
            assert!(matches!(next(), Event::Message(m) if m.text == "hey!"), "{proxy}: frames come back through it");
        }
    }

    fn socks4_proxy() -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut c, _) = l.accept().unwrap();
            let mut head = [0u8; 8];
            c.read_exact(&mut head).unwrap();
            assert_eq!(head[..2], [4, 1]);
            assert_eq!(head[4..], [0, 0, 0, 1], "SOCKS4a: the proxy resolves the name");
            let mut read_cstr = || {
                let mut out = Vec::new();
                let mut b = [0u8; 1];
                loop {
                    c.read_exact(&mut b).unwrap();
                    if b[0] == 0 {
                        return String::from_utf8(out).unwrap();
                    }
                    out.push(b[0]);
                }
            };
            let _user = read_cstr();
            let host = read_cstr();
            let upstream = TcpStream::connect((host.as_str(), u16::from_be_bytes([head[2], head[3]]))).unwrap();
            c.write_all(&[0, 0x5A, 0, 0, 0, 0, 0, 0]).unwrap();
            pipe(c, upstream);
        });
        port
    }

    #[test]
    fn connects_through_socks4() {
        let echo = echo_server();
        let settings = crate::http::Settings {
            proxy_enabled: true,
            proxy_url: format!("socks4://127.0.0.1:{}", socks4_proxy()),
            proxy_bypass: String::new(),
            ..Default::default()
        };
        let session = connect(&format!("ws://127.0.0.1:{echo}/"), vec![("Authorization".into(), "Bearer abc".into())], false, &settings, egui::Context::default());
        let next = || session.events.recv_timeout(Duration::from_secs(5)).expect("an event");
        assert!(matches!(next(), Event::Open { .. }));
        assert!(matches!(next(), Event::Message(m) if m.text == "hey!"));
    }

    /// A Socket.IO v2 server that accepts EIO=4 (as some do): it joins "/" by itself and wants
    /// the client to ping. The client must notice, not join twice, and ping.
    #[test]
    fn socket_io_v2_server() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (seen_tx, seen_rx) = channel::<String>();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = s.read(&mut chunk).unwrap();
                buf.extend_from_slice(&chunk[..n]);
            }
            let head = String::from_utf8_lossy(&buf).to_string();
            let key = head.lines().find_map(|l| l.strip_prefix("Sec-WebSocket-Key: ")).unwrap().trim().to_string();
            write!(s, "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n", accept_key(&key)).unwrap();
            let frame = |t: &str| { let mut f = vec![0x81, t.len() as u8]; f.extend_from_slice(t.as_bytes()); f };
            s.write_all(&frame(r#"0{"sid":"x","upgrades":[],"pingInterval":100,"pingTimeout":500}"#)).unwrap();
            s.write_all(&frame("40")).unwrap();
            s.set_read_timeout(Some(Duration::from_millis(900))).unwrap();
            let mut buf = Vec::new();
            let started = Instant::now();
            while started.elapsed() < Duration::from_millis(900) {
                match s.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                }
                while let Some(f) = decode_frame(&mut buf) {
                    let _ = seen_tx.send(String::from_utf8_lossy(&f.payload).to_string());
                }
            }
        });
        let session = connect(&format!("http://127.0.0.1:{port}"), vec![], true, &crate::http::Settings::default(), egui::Context::default());
        let mut infos = Vec::new();
        let end = Instant::now() + Duration::from_millis(800);
        while Instant::now() < end {
            if let Ok(Event::Message(m)) = session.events.recv_timeout(Duration::from_millis(50)) {
                infos.push(m.text);
            }
        }
        let sent: Vec<String> = seen_rx.try_iter().collect();
        assert!(infos.iter().any(|t| t.contains("Socket.IO v2")), "{infos:?}");
        assert!(!sent.iter().any(|p| p.starts_with("40")), "no second join: {sent:?}");
        assert!(sent.iter().filter(|p| p.as_str() == "2").count() >= 3, "pings every 100 ms: {sent:?}");
    }
}
