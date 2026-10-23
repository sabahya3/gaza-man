//! A local mock server: answers requests with the examples saved in a collection, so a
//! front end (or a class) can work before the real API exists.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use crate::collection::{Collection, Example, Id, KeyValue};
use crate::http;

#[derive(Clone, Debug, PartialEq)]
enum Seg {
    Lit(String),
    Any,
}

#[derive(Clone, Debug)]
pub struct Route {
    method: String,
    segments: Vec<Seg>,
    pub name: String,
    examples: Vec<Example>,
}

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub at: u64,
    pub method: String,
    pub path: String,
    pub status: u16,
    /// The request and example that answered.
    pub matched: Option<String>,
}

/// The path segments of a request URL, with variables as wildcards.
fn route_path(url: &str, vars: &[KeyValue]) -> Vec<Seg> {
    let resolved = http::resolve(url, vars);
    let base = crate::collection::url_base(&resolved);
    let path = match base.split_once("://") {
        Some((_, rest)) => rest.find('/').map(|i| &rest[i..]).unwrap_or("/"),
        // An unresolved `{{baseUrl}}/users`: drop the first segment.
        None if base.starts_with("{{") => base.find('/').map(|i| &base[i..]).unwrap_or("/"),
        None => base.find('/').map(|i| &base[i..]).unwrap_or("/"),
    };
    path.split('/')
        .filter(|s| !s.is_empty())
        .map(|s| if s.starts_with(':') || s.contains("{{") { Seg::Any } else { Seg::Lit(s.to_string()) })
        .collect()
}

pub fn routes(c: &Collection, vars: &[KeyValue]) -> Vec<Route> {
    c.requests()
        .into_iter()
        .map(|r| Route {
            method: r.method.clone(),
            segments: route_path(&r.url, vars),
            name: r.name.clone(),
            examples: r.examples.clone(),
        })
        .collect()
}

/// The best route for a request: same method, same length, the most literal segments.
fn find<'a>(routes: &'a [Route], method: &str, path: &str) -> Option<&'a Route> {
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    routes
        .iter()
        .filter(|r| r.method.eq_ignore_ascii_case(method) && r.segments.len() == parts.len())
        .filter(|r| r.segments.iter().zip(&parts).all(|(s, p)| matches!(s, Seg::Any) || matches!(s, Seg::Lit(l) if l == p)))
        .max_by_key(|r| r.segments.iter().filter(|s| matches!(s, Seg::Lit(_))).count())
}

/// Picks the example: by `x-mock-response-name`, then `x-mock-response-code`, else the first.
fn pick<'a>(route: &'a Route, headers: &[(String, String)]) -> Option<&'a Example> {
    let header = |n: &str| headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.trim());
    if let Some(name) = header("x-mock-response-name") {
        if let Some(e) = route.examples.iter().find(|e| e.name.eq_ignore_ascii_case(name)) {
            return Some(e);
        }
    }
    if let Some(code) = header("x-mock-response-code").and_then(|c| c.parse::<u16>().ok()) {
        if let Some(e) = route.examples.iter().find(|e| e.status == code) {
            return Some(e);
        }
    }
    route.examples.first()
}

struct Answer {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    matched: Option<String>,
}

fn json_error(status: u16, error: &str, message: String) -> Answer {
    let body = serde_json::json!({ "error": error, "message": message }).to_string().into_bytes();
    Answer { status, headers: vec![("Content-Type".into(), "application/json".into())], body, matched: None }
}

fn answer(routes: &[Route], method: &str, path: &str, headers: &[(String, String)]) -> Answer {
    if method == "OPTIONS" {
        return Answer { status: 204, headers: vec![], body: vec![], matched: Some("CORS preflight".into()) };
    }
    let Some(route) = find(routes, method, path) else {
        return json_error(404, "mockRequestNotFoundError", format!("No request in the collection matches {method} {path}"));
    };
    let Some(example) = pick(route, headers) else {
        return json_error(
            404,
            "mockResponseNotFoundError",
            format!("\"{}\" has no saved example. Send it, then use \"Save as example\".", route.name),
        );
    };
    let headers = example
        .headers
        .iter()
        .filter(|h| h.enabled && !h.key.is_empty())
        .filter(|h| !["content-length", "transfer-encoding", "connection", "content-encoding"].contains(&h.key.to_ascii_lowercase().as_str()))
        .map(|h| (h.key.clone(), h.value.clone()))
        .collect();
    // Dynamic variables like {{$randomInt}} work in examples, as in Postman.
    let body = http::resolve(&example.body, &[]).into_bytes();
    Answer { status: example.status, headers, body, matched: Some(format!("{} → {}", route.name, example.name)) }
}

fn handle(mut stream: TcpStream, routes: &RwLock<Vec<Route>>, log: &Mutex<Vec<LogEntry>>) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > 1 << 20 {
            return;
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let mut lines = head.lines();
    let mut first = lines.next().unwrap_or("").split_whitespace();
    let (method, target) = (first.next().unwrap_or("GET").to_string(), first.next().unwrap_or("/").to_string());
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    // Read (and ignore) the body, so the client isn't cut off mid-send.
    let length: usize = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
    let mut have = buf.len() - header_end;
    while have < length.min(50 << 20) {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => have += n,
        }
    }
    let path = target.split(['?', '#']).next().unwrap_or("/").to_string();
    let a = answer(&routes.read().map(|r| r.clone()).unwrap_or_default(), &method, &path, &headers);

    let mut out = format!("HTTP/1.1 {} {}\r\n", a.status, http::reason(a.status));
    for (k, v) in &a.headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    if !a.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-type")) && !a.body.is_empty() {
        out.push_str("Content-Type: text/plain; charset=utf-8\r\n");
    }
    out.push_str(&format!(
        "Content-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: *\r\nAccess-Control-Allow-Methods: *\r\nX-Powered-By: Gaza Man mock\r\nConnection: close\r\n\r\n",
        a.body.len()
    ));
    let _ = stream.write_all(out.as_bytes());
    let _ = stream.write_all(&a.body);
    let _ = stream.flush();
    if let Ok(mut l) = log.lock() {
        l.push(LogEntry { at: http::now_secs(), method, path: target, status: a.status, matched: a.matched });
        if l.len() > 200 {
            l.remove(0);
        }
    }
}

pub struct MockServer {
    pub coll: Id,
    pub port: u16,
    pub public: bool,
    pub routes: Arc<RwLock<Vec<Route>>>,
    pub log: Arc<Mutex<Vec<LogEntry>>>,
    /// The workspace revision the routes were built from.
    pub revision: u64,
    stop: Arc<AtomicBool>,
}

impl MockServer {
    /// Starts serving on `port`. `public` listens on every network interface (for other devices).
    pub fn start(coll: Id, port: u16, public: bool, routes: Vec<Route>, revision: u64, ctx: eframe::egui::Context) -> Result<Self, String> {
        let addr = if public { "0.0.0.0" } else { "127.0.0.1" };
        let listener = TcpListener::bind((addr, port)).map_err(|e| format!("Can't use port {port}: {e}"))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let server = Self {
            coll,
            port,
            public,
            routes: Arc::new(RwLock::new(routes)),
            log: Arc::new(Mutex::new(vec![])),
            revision,
            stop: Arc::new(AtomicBool::new(false)),
        };
        let (routes, log, stop) = (server.routes.clone(), server.log.clone(), server.stop.clone());
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let (routes, log, ctx) = (routes.clone(), log.clone(), ctx.clone());
                        std::thread::spawn(move || {
                            handle(stream, &routes, &log);
                            ctx.request_repaint();
                        });
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(40)),
                }
            }
        });
        Ok(server)
    }

    pub fn url(&self) -> String {
        format!("http://localhost:{}", self.port)
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collection::{Node, Request};

    fn collection() -> Collection {
        let mut c = Collection::new("Shop");
        let mut get = Request::new("Get product");
        get.url = "{{baseUrl}}/products/:id".into();
        get.examples = vec![
            Example { name: "Found".into(), body: "{\"id\":1}".into(), ..Default::default() },
            Example { name: "Missing".into(), status: 404, body: "{}".into(), ..Default::default() },
        ];
        let mut list = Request::new("List products");
        list.url = "https://shop.io/products".into();
        let mut special = Request::new("Featured");
        special.url = "{{baseUrl}}/products/featured".into();
        special.examples = vec![Example { name: "Featured".into(), body: "[]".into(), ..Default::default() }];
        c.items = vec![Node::Request(get), Node::Request(list), Node::Request(special)];
        c
    }

    #[test]
    fn matching_and_examples() {
        let routes = routes(&collection(), &[]);
        let a = answer(&routes, "GET", "/products/7", &[]);
        assert_eq!((a.status, a.body.as_slice()), (200, b"{\"id\":1}".as_slice()));
        let a = answer(&routes, "GET", "/products/7", &[("X-Mock-Response-Code".into(), "404".into())]);
        assert_eq!(a.status, 404);
        // Literal segments win over `:id`.
        assert_eq!(answer(&routes, "GET", "/products/featured", &[]).body, b"[]");
        // A matching request with no example, and no matching request at all.
        assert!(String::from_utf8_lossy(&answer(&routes, "GET", "/products", &[]).body).contains("mockResponseNotFoundError"));
        assert_eq!(answer(&routes, "POST", "/products/1", &[]).status, 404);
    }

    #[test]
    fn serves_over_http() {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let server = MockServer::start(1, port, false, routes(&collection(), &[]), 0, eframe::egui::Context::default()).unwrap();
        let req = Request { url: format!("{}/products/3", server.url()), ..Default::default() };
        let p = http::prepare(&req, &crate::collection::Auth::None, &[]);
        let (outcome, _) = http::send(&p, &http::Settings { use_system_proxy: false, ..Default::default() }, &Mutex::new(http::CookieJar::default()));
        assert_eq!(outcome.status(), Some(200));
        assert_eq!(outcome.header("x-powered-by"), Some("Gaza Man mock"));
        assert_eq!(server.log.lock().unwrap().len(), 1);
    }
}
