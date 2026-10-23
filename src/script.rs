//! Postman scripts (pre-request and post-response), run with QuickJS and the `pm` API from
//! `pm.js`. Rust hands the variables, request and response over as JSON, and reads back the
//! variable changes, test results, console output and request edits.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use crate::collection::{Auth, BodyType, KeyValue, RawLang, Request, Scope};
use crate::http::{self, CookieJar, Outcome, Prepared, Settings};
use crate::tests::TestResult;

const PM_JS: &str = include_str!("pm.js");
const PM_LIBS: &str = include_str!("pm_libs.js");
/// A whole phase (all its scripts and `pm.sendRequest` calls) must finish within this.
const TIME_LIMIT: Duration = Duration::from_secs(10);
const MEMORY_LIMIT: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    PreRequest,
    PostResponse,
}

impl Phase {
    /// The `listen` value Postman stores the script under.
    pub fn listen(self) -> &'static str {
        match self {
            Phase::PreRequest => "prerequest",
            Phase::PostResponse => "test",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Phase::PreRequest => "Pre-request",
            Phase::PostResponse => "Post-response",
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Where scripts live: Postman `event` entries
// ---------------------------------------------------------------------------------------------

/// The code of the `listen` event, lines joined.
pub fn code_of(events: &[Value], listen: &str) -> String {
    events
        .iter()
        .filter(|e| e["listen"] == listen)
        .filter_map(|e| match &e["script"]["exec"] {
            Value::Array(lines) => Some(lines.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n")),
            Value::String(s) => Some(s.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Stores `code` as the `listen` event, removing it when the code is empty.
pub fn set_code(events: &mut Vec<Value>, listen: &str, code: &str) {
    let lines: Vec<Value> = code.split('\n').map(Value::from).collect();
    let empty = code.trim().is_empty();
    match events.iter().position(|e| e["listen"] == listen) {
        Some(i) if empty => {
            events.remove(i);
        }
        Some(i) => {
            events[i]["script"]["exec"] = Value::Array(lines);
            if !events[i]["script"]["type"].is_string() {
                events[i]["script"]["type"] = "text/javascript".into();
            }
            // Only one event per kind.
            let mut j = events.len();
            while j > i + 1 {
                j -= 1;
                if events[j]["listen"] == listen {
                    events.remove(j);
                }
            }
        }
        None if empty => {}
        None => events.push(json!({ "listen": listen, "script": { "type": "text/javascript", "exec": lines } })),
    }
}

#[derive(Clone, Debug)]
pub struct Script {
    /// Where it comes from, like `Collection "Shop"` or `Request "Login"`.
    pub label: String,
    pub code: String,
}

/// The scripts that run around one request, outermost (collection) first.
#[derive(Clone, Debug, Default)]
pub struct Chain {
    pub pre: Vec<Script>,
    pub post: Vec<Script>,
}

impl Chain {
    /// Adds the scripts of one level (collection, folder or request).
    pub fn push(&mut self, label: String, events: &[Value]) {
        for (phase, list) in [(Phase::PreRequest, &mut self.pre), (Phase::PostResponse, &mut self.post)] {
            let code = code_of(events, phase.listen());
            if !code.trim().is_empty() {
                list.push(Script { label: label.clone(), code });
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.pre.is_empty() && self.post.is_empty()
    }
}

/// The collection's scripts, then each folder's on the way down, then the request's own.
pub fn chain_for(c: Option<&crate::collection::Collection>, r: &Request) -> Chain {
    let mut chain = Chain::default();
    if let Some(c) = c.filter(|c| c.find(r.id).is_some()) {
        chain.push(format!("Collection \"{}\"", c.name), &c.events);
        for f in c.path_to(r.id) {
            chain.push(format!("Folder \"{}\"", f.name), &f.events);
        }
    }
    chain.push(format!("Request \"{}\"", r.name), &r.events);
    chain
}

// ---------------------------------------------------------------------------------------------
// Variables
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum VarChange {
    Set(Scope, String, String),
    Unset(Scope, String),
    Clear(Scope),
    /// `pm.vault.set` (with a value) or `pm.vault.unset`.
    Vault(String, Option<String>),
}

/// Every variable scope a script can see, as copies it may change.
#[derive(Clone, Debug, Default)]
pub struct Scopes {
    pub globals: Vec<KeyValue>,
    pub collection: Vec<KeyValue>,
    pub environment: Vec<KeyValue>,
    /// Without an active environment, `pm.environment.set` lands in globals (as captures do).
    pub has_env: bool,
    pub env_name: Option<String>,
    /// The Runner's data file row.
    pub data: Vec<(String, String)>,
    /// Things like `{{iteration}}`, resolved after the scopes above.
    pub extra: Vec<KeyValue>,
    /// `pm.variables.set`: only for this request.
    pub locals: BTreeMap<String, String>,
    /// Secrets: in the workspace only, never exported with a collection.
    pub vault: Vec<KeyValue>,
}

fn as_object(list: &[KeyValue]) -> Value {
    let mut m = Map::new();
    for kv in list.iter().filter(|kv| kv.enabled) {
        m.insert(kv.key.clone(), Value::String(kv.value.clone()));
    }
    Value::Object(m)
}

impl Scopes {
    /// All variables, lowest priority first, as `http::prepare` wants them.
    pub fn vars(&self) -> Vec<KeyValue> {
        let mut v = self.globals.clone();
        v.extend(self.collection.iter().cloned());
        v.extend(self.environment.iter().cloned());
        v.extend(self.data.iter().map(|(k, val)| KeyValue::new(k, val)));
        v.extend(self.extra.iter().cloned());
        v.extend(self.locals.iter().map(|(k, val)| KeyValue::new(k, val)));
        // `{{vault:name}}`, as in Postman.
        v.extend(self.vault.iter().filter(|kv| kv.enabled).map(|kv| KeyValue::new(&format!("vault:{}", kv.key), &kv.value)));
        v
    }

    fn list(&mut self, scope: Scope) -> &mut Vec<KeyValue> {
        match scope {
            Scope::Environment if self.has_env => &mut self.environment,
            Scope::Collection => &mut self.collection,
            _ => &mut self.globals,
        }
    }

    pub fn apply(&mut self, changes: &[VarChange]) {
        for c in changes {
            match c {
                VarChange::Set(s, k, v) => crate::collection::set_var(self.list(*s), k, v),
                VarChange::Unset(s, k) => self.list(*s).retain(|kv| &kv.key != k),
                VarChange::Clear(s) => self.list(*s).clear(),
                VarChange::Vault(k, Some(v)) => crate::collection::set_var(&mut self.vault, k, v),
                VarChange::Vault(k, None) => self.vault.retain(|kv| &kv.key != k),
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Results
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct LogLine {
    pub phase: Phase,
    pub level: String,
    pub text: String,
    pub script: String,
}

#[derive(Clone, Debug)]
pub struct ScriptError {
    pub phase: Phase,
    /// Which script, like `Request "Login"`.
    pub script: String,
    pub message: String,
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub tests: Vec<TestResult>,
    pub logs: Vec<LogLine>,
    pub errors: Vec<ScriptError>,
    pub changes: Vec<VarChange>,
    pub skipped: bool,
    /// `pm.execution.setNextRequest`: `Some(None)` stops the run.
    pub next_request: Option<Option<String>>,
    pub ran: bool,
    /// `pm.visualizer.set`: the rendered HTML and its data.
    pub visualizer: Option<Visualizer>,
}

#[derive(Clone, Debug)]
pub struct Visualizer {
    pub html: String,
    pub data: Value,
}

impl Visualizer {
    /// A full page for the browser, with Postman's `pm.getData` so template scripts work.
    pub fn page(&self) -> String {
        let data = self.data.to_string().replace("</", "<\\/");
        format!(
            "<!doctype html><meta charset=\"utf-8\"><script>var pm = {{ getData: function (cb) {{ cb(null, {data}); }} }};</script>\n{}",
            self.html
        )
    }
}

#[derive(Clone, Debug, Default)]
pub struct Info {
    pub request_name: String,
    pub request_id: String,
    pub iteration: u32,
    pub iteration_count: u32,
}

// ---------------------------------------------------------------------------------------------
// Requests and responses as the sandbox sees them
// ---------------------------------------------------------------------------------------------

fn kv_json(list: &[KeyValue]) -> Value {
    list.iter().map(|kv| json!({ "key": kv.key, "value": kv.value, "disabled": !kv.enabled })).collect()
}

fn kv_from(v: &Value) -> Vec<KeyValue> {
    v.as_array()
        .map(|a| {
            a.iter()
                .map(|it| KeyValue {
                    key: it["key"].as_str().unwrap_or("").to_string(),
                    value: match &it["value"] {
                        Value::String(s) => s.clone(),
                        Value::Null => String::new(),
                        other => other.to_string(),
                    },
                    enabled: !it["disabled"].as_bool().unwrap_or(false),
                    ..Default::default()
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn request_json(r: &Request) -> Value {
    let body = match r.body_type {
        BodyType::Raw => json!({ "mode": "raw", "raw": r.raw }),
        BodyType::UrlEncoded => json!({ "mode": "urlencoded", "urlencoded": kv_json(&r.urlencoded) }),
        BodyType::FormData => json!({ "mode": "formdata", "formdata": kv_json(&r.form) }),
        BodyType::GraphQl => json!({ "mode": "graphql", "graphql": { "query": r.gql_query, "variables": r.gql_variables } }),
        BodyType::Binary => json!({ "mode": "file", "file": { "src": r.binary_path } }),
        BodyType::None => json!({ "mode": "none" }),
    };
    json!({
        "id": r.id.to_string(),
        "name": r.name,
        "method": r.method,
        "url": r.url,
        "headers": kv_json(&r.headers),
        "body": body,
    })
}

/// Takes the edits a pre-request script made to `pm.request`.
fn apply_request(r: &mut Request, v: &Value) {
    if let Some(m) = v["method"].as_str() {
        r.method = m.to_uppercase();
    }
    if let Some(u) = v["url"].as_str() {
        if u != r.url {
            r.url = u.to_string();
            r.sync_from_url();
        }
    }
    if v["headers"].is_array() {
        r.headers = kv_from(&v["headers"]);
    }
    let body = &v["body"];
    match body["mode"].as_str() {
        Some("raw") => {
            let raw = match &body["raw"] {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                other => other.to_string(),
            };
            if r.body_type != BodyType::Raw || r.raw != raw {
                r.body_type = BodyType::Raw;
                r.raw = raw;
            }
        }
        Some("urlencoded") if body["urlencoded"].is_array() => {
            r.body_type = BodyType::UrlEncoded;
            r.urlencoded = kv_from(&body["urlencoded"]);
        }
        Some("graphql") => {
            if let Some(q) = body["graphql"]["query"].as_str() {
                r.gql_query = q.to_string();
            }
            if let Some(vars) = body["graphql"]["variables"].as_str() {
                r.gql_variables = vars.to_string();
            }
        }
        _ => {}
    }
}

fn response_json(o: &Outcome) -> Value {
    match o {
        Outcome::Response { status, status_text, headers, body, bytes, time_ms } => json!({
            "code": status,
            "status": status_text,
            "headers": headers.iter().map(|(k, v)| json!({ "key": k, "value": v })).collect::<Vec<_>>(),
            "body": body,
            "responseTime": *time_ms as u64,
            "responseSize": bytes.len(),
        }),
        Outcome::Error { .. } => Value::Null,
    }
}

// ---------------------------------------------------------------------------------------------
// Native helpers for the sandbox
// ---------------------------------------------------------------------------------------------

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).filter_map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()).collect()
}

/// `__digest(alg, dataHex, keyHex)`: a hash, or an HMAC when there's a key.
fn digest(alg: &str, data_hex: &str, key_hex: &str) -> String {
    use crate::crypto;
    let (data, key) = (unhex(data_hex), unhex(key_hex));
    let out = match (alg, key_hex.is_empty()) {
        ("md5", true) => crypto::md5(&data).to_vec(),
        ("sha1", true) => crypto::sha1(&data).to_vec(),
        ("sha256", true) => crypto::sha256(&data).to_vec(),
        ("md5", false) => crypto::hmac_md5(&key, &data),
        ("sha1", false) => crypto::hmac_sha1(&key, &data),
        ("sha256", false) => crypto::hmac_sha256(&key, &data),
        _ => vec![],
    };
    crypto::hex(&out)
}

/// `pm.cookies.jar()`: reads and changes the app's cookie jar. Answers `{result}` or `{error}`.
fn cookie_op(jar: &Mutex<CookieJar>, op: &str, url: &str, args: &str) -> String {
    let args: Value = serde_json::from_str(args).unwrap_or(Value::Null);
    let Ok(mut jar) = jar.lock() else {
        return json!({ "error": "The cookie jar is busy" }).to_string();
    };
    let (scheme, host, path) = http::split_url(url);
    if host.is_empty() {
        return json!({ "error": format!("Not a URL: {url}") }).to_string();
    }
    let for_url = |c: &http::Cookie| http::domain_matches(&host, &c.domain) && path.starts_with(&c.path) && (!c.secure || scheme == "https");
    let name = args["name"].as_str().unwrap_or("").to_string();
    let result = match op {
        "get" => jar.cookies.iter().find(|c| for_url(c) && c.name == name).map(|c| Value::String(c.value.clone())).unwrap_or(Value::Null),
        "getAll" => jar
            .cookies
            .iter()
            .filter(|c| for_url(c))
            .map(|c| json!({ "name": c.name, "value": c.value, "domain": c.domain, "path": c.path, "secure": c.secure, "httpOnly": c.http_only }))
            .collect(),
        "set" => {
            if name.is_empty() {
                return json!({ "error": "A cookie needs a name" }).to_string();
            }
            let cookie_path = args["path"].as_str().map(str::to_string).unwrap_or_else(|| "/".into());
            jar.cookies.retain(|c| !(c.domain == host && c.path == cookie_path && c.name == name));
            jar.cookies.push(http::Cookie {
                domain: host.clone(),
                path: cookie_path,
                name: name.clone(),
                value: args["value"].as_str().unwrap_or("").to_string(),
                expires: None,
                secure: args["secure"] == true,
                http_only: args["httpOnly"] == true,
            });
            Value::Null
        }
        "unset" => {
            jar.cookies.retain(|c| !(for_url(c) && c.name == name));
            Value::Null
        }
        "clear" => {
            jar.cookies.retain(|c| !for_url(c));
            Value::Null
        }
        _ => return json!({ "error": format!("Unknown cookie jar call {op}") }).to_string(),
    };
    json!({ "result": result }).to_string()
}

/// `pm.sendRequest`: sends right away and answers with the response as JSON.
fn host_send(spec: &str, settings: &Settings, jar: &Mutex<CookieJar>) -> String {
    let Ok(spec) = serde_json::from_str::<Value>(spec) else {
        return json!({ "error": "Bad request options" }).to_string();
    };
    let mut r = Request::new("pm.sendRequest");
    r.url = spec["url"].as_str().unwrap_or("").to_string();
    r.sync_from_url();
    r.method = spec["method"].as_str().unwrap_or("GET").to_string();
    r.headers = kv_from(&spec["headers"]);
    let body = &spec["body"];
    match body["mode"].as_str() {
        Some("raw") => {
            r.body_type = BodyType::Raw;
            r.raw = match &body["raw"] {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            r.raw_lang = if serde_json::from_str::<Value>(&r.raw).is_ok() { RawLang::Json } else { RawLang::Text };
        }
        Some("urlencoded") => {
            r.body_type = BodyType::UrlEncoded;
            r.urlencoded = kv_from(&body["urlencoded"]);
        }
        Some("formdata") => {
            r.body_type = BodyType::FormData;
            r.form = kv_from(&body["formdata"]);
        }
        _ => {}
    }
    let prepared = http::prepare(&r, &Auth::None, &[]);
    let (outcome, _) = http::send(&prepared, settings, jar);
    match outcome {
        Outcome::Error { message, .. } => json!({ "error": message }).to_string(),
        o => response_json(&o).to_string(),
    }
}

// ---------------------------------------------------------------------------------------------
// Running
// ---------------------------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn run_phase(
    phase: Phase,
    scripts: &[Script],
    scopes: &mut Scopes,
    req: &mut Request,
    outcome: Option<&Outcome>,
    info: &Info,
    settings: &Settings,
    jar: &Arc<Mutex<CookieJar>>,
    report: &mut Report,
) {
    if scripts.is_empty() {
        return;
    }
    report.ran = true;
    let input = json!({
        "scripts": scripts.iter().map(|s| json!({ "label": s.label, "code": s.code })).collect::<Vec<_>>(),
        "vars": {
            "globals": as_object(&scopes.globals),
            "collection": as_object(&scopes.collection),
            "environment": as_object(&scopes.environment),
            "data": scopes.data.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect::<Map<_, _>>(),
            "locals": scopes.locals.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect::<Map<_, _>>(),
            "vault": as_object(&scopes.vault),
        },
        "envName": scopes.env_name,
        "request": request_json(req),
        "response": outcome.map(response_json).unwrap_or(Value::Null),
        "info": {
            "requestName": info.request_name,
            "requestId": info.request_id,
            "iteration": info.iteration.saturating_sub(1),
            "iterationCount": info.iteration_count.max(1),
            "eventName": phase.listen(),
        },
    });

    let fail = |report: &mut Report, message: String| {
        report.errors.push(ScriptError { phase, script: format!("{} scripts", phase.label()), message })
    };
    let result = (|| -> Result<Value, String> {
        let rt = rquickjs::Runtime::new().map_err(|e| e.to_string())?;
        rt.set_memory_limit(MEMORY_LIMIT);
        rt.set_max_stack_size(1024 * 1024);
        let deadline = Instant::now() + TIME_LIMIT;
        let timed_out = Arc::new(AtomicBool::new(false));
        let finishing = Arc::new(AtomicBool::new(false));
        {
            let (timed_out, finishing) = (timed_out.clone(), finishing.clone());
            rt.set_interrupt_handler(Some(Box::new(move || {
                if !finishing.load(Ordering::Relaxed) && Instant::now() > deadline {
                    timed_out.store(true, Ordering::Relaxed);
                    return true;
                }
                false
            })));
        }
        let ctx = rquickjs::Context::full(&rt).map_err(|e| e.to_string())?;
        let error_of = |ctx: &rquickjs::Ctx, e: rquickjs::Error| -> String {
            if e.is_exception() {
                let ex = ctx.catch();
                ex.as_exception()
                    .map(|x| x.message().unwrap_or_default())
                    .or_else(|| ex.as_string().and_then(|s| s.to_string().ok()))
                    .unwrap_or_else(|| "script error".into())
            } else {
                e.to_string()
            }
        };

        ctx.with(|ctx| -> Result<(), String> {
            let globals = ctx.globals();
            globals.set("__input_json", input.to_string()).map_err(|e| e.to_string())?;
            let (settings, send_jar) = (settings.clone(), jar.clone());
            let send = rquickjs::Function::new(ctx.clone(), move |spec: String| -> String { host_send(&spec, &settings, &send_jar) })
                .map_err(|e| e.to_string())?;
            globals.set("__sendRequest", send).map_err(|e| e.to_string())?;
            let dig = rquickjs::Function::new(ctx.clone(), |alg: String, data: String, key: String| -> String { digest(&alg, &data, &key) })
                .map_err(|e| e.to_string())?;
            globals.set("__digest", dig).map_err(|e| e.to_string())?;
            let jar = jar.clone();
            let cookies = rquickjs::Function::new(ctx.clone(), move |op: String, url: String, args: String| -> String { cookie_op(&jar, &op, &url, &args) })
                .map_err(|e| e.to_string())?;
            globals.set("__cookies", cookies).map_err(|e| e.to_string())?;
            ctx.eval::<(), _>("globalThis.__input = JSON.parse(__input_json);").map_err(|e| error_of(&ctx, e))?;
            ctx.eval::<(), _>(PM_LIBS).map_err(|e| error_of(&ctx, e))?;
            ctx.eval::<(), _>(PM_JS).map_err(|e| error_of(&ctx, e))
        })?;

        // Promises (async scripts, `await pm.sendRequest`, `setTimeout`) run here.
        loop {
            match rt.execute_pending_job() {
                Ok(true) => {}
                Ok(false) => break,
                Err(_) => {
                    if timed_out.load(Ordering::Relaxed) {
                        break;
                    }
                }
            }
        }
        finishing.store(true, Ordering::Relaxed);
        let text = ctx.with(|ctx| ctx.eval::<String, _>("__finish()").map_err(|e| error_of(&ctx, e)))?;
        let mut out: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        if timed_out.load(Ordering::Relaxed) {
            out["timedOut"] = true.into();
        }
        Ok(out)
    })();

    let out = match result {
        Ok(v) => v,
        Err(e) => {
            fail(report, e);
            return;
        }
    };
    if out["timedOut"] == true {
        fail(report, format!("Stopped after {} seconds: is there an endless loop?", TIME_LIMIT.as_secs()));
    } else if out["done"] != true {
        fail(report, "A script is still waiting on a promise that never finished".into());
    }

    if let Some(v) = out.get("visualizer").filter(|v| v.is_object()) {
        report.visualizer = Some(Visualizer {
            html: v["html"].as_str().unwrap_or("").to_string(),
            data: v["data"].clone(),
        });
    }
    let scope_of = |s: &str| match s {
        "globals" => Scope::Global,
        "collection" => Scope::Collection,
        _ => Scope::Environment,
    };
    let changes: Vec<VarChange> = out["changes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            if c["scope"] == "vault" {
                let key = c["key"].as_str()?.to_string();
                return Some(VarChange::Vault(key, (c["op"] == "set").then(|| c["value"].as_str().unwrap_or("").to_string())));
            }
            let scope = scope_of(c["scope"].as_str()?);
            let key = c["key"].as_str().unwrap_or("").to_string();
            Some(match c["op"].as_str()? {
                "set" => VarChange::Set(scope, key, c["value"].as_str().unwrap_or("").to_string()),
                "unset" => VarChange::Unset(scope, key),
                _ => VarChange::Clear(scope),
            })
        })
        .collect();
    scopes.apply(&changes);
    report.changes.extend(changes);
    if let Some(l) = out["locals"].as_object() {
        scopes.locals = l.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect();
    }
    for t in out["tests"].as_array().into_iter().flatten() {
        let skipped = t["skipped"] == true;
        report.tests.push(TestResult {
            name: t["name"].as_str().unwrap_or("").to_string(),
            passed: t["passed"] == true,
            detail: if skipped { "skipped".into() } else { t["error"].as_str().unwrap_or("").to_string() },
        });
    }
    for l in out["logs"].as_array().into_iter().flatten() {
        report.logs.push(LogLine {
            phase,
            level: l["level"].as_str().unwrap_or("log").to_string(),
            text: l["text"].as_str().unwrap_or("").to_string(),
            script: l["script"].as_str().unwrap_or("").to_string(),
        });
    }
    for e in out["errors"].as_array().into_iter().flatten() {
        report.errors.push(ScriptError {
            phase,
            script: e["script"].as_str().unwrap_or("").to_string(),
            message: e["message"].as_str().unwrap_or("").to_string(),
        });
    }
    if phase == Phase::PreRequest {
        report.skipped |= out["skip"] == true;
        apply_request(req, &out["request"]);
    }
    match &out["nextRequest"] {
        Value::String(s) if s == "__unset__" => {}
        Value::String(s) => report.next_request = Some(Some(s.clone())),
        Value::Null => report.next_request = Some(None),
        _ => {}
    }
}

pub struct Exchange {
    /// The request as sent: pre-request scripts may have changed it.
    pub request: Request,
    pub sent: Prepared,
    pub outcome: Outcome,
    pub report: Report,
    pub scopes: Scopes,
}

/// Pre-request scripts, the request itself, then post-response scripts.
#[allow(clippy::too_many_arguments)]
pub fn exchange(
    mut req: Request,
    auth: &Auth,
    chain: &Chain,
    mut scopes: Scopes,
    info: &Info,
    settings: &Settings,
    jar: &Arc<Mutex<CookieJar>>,
) -> Exchange {
    let mut report = Report::default();
    run_phase(Phase::PreRequest, &chain.pre, &mut scopes, &mut req, None, info, settings, jar, &mut report);
    let prepared = http::prepare(&req, auth, &scopes.vars());
    let (outcome, sent) = if report.skipped {
        let skipped = Outcome::Error { message: "Skipped by pm.execution.skipRequest() in a pre-request script".into(), time_ms: 0 };
        (skipped, prepared)
    } else {
        http::send(&prepared, settings, jar)
    };
    if !report.skipped {
        run_phase(Phase::PostResponse, &chain.post, &mut scopes, &mut req, Some(&outcome), info, settings, jar, &mut report);
    }
    Exchange { request: req, sent, outcome, report, scopes }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(phase: Phase, code: &str, outcome: Option<&Outcome>) -> (Report, Scopes, Request) {
        let mut scopes = Scopes {
            globals: vec![KeyValue::new("g", "1")],
            collection: vec![KeyValue::new("baseUrl", "http://localhost:3000")],
            environment: vec![KeyValue::new("token", "abc")],
            has_env: true,
            env_name: Some("Local".into()),
            ..Default::default()
        };
        let mut req = Request::new("Login");
        req.method = "POST".into();
        req.url = "{{baseUrl}}/login?x=1".into();
        req.headers = vec![KeyValue::new("Accept", "application/json")];
        req.body_type = BodyType::Raw;
        req.raw = "{\"user\":\"mona\"}".into();
        let mut report = Report::default();
        let jar = Arc::new(Mutex::new(CookieJar::default()));
        let chain = vec![Script { label: "Request \"Login\"".into(), code: code.into() }];
        run_phase(phase, &chain, &mut scopes, &mut req, outcome, &Info { request_name: "Login".into(), iteration: 1, ..Default::default() }, &Settings::default(), &jar, &mut report);
        (report, scopes, req)
    }

    fn response(status: u16, body: &str) -> Outcome {
        Outcome::Response {
            status,
            status_text: http::reason(status).into(),
            headers: vec![("Content-Type".into(), "application/json".into()), ("X-Id".into(), "7".into())],
            body: body.into(),
            bytes: body.as_bytes().to_vec(),
            time_ms: 42,
        }
    }

    #[test]
    fn events_round_trip() {
        let mut events = vec![];
        set_code(&mut events, "test", "pm.test('a', () => {});\nconsole.log(1);");
        assert_eq!(code_of(&events, "test"), "pm.test('a', () => {});\nconsole.log(1);");
        assert_eq!(events[0]["script"]["exec"].as_array().unwrap().len(), 2);
        set_code(&mut events, "test", "  ");
        assert!(events.is_empty());
    }

    #[test]
    fn variables_and_request_edits() {
        let (r, scopes, req) = run(
            Phase::PreRequest,
            r#"
            pm.environment.set("ts", 123);
            pm.collectionVariables.set("obj", { a: 1 });
            pm.globals.unset("g");
            pm.variables.set("local", "here");
            console.log("url is", pm.request.url.toString(), pm.variables.get("token"), pm.environment.name);
            pm.request.headers.add({ key: "Authorization", value: "Bearer " + pm.environment.get("token") });
            pm.request.url.query.add({ key: "y", value: "{{local}}" });
            const data = JSON.parse(pm.request.body.raw);   // `data` must not clash with the old global
            data.at = "now";
            pm.request.body.raw = JSON.stringify(data);
            "#,
            None,
        );
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(r.logs[0].text, "url is {{baseUrl}}/login?x=1 abc Local");
        assert!(scopes.environment.iter().any(|v| v.key == "ts" && v.value == "123"));
        assert!(scopes.collection.iter().any(|v| v.key == "obj" && v.value == "{\"a\":1}"));
        assert!(scopes.globals.is_empty());
        assert_eq!(scopes.locals.get("local").map(String::as_str), Some("here"));
        assert_eq!(req.url, "{{baseUrl}}/login?x=1&y={{local}}");
        assert!(req.headers.iter().any(|h| h.key == "Authorization" && h.value == "Bearer abc"));
        assert!(req.raw.contains("\"at\":\"now\""));
        // Locals resolve in the request like any variable.
        let prepared = http::prepare(&req, &Auth::None, &scopes.vars());
        assert_eq!(prepared.url, "http://localhost:3000/login?x=1&y=here");
    }

    #[test]
    fn tests_and_expect() {
        let res = response(201, r#"{"id": 5, "name": "Mona", "tags": ["a", "b"], "user": {"role": "admin"}}"#);
        let (r, scopes, _) = run(
            Phase::PostResponse,
            r#"
            pm.test("Status is 201", () => pm.response.to.have.status(201));
            pm.test("Is success", () => { pm.response.to.be.success; pm.response.to.be.ok; });
            const json = pm.response.json();
            pm.test("Body fields", function () {
                pm.expect(json.name).to.equal("Mona");
                pm.expect(json).to.have.property("id").that.is.a("number").and.above(3);
                pm.expect(json.tags).to.include("b").and.have.lengthOf(2);
                pm.expect(json).to.deep.include({ user: { role: "admin" } });
                pm.expect(json.user).to.eql({ role: "admin" });
                pm.expect(json.missing).to.be.undefined;
                pm.expect(json.name).to.match(/^Mo/);
                pm.expect(pm.response.headers.get("x-id")).to.equal("7");
                pm.expect(pm.response.responseTime).to.be.below(500);
            });
            pm.test("Fails with a message", () => pm.expect(json.id, "the id").to.equal(6));
            pm.test("Not", () => pm.expect(json.name).to.not.equal("Ali"));
            pm.test.skip("Later", () => {});
            tests["legacy works"] = responseCode.code === 201;
            pm.environment.set("newId", json.id);
            postman.setGlobalVariable("old", "api");
            "#,
            Some(&res),
        );
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let by_name = |n: &str| r.tests.iter().find(|t| t.name == n).unwrap_or_else(|| panic!("no test {n}: {:?}", r.tests));
        assert!(by_name("Status is 201").passed);
        assert!(by_name("Is success").passed);
        assert!(by_name("Body fields").passed, "{}", by_name("Body fields").detail);
        assert!(!by_name("Fails with a message").passed);
        assert_eq!(by_name("Fails with a message").detail, "AssertionError: the id: expected 5 to equal 6");
        assert!(by_name("Not").passed);
        assert_eq!(by_name("Later").detail, "skipped");
        assert!(by_name("legacy works").passed);
        assert!(scopes.environment.iter().any(|v| v.key == "newId" && v.value == "5"));
        assert!(scopes.globals.iter().any(|v| v.key == "old" && v.value == "api"));
    }

    #[test]
    fn errors_crypto_and_limits() {
        let (r, _, _) = run(Phase::PreRequest, "console.log(CryptoJS.SHA256('abc').toString()); console.log(CryptoJS.HmacSHA256('what do ya want for nothing?', 'Jefe').toString(CryptoJS.enc.Base64)); console.log(btoa('hi'), atob('aGk='), require('crypto-js').MD5('').toString())", None);
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(r.logs[0].text, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(r.logs[1].text, "W9zBRr9gdU5qBCQmCJV1x1oAPwidJzmDnexYuWTsOEM=");
        assert_eq!(r.logs[2].text, "aGk= hi d41d8cd98f00b204e9800998ecf8427e");

        let (r, _, _) = run(Phase::PreRequest, "nope(;", None);
        assert_eq!(r.errors.len(), 1);
        assert!(r.errors[0].message.contains("SyntaxError"), "{:?}", r.errors);

        let (r, _, _) = run(Phase::PreRequest, "undefinedFunction()", None);
        assert!(r.errors[0].message.contains("ReferenceError"), "{:?}", r.errors);

        let (r, _, _) = run(Phase::PreRequest, "pm.execution.skipRequest(); pm.execution.setNextRequest(null)", None);
        assert!(r.skipped);
        assert_eq!(r.next_request, Some(None));

        let started = Instant::now();
        let (r, _, _) = run(Phase::PreRequest, "while (true) {}", None);
        assert!(started.elapsed() < TIME_LIMIT + Duration::from_secs(3));
        assert!(r.errors.iter().any(|e| e.message.contains("endless loop")), "{:?}", r.errors);
    }

    #[test]
    fn moment_cheerio_xml() {
        let (r, _, _) = run(
            Phase::PreRequest,
            r##"
            const d = moment.utc("2026-03-15T10:05:09Z");
            console.log(d.format("YYYY-MM-DD HH:mm:ss"), d.format("ddd, D MMM YYYY [at] h:mm A"), d.unix());
            console.log(d.clone().add(1, "month").format("YYYY-MM-DD"), d.clone().subtract(2, "days").format("DD/MM"), d.clone().startOf("month").toISOString());
            console.log(moment.utc("2026-03-20").diff(d, "days"), d.isBefore("2027-01-01"), require("moment")("31/12/2025", "DD/MM/YYYY").format("YYYY-MM-DD"));
            const $ = cheerio.load("<html><head><title>Shop</title></head><body><ul id='list'><li class='item a' data-id='1'>Olive oil</li><li class='item'>Za&#39;atar</li></ul><a href='/x'>link</a><p>One<br>Two</p></body></html>");
            console.log($("title").text(), $("li.item").length, $("#list > li").eq(1).text(), $("li[data-id]").attr("data-id"), $("a").attr("href"));
            console.log($("li").map((i, el) => $(el).text()).get().join("|"), $("ul li:first-child").hasClass("a"), $("body p").html());
            const xml = xml2Json('<?xml version="1.0"?><order id="7"><item sku="A1">Tea</item><item sku="B2">Dates</item><total>12.5</total><note><![CDATA[a < b]]></note></order>');
            console.log(xml.order.$.id, xml.order.item.length, xml.order.item[1]._, xml.order.item[0].$.sku, xml.order.total, xml.order.note);
            "##,
            None,
        );
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let texts: Vec<&str> = r.logs.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, vec![
            "2026-03-15 10:05:09 Sun, 15 Mar 2026 at 10:05 AM 1773569109",
            "2026-04-15 13/03 2026-03-01T00:00:00.000Z",
            "4 true 2025-12-31",
            "Shop 2 Za'atar 1 /x",
            "Olive oil|Za'atar true One<br>Two",
            "7 2 Dates A1 12.5 a < b",
        ]);
    }

    #[test]
    fn visualizer_vault_cookies() {
        let res = response(200, r#"{"products": [{"title": "Tea", "price": 3}, {"title": "Dates <b>", "price": 0}]}"#);
        let mut scopes = Scopes { vault: vec![KeyValue::new("apiKey", "s3cret")], ..Default::default() };
        let mut req = Request::new("Shop");
        req.url = "https://shop.example/items".into();
        req.headers = vec![KeyValue::new("X-Key", "{{vault:apiKey}}")];
        let jar = Arc::new(Mutex::new(CookieJar::default()));
        let mut report = Report::default();
        let code = r##"
            const t = `<h1>{{title}}</h1><table>{{#each products}}<tr><td>{{@index}}</td><td>{{title}}</td><td>{{#if price}}{{price}}{{else}}free{{/if}}</td><td>{{../title}}</td></tr>{{/each}}</table>`;
            pm.visualizer.set(t, { title: "Menu", products: pm.response.json().products });
            const key = await pm.vault.get("apiKey");
            console.log("vault", key, await pm.vault.get("missing"));
            await pm.vault.set("token", "t-1");
            const jar = pm.cookies.jar();
            jar.set("https://shop.example/", "session", "abc", (err) => console.log("set", err));
            jar.get("https://shop.example/cart", "session", (err, v) => console.log("get", v));
            jar.getAll("https://other.example/", (err, all) => console.log("other", all.length));
        "##;
        let chain = vec![Script { label: "Request \"Shop\"".into(), code: code.into() }];
        run_phase(Phase::PostResponse, &chain, &mut scopes, &mut req, Some(&res), &Info::default(), &Settings::default(), &jar, &mut report);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        let v = report.visualizer.expect("a visualizer");
        assert_eq!(v.html, "<h1>Menu</h1><table><tr><td>0</td><td>Tea</td><td>3</td><td>Menu</td></tr><tr><td>1</td><td>Dates &lt;b&gt;</td><td>free</td><td>Menu</td></tr></table>");
        assert!(v.page().contains("getData") && v.page().contains("\"Tea\""));
        let texts: Vec<&str> = report.logs.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, vec!["vault s3cret undefined", "set null", "get abc", "other 0"]);
        assert!(scopes.vault.iter().any(|v| v.key == "token" && v.value == "t-1"));
        assert_eq!(jar.lock().unwrap().cookies.len(), 1);
        // `{{vault:…}}` resolves in requests too.
        let prepared = http::prepare(&req, &Auth::None, &scopes.vars());
        assert!(prepared.headers.iter().any(|(k, v)| k == "X-Key" && v == "s3cret"));
    }

    #[test]
    fn lodash() {
        let (r, _, _) = run(
            Phase::PreRequest,
            r#"
            const people = [{ name: "Mona", age: 21, city: "Gaza" }, { name: "Ali", age: 25, city: "Rafah" }, { name: "Sara", age: 21, city: "Gaza" }];
            console.log(_.get({ a: { b: [1, { c: 7 }] } }, "a.b[1].c"), _.get({}, "x.y", "none"));
            console.log(_.map(_.sortBy(people, "age"), "name").join(","));
            console.log(JSON.stringify(_.countBy(people, "city")), _.uniq([1, 2, 2, 3]).length, _.sumBy(people, "age"));
            console.log(_.isEqual({ a: [1] }, { a: [1] }), _.isEmpty({}), JSON.stringify(_.pick(people[0], ["name"])));
            console.log(_.camelCase("hello world-again"), _.range(3).join(""), require("lodash").chunk([1, 2, 3], 2).length);
            "#,
            None,
        );
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let texts: Vec<&str> = r.logs.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, vec!["7 none", "Mona,Sara,Ali", "{\"Gaza\":2,\"Rafah\":1} 3 67", "true true {\"name\":\"Mona\"}", "helloWorldAgain 012 2"]);
    }

    #[test]
    fn async_and_send_request() {
        // A local server, so pm.sendRequest runs a real request.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            for _ in 0..2 {
                let (mut s, _) = listener.accept().unwrap();
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf);
                let body = r#"{"token":"t-123"}"#;
                let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        let code = format!(
            r#"
            pm.sendRequest({{ url: "http://127.0.0.1:{port}/token", method: "POST", body: {{ mode: "raw", raw: "{{}}" }} }}, (err, res) => {{
                pm.environment.set("token", res.json().token);
            }});
            const res = await pm.sendRequest("http://127.0.0.1:{port}/again");
            console.log("awaited", res.code);
            await new Promise(r => setTimeout(r, 10));
            console.log("after timeout");
            "#
        );
        let (r, scopes, _) = run(Phase::PreRequest, &code, None);
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert!(scopes.environment.iter().any(|v| v.key == "token" && v.value == "t-123"));
        assert_eq!(r.logs.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), vec!["awaited 200", "after timeout"]);
    }
}
