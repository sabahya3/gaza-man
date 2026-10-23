//! Importing from other formats: Postman, cURL commands, OpenAPI / Swagger (JSON or YAML),
//! Insomnia and HAR files (browser DevTools recordings).

use serde_json::{Value, json};

use crate::collection::{
    self, Auth, BodyType, Collection, Environment, Example, Folder, KeyValue, Node, RawLang, Request,
    new_id,
};

pub enum Imported {
    Collection(Collection),
    Environment(Environment),
    /// Postman globals.
    Globals(Vec<KeyValue>),
    Request(Request),
    /// Insomnia exports carry both.
    Workspace(Vec<Collection>, Vec<Environment>),
}

impl Imported {
    pub fn describe(&self) -> String {
        match self {
            Imported::Collection(c) => format!(
                "collection \"{}\" ({} requests)",
                c.name,
                c.count_requests()
            ),
            Imported::Environment(e) => format!("environment \"{}\"", e.name),
            Imported::Globals(g) => format!("{} global variables", g.len()),
            Imported::Request(r) => format!("request \"{}\"", r.name),
            Imported::Workspace(c, e) => {
                format!("{} collections and {} environments", c.len(), e.len())
            }
        }
    }
}

/// Works out what `text` is and imports it.
pub fn detect(text: &str) -> Result<Imported, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("Nothing to import".into());
    }
    if trimmed.starts_with("curl") {
        return parse_curl(trimmed).map(Imported::Request);
    }
    let json: Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(_) if looks_like_yaml_openapi(trimmed) => {
            let spec = crate::yaml::parse(trimmed)?;
            return parse_openapi(&spec).map(Imported::Collection);
        }
        Err(e) => return Err(format!("Not JSON, YAML OpenAPI or a cURL command: {e}")),
    };
    if collection::is_postman_collection(&json) {
        return collection::parse(&json).map(Imported::Collection);
    }
    if json.get("openapi").is_some() || json.get("swagger").is_some() {
        return parse_openapi(&json).map(Imported::Collection);
    }
    if json["log"]["entries"].is_array() {
        return parse_har(&json).map(Imported::Collection);
    }
    if json["_type"] == "export" && json.get("resources").is_some() {
        return parse_insomnia(&json);
    }
    if collection::is_postman_environment(&json) {
        let env = collection::parse_environment(&json)?;
        if json["_postman_variable_scope"] == "globals" {
            return Ok(Imported::Globals(env.values));
        }
        return Ok(Imported::Environment(env));
    }
    Err("Unknown format. Gaza Man imports Postman collections and environments, cURL, OpenAPI 3 / Swagger 2 (JSON or YAML), Insomnia exports and HAR files.".into())
}

/// A YAML document whose top level has `openapi:` or `swagger:` (comments allowed before it).
fn looks_like_yaml_openapi(text: &str) -> bool {
    text.lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#') && *l != "---")
        .take(40)
        .any(|l| l.starts_with("openapi:") || l.starts_with("swagger:"))
}

// ---------------------------------------------------------------------------------------------
// HAR (HTTP Archive: what browser DevTools export from the Network tab)
// ---------------------------------------------------------------------------------------------

/// Headers the browser adds itself, or HTTP/2 pseudo-headers; replaying them only causes trouble.
fn har_skip_header(name: &str) -> bool {
    name.starts_with(':')
        || ["host", "content-length", "connection", "accept-encoding", "cookie"]
            .iter()
            .any(|h| name.eq_ignore_ascii_case(h))
}

fn har_pairs(v: &Value) -> Vec<KeyValue> {
    v.as_array()
        .map(|a| {
            a.iter()
                .map(|p| KeyValue::new(p["name"].as_str().unwrap_or(""), p["value"].as_str().unwrap_or("")))
                .collect()
        })
        .unwrap_or_default()
}

fn har_request(entry: &Value) -> Request {
    let req = &entry["request"];
    let mut r = Request::default();
    r.method = req["method"].as_str().unwrap_or("GET").to_uppercase();
    r.url = req["url"].as_str().unwrap_or("").to_string();
    r.sync_from_url();
    r.headers = har_pairs(&req["headers"]).into_iter().filter(|h| !har_skip_header(&h.key)).collect();

    let post = &req["postData"];
    let mime = post["mimeType"].as_str().unwrap_or("").to_ascii_lowercase();
    let text = post["text"].as_str().unwrap_or("");
    if mime.contains("x-www-form-urlencoded") {
        r.body_type = BodyType::UrlEncoded;
        r.urlencoded = match har_pairs(&post["params"]) {
            p if !p.is_empty() => p,
            _ => collection::parse_query(text)
                .into_iter()
                .map(|kv| KeyValue::new(&url_decode(&kv.key), &url_decode(&kv.value)))
                .collect(),
        };
        r.headers.retain(|h| !h.key.eq_ignore_ascii_case("content-type"));
    } else if mime.contains("multipart/form-data") {
        r.body_type = BodyType::FormData;
        r.form = har_pairs(&post["params"]);
        r.headers.retain(|h| !h.key.eq_ignore_ascii_case("content-type"));
    } else if !text.is_empty() {
        r.body_type = BodyType::Raw;
        match serde_json::from_str::<Value>(text) {
            Ok(v) => {
                r.raw_lang = RawLang::Json;
                r.raw = serde_json::to_string_pretty(&v).unwrap_or_else(|_| text.to_string());
            }
            Err(_) => {
                r.raw_lang = if mime.contains("xml") { RawLang::Xml } else if mime.contains("html") { RawLang::Html } else { RawLang::Text };
                r.raw = text.to_string();
            }
        }
    }

    // The recorded response becomes an example, so it shows in docs and the mock server.
    let res = &entry["response"];
    if let Some(status) = res["status"].as_u64().filter(|s| *s > 0) {
        let content = &res["content"];
        let body = match (content["text"].as_str(), content["encoding"].as_str()) {
            (Some(t), Some("base64")) => {
                format!("(binary {} response, {} bytes, not imported)", content["mimeType"].as_str().unwrap_or("?"), t.len() * 3 / 4)
            }
            (Some(t), _) => match serde_json::from_str::<Value>(t) {
                Ok(v) => serde_json::to_string_pretty(&v).unwrap_or_else(|_| t.to_string()),
                Err(_) => t.to_string(),
            },
            (None, _) => String::new(),
        };
        let headers = har_pairs(&res["headers"])
            .into_iter()
            .filter(|h| !h.key.starts_with(':') && !h.key.eq_ignore_ascii_case("content-length") && !h.key.eq_ignore_ascii_case("content-encoding"))
            .collect();
        r.examples.push(Example {
            name: format!("Recorded response ({status})"),
            status: status as u16,
            headers,
            body,
            ..Default::default()
        });
    }

    let (_, _, path) = crate::http::split_url(&r.url);
    r.name = format!("{} {path}", r.method);
    r
}

/// One folder per host, requests in the order they were recorded.
pub fn parse_har(json: &Value) -> Result<Collection, String> {
    let entries = json["log"]["entries"].as_array().ok_or("The HAR file has no entries")?;
    if entries.is_empty() {
        return Err("The HAR file has no requests in it".into());
    }
    let mut folders: Vec<Folder> = Vec::new();
    for entry in entries {
        let r = har_request(entry);
        let (_, host, _) = crate::http::split_url(&r.url);
        let folder = match folders.iter_mut().position(|f| f.name == host) {
            Some(i) => &mut folders[i],
            None => {
                folders.push(Folder { name: host, ..Default::default() });
                folders.last_mut().expect("just pushed")
            }
        };
        folder.items.push(Node::Request(r));
    }
    let creator = json["log"]["creator"]["name"].as_str().unwrap_or("browser");
    let mut c = Collection {
        name: "HAR import".into(),
        description: format!("{} requests recorded by {creator}.", entries.len()),
        ..Default::default()
    };
    // A single host doesn't need a folder.
    c.items = if folders.len() == 1 {
        folders.remove(0).items
    } else {
        folders.into_iter().map(Node::Folder).collect()
    };
    Ok(c)
}

// ---------------------------------------------------------------------------------------------
// cURL
// ---------------------------------------------------------------------------------------------

/// Splits a shell command into words, handling quotes, escapes and line continuations.
fn shell_words(cmd: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut chars = cmd.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('\n') => {}
                Some('\r') => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                }
                Some(n) => {
                    cur.push(n);
                    in_word = true;
                }
                None => {}
            },
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(ch) => cur.push(ch),
                        None => return Err("Unclosed ' quote".into()),
                    }
                }
            }
            '$' if chars.peek() == Some(&'\'') => {
                // Bash ANSI-C quoting: $'...'
                chars.next();
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some('\\') => match chars.next() {
                            Some('n') => cur.push('\n'),
                            Some('t') => cur.push('\t'),
                            Some('r') => cur.push('\r'),
                            Some(o) => cur.push(o),
                            None => return Err("Unclosed $' quote".into()),
                        },
                        Some(ch) => cur.push(ch),
                        None => return Err("Unclosed $' quote".into()),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(n @ ('"' | '\\' | '$' | '`')) => cur.push(n),
                            Some('\n') => {}
                            Some(o) => {
                                cur.push('\\');
                                cur.push(o);
                            }
                            None => return Err("Unclosed \" quote".into()),
                        },
                        Some(ch) => cur.push(ch),
                        None => return Err("Unclosed \" quote".into()),
                    }
                }
            }
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            c => {
                cur.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(cur);
    }
    Ok(words)
}

fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3])
                    .ok()
                    .and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(b) => {
                        out.push(b);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Without a Content-Type, cURL sends `-d` data as a form, unless it's clearly JSON.
fn form_data(data: &[String], ct: Option<&str>) -> bool {
    match ct {
        Some(c) => c.contains("x-www-form-urlencoded"),
        None => {
            serde_json::from_str::<Value>(&data.join("&")).is_err()
                && data.iter().all(|d| d.contains('='))
        }
    }
}

pub fn parse_curl(cmd: &str) -> Result<Request, String> {
    let words = shell_words(cmd)?;
    let mut it = words.into_iter().peekable();
    match it.next() {
        Some(w) if w == "curl" => {}
        _ => return Err("A cURL command starts with \"curl\"".into()),
    }
    let mut method: Option<String> = None;
    let mut url = String::new();
    let mut headers: Vec<KeyValue> = Vec::new();
    let mut data: Vec<String> = Vec::new();
    let mut urlencoded: Vec<KeyValue> = Vec::new();
    let mut form: Vec<KeyValue> = Vec::new();
    let mut auth = Auth::Inherit;
    let mut get = false;
    let mut binary_path = String::new();

    while let Some(w) = it.next() {
        // Support both `--header value` and `--header=value`.
        let (flag, inline) = match w.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (w.clone(), None),
        };
        let value = |it: &mut std::iter::Peekable<std::vec::IntoIter<String>>| {
            inline.clone().or_else(|| it.next()).unwrap_or_default()
        };
        match flag.as_str() {
            "-X" | "--request" => method = Some(value(&mut it).to_uppercase()),
            "-H" | "--header" => {
                let h = value(&mut it);
                if let Some((k, v)) = h.split_once(':') {
                    headers.push(KeyValue::new(k.trim(), v.trim()));
                }
            }
            "-d" | "--data" | "--data-raw" | "--data-ascii" => data.push(value(&mut it)),
            "--data-binary" => {
                let v = value(&mut it);
                match v.strip_prefix('@') {
                    Some(path) => binary_path = path.to_string(),
                    None => data.push(v),
                }
            }
            "--json" => {
                data.push(value(&mut it));
                headers.push(KeyValue::new("Content-Type", "application/json"));
                headers.push(KeyValue::new("Accept", "application/json"));
            }
            "--data-urlencode" => {
                let v = value(&mut it);
                match v.split_once('=') {
                    Some((k, val)) => urlencoded.push(KeyValue::new(k, val)),
                    None => urlencoded.push(KeyValue::new(&v, "")),
                }
            }
            "-F" | "--form" | "--form-string" => {
                let v = value(&mut it);
                let (k, val) = v.split_once('=').unwrap_or((&v, ""));
                let mut kv = KeyValue::new(k, val);
                if let Some(path) = val.strip_prefix('@') {
                    kv.value = path
                        .split(';')
                        .next()
                        .unwrap_or(path)
                        .trim_matches('"')
                        .to_string();
                    kv.file = true;
                }
                form.push(kv);
            }
            "-u" | "--user" => {
                let v = value(&mut it);
                let (u, p) = v.split_once(':').unwrap_or((&v, ""));
                auth = Auth::Basic {
                    username: u.into(),
                    password: p.into(),
                };
            }
            "-b" | "--cookie" => headers.push(KeyValue::new("Cookie", &value(&mut it))),
            "-A" | "--user-agent" => headers.push(KeyValue::new("User-Agent", &value(&mut it))),
            "-e" | "--referer" => headers.push(KeyValue::new("Referer", &value(&mut it))),
            "--url" => url = value(&mut it),
            "-G" | "--get" => get = true,
            "-I" | "--head" => method = Some("HEAD".into()),
            "-o" | "--output" | "-m" | "--max-time" | "--connect-timeout" | "-w"
            | "--write-out" | "--retry" | "-x" | "--proxy" | "--cacert" | "--cert" | "--key"
            | "-c" | "--cookie-jar" => {
                value(&mut it);
            }
            f if f.starts_with('-') => {} // --location, --compressed, -k, -s, -v, ...
            _ if url.is_empty() => url = w,
            _ => {}
        }
    }
    if url.is_empty() {
        return Err("No URL found in the cURL command".into());
    }

    let mut r = Request::new("Imported from cURL");
    let ct = headers
        .iter()
        .find(|h| h.key.eq_ignore_ascii_case("content-type"))
        .map(|h| h.value.to_ascii_lowercase());
    if get && !data.is_empty() {
        let sep = if url.contains('?') { '&' } else { '?' };
        url = format!("{url}{sep}{}", data.join("&"));
        data.clear();
    }
    r.url = url;
    r.sync_from_url();
    if !form.is_empty() {
        r.body_type = BodyType::FormData;
        r.form = form;
    } else if !binary_path.is_empty() {
        r.body_type = BodyType::Binary;
        r.binary_path = binary_path;
    } else if !urlencoded.is_empty() || (!data.is_empty() && form_data(&data, ct.as_deref())) {
        r.body_type = BodyType::UrlEncoded;
        for d in &data {
            for kv in collection::parse_query(d) {
                urlencoded.push(KeyValue::new(&url_decode(&kv.key), &url_decode(&kv.value)));
            }
        }
        r.urlencoded = urlencoded;
        headers.retain(|h| !h.key.eq_ignore_ascii_case("content-type"));
    } else if !data.is_empty() {
        let raw = data.join("&");
        let is_json = serde_json::from_str::<Value>(&raw).is_ok();
        r.body_type = BodyType::Raw;
        r.raw_lang = match ct.as_deref() {
            Some(c) if c.contains("json") => RawLang::Json,
            Some(c) if c.contains("xml") => RawLang::Xml,
            Some(c) if c.contains("html") => RawLang::Html,
            None if is_json => RawLang::Json,
            _ if is_json => RawLang::Json,
            _ => RawLang::Text,
        };
        if r.raw_lang == RawLang::Json {
            if let Ok(v) = serde_json::from_str::<Value>(&raw) {
                r.raw = serde_json::to_string_pretty(&v).unwrap_or(raw);
            } else {
                r.raw = raw;
            }
        } else {
            r.raw = raw;
        }
    }
    let has_body = r.body_type != BodyType::None;
    r.method = method.unwrap_or_else(|| {
        if has_body {
            "POST".into()
        } else {
            "GET".into()
        }
    });
    r.headers = headers;
    r.auth = auth;
    let (_, host, path) = crate::http::split_url(&r.url);
    r.name = format!("{} {}", r.method, if path == "/" { host } else { path });
    Ok(r)
}

// ---------------------------------------------------------------------------------------------
// OpenAPI 3 / Swagger 2
// ---------------------------------------------------------------------------------------------

fn resolve_ref<'a>(root: &'a Value, v: &'a Value) -> &'a Value {
    let mut v = v;
    for _ in 0..10 {
        let Some(r) = v.get("$ref").and_then(Value::as_str) else {
            break;
        };
        let Some(path) = r.strip_prefix("#/") else {
            break;
        };
        let mut target = root;
        for part in path.split('/') {
            target = &target[part.replace("~1", "/").replace("~0", "~")];
        }
        if target.is_null() {
            break;
        }
        v = target;
    }
    v
}

/// An example value built from a JSON schema.
fn sample_from_schema(root: &Value, schema: &Value, depth: usize) -> Value {
    let s = resolve_ref(root, schema);
    if depth > 6 {
        return Value::Null;
    }
    if let Some(e) = s.get("example") {
        return e.clone();
    }
    if let Some(e) = s.get("default") {
        return e.clone();
    }
    if let Some(e) = s
        .get("enum")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
    {
        return e.clone();
    }
    if let Some(all) = s.get("allOf").and_then(Value::as_array) {
        let mut merged = serde_json::Map::new();
        for part in all {
            if let Value::Object(o) = sample_from_schema(root, part, depth + 1) {
                merged.extend(o);
            }
        }
        return Value::Object(merged);
    }
    if let Some(first) = s
        .get("oneOf")
        .or_else(|| s.get("anyOf"))
        .and_then(Value::as_array)
        .and_then(|a| a.first())
    {
        return sample_from_schema(root, first, depth + 1);
    }
    let ty = s["type"]
        .as_str()
        .unwrap_or(if s.get("properties").is_some() {
            "object"
        } else {
            ""
        });
    match ty {
        "object" => {
            let mut o = serde_json::Map::new();
            if let Some(props) = s["properties"].as_object() {
                for (k, v) in props {
                    o.insert(k.clone(), sample_from_schema(root, v, depth + 1));
                }
            }
            Value::Object(o)
        }
        "array" => Value::Array(vec![sample_from_schema(root, &s["items"], depth + 1)]),
        "integer" => json!(0),
        "number" => json!(0.0),
        "boolean" => json!(true),
        "string" => match s["format"].as_str() {
            Some("date-time") => json!("2026-01-01T00:00:00Z"),
            Some("date") => json!("2026-01-01"),
            Some("email") => json!("user@example.com"),
            Some("uuid") => json!("{{$guid}}"),
            _ => json!("string"),
        },
        _ => Value::Null,
    }
}

pub fn parse_openapi(spec: &Value) -> Result<Collection, String> {
    let paths = spec["paths"]
        .as_object()
        .ok_or("The OpenAPI file has no \"paths\"")?;
    let swagger2 = spec.get("swagger").is_some();
    let base = if swagger2 {
        let scheme = spec["schemes"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(Value::as_str)
            .unwrap_or("https");
        format!(
            "{scheme}://{}{}",
            spec["host"].as_str().unwrap_or("localhost"),
            spec["basePath"].as_str().unwrap_or("")
        )
    } else {
        spec["servers"][0]["url"]
            .as_str()
            .unwrap_or("http://localhost")
            .to_string()
    };
    let mut c = Collection::new(spec["info"]["title"].as_str().unwrap_or("OpenAPI"));
    c.description = spec["info"]["description"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    c.variables
        .push(KeyValue::new("baseUrl", base.trim_end_matches('/')));

    // Bearer auth when the spec declares an HTTP bearer scheme.
    let schemes = if swagger2 {
        &spec["securityDefinitions"]
    } else {
        &spec["components"]["securitySchemes"]
    };
    if let Some(o) = schemes.as_object() {
        for s in o.values() {
            if s["scheme"].as_str() == Some("bearer") {
                c.auth = Auth::Bearer {
                    token: "{{token}}".into(),
                };
                c.variables.push(KeyValue::new("token", ""));
                break;
            } else if s["type"] == "apiKey" {
                let key = s["name"].as_str().unwrap_or("X-API-Key");
                c.auth = Auth::ApiKey {
                    key: key.into(),
                    value: "{{apiKey}}".into(),
                    in_query: s["in"] == "query",
                };
                c.variables.push(KeyValue::new("apiKey", ""));
                break;
            }
        }
    }

    let mut folders: Vec<Folder> = Vec::new();
    for (path, item) in paths {
        let item = resolve_ref(spec, item);
        let shared = item["parameters"].as_array().cloned().unwrap_or_default();
        for method in ["get", "post", "put", "patch", "delete", "head", "options"] {
            let Some(op) = item.get(method) else { continue };
            // `/users/{id}` becomes `/users/:id`, Postman style.
            let url_path: String = path
                .split('/')
                .map(
                    |seg| match seg.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
                        Some(name) => format!(":{name}"),
                        None => seg.to_string(),
                    },
                )
                .collect::<Vec<_>>()
                .join("/");
            let name = op["summary"]
                .as_str()
                .or(op["operationId"].as_str())
                .map(str::to_string)
                .unwrap_or_else(|| format!("{} {path}", method.to_uppercase()));
            let mut r = Request::new(&name);
            r.method = method.to_uppercase();
            r.url = format!("{{{{baseUrl}}}}{url_path}");
            r.description = op["description"].as_str().unwrap_or_default().to_string();
            r.sync_path_vars();

            let mut params = shared.clone();
            params.extend(op["parameters"].as_array().cloned().unwrap_or_default());
            for p in &params {
                let p = resolve_ref(spec, p);
                let pname = p["name"].as_str().unwrap_or_default();
                let schema = if swagger2 {
                    p.clone()
                } else {
                    p["schema"].clone()
                };
                let example = p
                    .get("example")
                    .cloned()
                    .unwrap_or_else(|| sample_from_schema(spec, &schema, 0));
                let value = match &example {
                    Value::Null => String::new(),
                    Value::String(s) if s == "string" => String::new(),
                    v => crate::tests::plain(v),
                };
                match p["in"].as_str() {
                    Some("query") => {
                        let mut kv = KeyValue::new(pname, &value);
                        kv.enabled = p["required"].as_bool().unwrap_or(false);
                        r.params.push(kv);
                    }
                    Some("header") => r.headers.push(KeyValue::new(pname, &value)),
                    Some("path") => {
                        if let Some(v) = r.path_vars.iter_mut().find(|v| v.key == pname) {
                            v.value = value;
                        }
                    }
                    Some("body") => {
                        r.body_type = BodyType::Raw;
                        r.raw_lang = RawLang::Json;
                        r.raw = serde_json::to_string_pretty(&sample_from_schema(
                            spec,
                            &p["schema"],
                            0,
                        ))
                        .unwrap_or_default();
                    }
                    Some("formData") => {
                        r.body_type = BodyType::FormData;
                        let mut kv = KeyValue::new(pname, &value);
                        kv.file = p["type"] == "file";
                        r.form.push(kv);
                    }
                    _ => {}
                }
            }
            r.sync_to_url();

            if let Some(content) = op["requestBody"]
                .get("content")
                .or_else(|| resolve_ref(spec, &op["requestBody"]).get("content"))
            {
                if let Some(media) = content
                    .get("application/json")
                    .or_else(|| content.as_object().and_then(|o| o.values().next()))
                {
                    let example = media
                        .get("example")
                        .cloned()
                        .unwrap_or_else(|| sample_from_schema(spec, &media["schema"], 0));
                    let ct = content
                        .as_object()
                        .and_then(|o| o.keys().next())
                        .cloned()
                        .unwrap_or_default();
                    if ct.contains("form-urlencoded") || ct.contains("multipart") {
                        let fields: Vec<KeyValue> = example
                            .as_object()
                            .map(|o| {
                                o.iter()
                                    .map(|(k, v)| KeyValue::new(k, &crate::tests::plain(v)))
                                    .collect()
                            })
                            .unwrap_or_default();
                        if ct.contains("multipart") {
                            r.body_type = BodyType::FormData;
                            r.form = fields;
                        } else {
                            r.body_type = BodyType::UrlEncoded;
                            r.urlencoded = fields;
                        }
                    } else {
                        r.body_type = BodyType::Raw;
                        r.raw_lang = RawLang::Json;
                        r.raw = serde_json::to_string_pretty(&example).unwrap_or_default();
                    }
                }
            }
            if op
                .get("security")
                .and_then(Value::as_array)
                .is_some_and(|s| s.is_empty())
            {
                r.auth = Auth::None;
            }

            let tag = op["tags"][0].as_str().unwrap_or("").to_string();
            if tag.is_empty() {
                c.items.push(Node::Request(r));
            } else {
                match folders.iter_mut().find(|f| f.name == tag) {
                    Some(f) => f.items.push(Node::Request(r)),
                    None => folders.push(Folder {
                        name: tag,
                        items: vec![Node::Request(r)],
                        ..Default::default()
                    }),
                }
            }
        }
    }
    let loose = std::mem::take(&mut c.items);
    c.items = folders.into_iter().map(Node::Folder).collect();
    c.items.extend(loose);
    Ok(c)
}

// ---------------------------------------------------------------------------------------------
// Insomnia
// ---------------------------------------------------------------------------------------------

/// Insomnia uses `{{ _.name }}` for variables.
fn insomnia_vars(s: &str) -> String {
    s.replace("{{ _.", "{{")
        .replace("{{_.", "{{")
        .replace(" }}", "}}")
}

fn parse_insomnia(json: &Value) -> Result<Imported, String> {
    let resources = json["resources"]
        .as_array()
        .ok_or("No resources in the Insomnia export")?;
    let of = |ty: &'static str| resources.iter().filter(move |r| r["_type"] == ty);

    fn children(resources: &[Value], parent: &str) -> Vec<Node> {
        resources
            .iter()
            .filter(|r| r["parentId"] == parent)
            .filter_map(|r| match r["_type"].as_str() {
                Some("request_group") => Some(Node::Folder(Folder {
                    id: new_id(),
                    name: r["name"].as_str().unwrap_or("Folder").to_string(),
                    items: children(resources, r["_id"].as_str().unwrap_or_default()),
                    ..Default::default()
                })),
                Some("request") => {
                    let mut req = Request::new(r["name"].as_str().unwrap_or("Request"));
                    req.method = r["method"].as_str().unwrap_or("GET").to_uppercase();
                    req.url = insomnia_vars(r["url"].as_str().unwrap_or_default());
                    req.sync_from_url();
                    req.headers = r["headers"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .map(|h| {
                                    KeyValue::new(
                                        h["name"].as_str().unwrap_or(""),
                                        &insomnia_vars(h["value"].as_str().unwrap_or("")),
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let mime = r["body"]["mimeType"].as_str().unwrap_or("");
                    if let Some(text) = r["body"]["text"].as_str() {
                        req.body_type = BodyType::Raw;
                        req.raw = insomnia_vars(text);
                        req.raw_lang = if mime.contains("json") {
                            RawLang::Json
                        } else if mime.contains("xml") {
                            RawLang::Xml
                        } else {
                            RawLang::Text
                        };
                    } else if let Some(params) = r["body"]["params"].as_array() {
                        let fields: Vec<KeyValue> = params
                            .iter()
                            .map(|p| {
                                let mut kv = KeyValue::new(
                                    p["name"].as_str().unwrap_or(""),
                                    &insomnia_vars(p["value"].as_str().unwrap_or("")),
                                );
                                if p["type"] == "file" {
                                    kv.file = true;
                                    kv.value = p["fileName"].as_str().unwrap_or("").to_string();
                                }
                                kv
                            })
                            .collect();
                        if mime.contains("multipart") {
                            req.body_type = BodyType::FormData;
                            req.form = fields;
                        } else {
                            req.body_type = BodyType::UrlEncoded;
                            req.urlencoded = fields;
                        }
                    }
                    match r["authentication"]["type"].as_str() {
                        Some("bearer") => {
                            req.auth = Auth::Bearer {
                                token: insomnia_vars(
                                    r["authentication"]["token"].as_str().unwrap_or(""),
                                ),
                            }
                        }
                        Some("basic") => {
                            req.auth = Auth::Basic {
                                username: insomnia_vars(
                                    r["authentication"]["username"].as_str().unwrap_or(""),
                                ),
                                password: insomnia_vars(
                                    r["authentication"]["password"].as_str().unwrap_or(""),
                                ),
                            }
                        }
                        _ => {}
                    }
                    req.description = r["description"].as_str().unwrap_or_default().to_string();
                    Some(Node::Request(req))
                }
                _ => None,
            })
            .collect()
    }

    let collections: Vec<Collection> = of("workspace")
        .map(|w| {
            let mut c = Collection::new(w["name"].as_str().unwrap_or("Insomnia"));
            c.description = w["description"].as_str().unwrap_or_default().to_string();
            c.items = children(resources, w["_id"].as_str().unwrap_or_default());
            c
        })
        .collect();
    let environments: Vec<Environment> = of("environment")
        .filter_map(|e| {
            let data = e["data"].as_object()?;
            if data.is_empty() {
                return None;
            }
            Some(Environment {
                id: new_id(),
                name: e["name"].as_str().unwrap_or("Environment").to_string(),
                values: data
                    .iter()
                    .map(|(k, v)| KeyValue::new(k, &crate::tests::plain(v)))
                    .collect(),
            })
        })
        .collect();
    if collections.is_empty() {
        return Err("No workspaces in the Insomnia export".into());
    }
    Ok(Imported::Workspace(collections, environments))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curl_json() {
        let r = parse_curl(
            "curl -X POST 'https://api.example.com/users?page=1' \\\n  -H 'Content-Type: application/json' \\\n  -H \"Authorization: Bearer abc\" \\\n  --data-raw '{\"name\":\"Lina\"}'",
        )
        .unwrap();
        assert_eq!(r.method, "POST");
        assert_eq!(r.url, "https://api.example.com/users?page=1");
        assert_eq!(r.params.len(), 1);
        assert_eq!(r.headers.len(), 2);
        assert_eq!(r.body_type, BodyType::Raw);
        assert!(r.raw.contains("\"name\": \"Lina\""));
    }

    #[test]
    fn curl_forms() {
        let r =
            parse_curl("curl https://x.io/up -F 'title=Hi' -F 'photo=@/tmp/a.png' -u me:secret")
                .unwrap();
        assert_eq!(r.method, "POST");
        assert_eq!(r.form.len(), 2);
        assert!(r.form[1].file);
        assert_eq!(
            r.auth,
            Auth::Basic {
                username: "me".into(),
                password: "secret".into()
            }
        );

        let r =
            parse_curl("curl --request PUT --url=https://x.io/a -d 'a=1&b=two%20words'").unwrap();
        assert_eq!(r.method, "PUT");
        assert_eq!(r.body_type, BodyType::UrlEncoded);
        assert_eq!(r.urlencoded[1].value, "two words");
    }

    #[test]
    fn openapi() {
        let spec = json!({
            "openapi": "3.0.0",
            "info": { "title": "Pets" },
            "servers": [{ "url": "https://pets.io/v1/" }],
            "paths": {
                "/pets/{petId}": {
                    "get": { "summary": "Get a pet", "tags": ["pets"], "parameters": [
                        { "name": "petId", "in": "path", "schema": { "type": "integer", "example": 3 } },
                        { "name": "full", "in": "query", "schema": { "type": "boolean" } }
                    ]}
                },
                "/pets": {
                    "post": { "tags": ["pets"], "requestBody": { "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Pet" } } } } }
                }
            },
            "components": { "schemas": { "Pet": { "type": "object", "properties": { "name": { "type": "string" }, "age": { "type": "integer" } } } } }
        });
        let c = parse_openapi(&spec).unwrap();
        assert_eq!(c.variables[0].value, "https://pets.io/v1");
        let reqs = c.requests();
        assert_eq!(reqs.len(), 2);
        assert_eq!(reqs[0].url, "{{baseUrl}}/pets/:petId");
        assert_eq!(reqs[0].path_vars[0].value, "3");
        assert_eq!(reqs[0].params[0].key, "full");
        assert!(reqs[1].raw.contains("\"age\": 0"));
    }

    #[test]
    fn detects_formats() {
        assert!(matches!(
            detect("curl https://a.io"),
            Ok(Imported::Request(_))
        ));
        assert!(matches!(
            detect(r#"{"name":"Dev","values":[{"key":"a","value":"1"}]}"#),
            Ok(Imported::Environment(_))
        ));
        assert!(matches!(
            detect(r#"{"info":{"name":"X"},"item":[]}"#),
            Ok(Imported::Collection(_))
        ));
        assert!(detect("hello").is_err());
    }
    #[test]
    fn detects_yaml_openapi() {
        let yaml = "# Pets API\nopenapi: 3.0.0\ninfo:\n  title: Pets\nservers:\n  - url: https://pets.io\npaths:\n  /pets:\n    get:\n      summary: List pets\n";
        match detect(yaml) {
            Ok(Imported::Collection(c)) => {
                assert_eq!(c.name, "Pets");
                assert_eq!(c.requests()[0].method, "GET");
            }
            _ => panic!("YAML OpenAPI should import as a collection"),
        }
    }

    #[test]
    fn imports_har() {
        let har = json!({ "log": { "creator": { "name": "Chrome" }, "entries": [
            { "request": { "method": "POST", "url": "http://localhost:3000/students?x=1",
                "headers": [ { "name": ":authority", "value": "localhost" }, { "name": "Content-Type", "value": "application/json" }, { "name": "Cookie", "value": "a=b" } ],
                "postData": { "mimeType": "application/json", "text": "{\"name\":\"Mona\"}" } },
              "response": { "status": 201, "headers": [ { "name": "Content-Type", "value": "application/json" } ],
                "content": { "mimeType": "application/json", "text": "{\"id\":5}" } } },
            { "request": { "method": "GET", "url": "http://localhost:3000/students", "headers": [] },
              "response": { "status": 200, "headers": [], "content": { "mimeType": "image/png", "text": "iVBORw0K", "encoding": "base64" } } }
        ] } });
        let c = match detect(&har.to_string()) {
            Ok(Imported::Collection(c)) => c,
            _ => panic!("HAR should import as a collection"),
        };
        let reqs = c.requests();
        assert_eq!(reqs.len(), 2);
        assert_eq!(reqs[0].method, "POST");
        assert_eq!(reqs[0].name, "POST /students");
        assert_eq!(reqs[0].params[0].key, "x");
        assert_eq!(reqs[0].headers.len(), 1, "pseudo-headers and cookies are dropped");
        assert_eq!(reqs[0].body_type, BodyType::Raw);
        assert!(reqs[0].raw.contains("\"name\": \"Mona\""));
        assert_eq!(reqs[0].examples[0].status, 201);
        assert!(reqs[0].examples[0].body.contains("\"id\": 5"));
        assert!(reqs[1].examples[0].body.starts_with("(binary image/png"));
    }
}
