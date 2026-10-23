//! Exports a whole collection to other formats.

use serde_json::{Map, Value, json};

use crate::codegen;
use crate::collection::{self, BodyType, Collection, KeyValue, Node, Request};
use crate::http::{self, Prepared, PreparedBody};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    PostmanJson,
    OpenApi,
    HtmlDocs,
    Markdown,
    Curl,
    HttpFile,
}

impl Format {
    pub const ALL: [Format; 6] = [
        Format::PostmanJson,
        Format::OpenApi,
        Format::HtmlDocs,
        Format::Markdown,
        Format::Curl,
        Format::HttpFile,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Format::PostmanJson => "Postman Collection v2.1 (.json)",
            Format::OpenApi => "OpenAPI 3.0 spec (.json)",
            Format::HtmlDocs => "API documentation page (.html)",
            Format::Markdown => "Markdown documentation (.md)",
            Format::Curl => "All requests as cURL (.sh)",
            Format::HttpFile => "All requests as a .http file",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::PostmanJson => "postman_collection.json",
            Format::OpenApi => "openapi.json",
            Format::HtmlDocs => "html",
            Format::Markdown => "md",
            Format::Curl => "sh",
            Format::HttpFile => "http",
        }
    }
}

/// Every request with its folder path, in tree order.
fn walk<'a>(items: &'a [Node], path: &mut Vec<String>, out: &mut Vec<(Vec<String>, &'a Request)>) {
    for n in items {
        match n {
            Node::Request(r) => out.push((path.clone(), r)),
            Node::Folder(f) => {
                path.push(f.name.clone());
                walk(&f.items, path, out);
                path.pop();
            }
        }
    }
}

fn flat(c: &Collection) -> Vec<(Vec<String>, &Request)> {
    let mut out = Vec::new();
    walk(&c.items, &mut Vec::new(), &mut out);
    out
}

fn full_name(folders: &[String], r: &Request) -> String {
    let mut parts = folders.to_vec();
    parts.push(r.name.clone());
    parts.join(" / ")
}

/// `vars` are the variables to resolve with (globals, collection, environment).
pub fn export(c: &Collection, format: Format, vars: &[KeyValue]) -> String {
    let prepared = |r: &Request| http::prepare(r, &c.effective_auth(r.id, &r.auth), vars);
    match format {
        Format::PostmanJson => {
            serde_json::to_string_pretty(&collection::to_json(c)).unwrap_or_default()
        }
        Format::OpenApi => serde_json::to_string_pretty(&openapi(c, vars)).unwrap_or_default(),
        Format::Curl => {
            let mut out = format!("#!/usr/bin/env bash\n# {}\n\n", c.name);
            for (folders, r) in flat(c) {
                out.push_str(&format!(
                    "# {}\n{}\necho\n\n",
                    full_name(&folders, r),
                    codegen::curl(&prepared(r))
                ));
            }
            out
        }
        Format::HttpFile => flat(c)
            .iter()
            .map(|(f, r)| http_block(&full_name(f, r), &prepared(r)))
            .collect::<Vec<_>>()
            .join("\n"),
        Format::Markdown => markdown(c, &prepared),
        Format::HtmlDocs => html(c, &prepared),
    }
}

fn http_block(name: &str, p: &Prepared) -> String {
    let mut out = format!("### {name}\n{} {}\n", p.method, p.url);
    for (k, v) in &p.headers {
        out.push_str(&format!("{k}: {v}\n"));
    }
    match &p.body {
        Some(PreparedBody::Raw(raw)) if !raw.is_empty() => out.push_str(&format!("\n{raw}\n")),
        Some(PreparedBody::UrlEncoded(fields)) => {
            out.push_str(&format!("\n{}\n", http::encode_urlencoded(fields)))
        }
        Some(PreparedBody::File(path)) => out.push_str(&format!("\n< {path}\n")),
        _ => {}
    }
    out
}

fn markdown(c: &Collection, prepared: &dyn Fn(&Request) -> Prepared) -> String {
    let mut md = format!("# {}\n\n", c.name);
    if !c.description.is_empty() {
        md.push_str(&format!("{}\n\n", c.description));
    }
    if !c.variables.is_empty() {
        md.push_str("## Variables\n\n| Key | Value |\n|---|---|\n");
        for v in &c.variables {
            md.push_str(&format!("| `{}` | `{}` |\n", v.key, v.value));
        }
        md.push('\n');
    }
    md.push_str("## Requests\n\n");
    for (folders, r) in flat(c) {
        md.push_str(&format!(
            "### {}\n\n`{} {}`\n\n",
            full_name(&folders, r),
            r.method,
            r.url
        ));
        if !r.description.is_empty() {
            md.push_str(&format!("{}\n\n", r.description));
        }
        let params: Vec<&KeyValue> = r.params.iter().filter(|p| p.enabled).collect();
        if !params.is_empty() {
            md.push_str("**Query params**\n\n| Key | Value |\n|---|---|\n");
            for p in params {
                md.push_str(&format!("| `{}` | `{}` |\n", p.key, p.value));
            }
            md.push('\n');
        }
        md.push_str(&format!(
            "```bash\n{}\n```\n\n",
            codegen::curl(&prepared(r))
        ));
        for e in &r.examples {
            let lang = if e.content_type().contains("json") { "json" } else { "" };
            md.push_str(&format!("**Example response: {}** (`{} {}`)\n\n```{lang}\n{}\n```\n\n", e.name, e.status, http::reason(e.status), e.body));
        }
    }
    md
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn method_color(m: &str) -> &'static str {
    match m {
        "GET" => "#22c55e",
        "POST" => "#f5a524",
        "PUT" => "#4c8dff",
        "PATCH" => "#a47cf3",
        "DELETE" => "#f04d4d",
        _ => "#8a93a3",
    }
}

fn html(c: &Collection, prepared: &dyn Fn(&Request) -> Prepared) -> String {
    let requests = flat(c);
    let mut nav = String::new();
    let mut body = String::new();
    for (i, (folders, r)) in requests.iter().enumerate() {
        let color = method_color(&r.method);
        nav.push_str(&format!(
            "<a href=\"#r{i}\"><span class=\"m\" style=\"color:{color}\">{}</span>{}</a>\n",
            esc(&r.method),
            esc(&full_name(folders, r))
        ));
        let p = prepared(r);
        let mut section = format!(
            "<section id=\"r{i}\"><div class=\"crumb\">{}</div><h2>{}</h2><div class=\"url\"><span class=\"pill\" style=\"background:{color}\">{}</span><code>{}</code></div>",
            esc(&folders.join(" / ")),
            esc(&r.name),
            esc(&r.method),
            esc(&r.url)
        );
        if !r.description.is_empty() {
            section.push_str(&format!(
                "<p class=\"desc\">{}</p>",
                esc(&r.description).replace('\n', "<br>")
            ));
        }
        let table = |title: &str, rows: &[KeyValue]| -> String {
            let rows: Vec<&KeyValue> = rows
                .iter()
                .filter(|r| r.enabled && !r.key.is_empty())
                .collect();
            if rows.is_empty() {
                return String::new();
            }
            let mut t = format!("<h4>{title}</h4><table>");
            for kv in rows {
                t.push_str(&format!(
                    "<tr><td><code>{}</code></td><td><code>{}</code></td></tr>",
                    esc(&kv.key),
                    esc(&kv.value)
                ));
            }
            t.push_str("</table>");
            t
        };
        section.push_str(&table("Path variables", &r.path_vars));
        section.push_str(&table("Query params", &r.params));
        section.push_str(&table("Headers", &r.headers));
        match r.body_type {
            BodyType::Raw if !r.raw.is_empty() => section.push_str(&format!(
                "<h4>Body ({})</h4><pre>{}</pre>",
                r.raw_lang.label(),
                esc(&r.raw)
            )),
            BodyType::UrlEncoded => {
                section.push_str(&table("Body (x-www-form-urlencoded)", &r.urlencoded))
            }
            BodyType::FormData => section.push_str(&table("Body (form-data)", &r.form)),
            BodyType::GraphQl => {
                section.push_str(&format!("<h4>GraphQL</h4><pre>{}</pre>", esc(&r.gql_query)))
            }
            _ => {}
        }
        section.push_str(&format!("<h4>Example request</h4><pre class=\"code\">{}</pre>", esc(&codegen::curl(&p))));
        for e in &r.examples {
            section.push_str(&format!(
                "<h4>Example response · {} <span class=\"pill\" style=\"background:{}\">{} {}</span></h4><pre>{}</pre>",
                esc(&e.name),
                if e.status < 400 { "#1db36b" } else { "#f04d4d" },
                e.status,
                http::reason(e.status),
                esc(&e.body)
            ));
        }
        section.push_str("</section>\n");
        body.push_str(&section);
    }
    let vars: String = c
        .variables
        .iter()
        .map(|v| {
            format!(
                "<tr><td><code>{{{{{}}}}}</code></td><td><code>{}</code></td></tr>",
                esc(&v.key),
                esc(&v.value)
            )
        })
        .collect();
    let vars = if vars.is_empty() {
        String::new()
    } else {
        format!("<h3>Variables</h3><table>{vars}</table>")
    };
    format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>{title} · API docs</title>
<style>
:root{{--bg:#0e1014;--panel:#12151b;--card:#171b22;--line:#262c36;--text:#e6e9ef;--muted:#8a93a3;--accent:#1db36b}}
@media (prefers-color-scheme: light){{:root{{--bg:#f7f8fa;--panel:#fff;--card:#fff;--line:#e3e6eb;--text:#1a1d23;--muted:#6b7280;--accent:#149954}}}}
*{{box-sizing:border-box}}body{{margin:0;background:var(--bg);color:var(--text);font:15px/1.55 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif;display:flex}}
nav{{width:300px;height:100vh;position:sticky;top:0;overflow:auto;background:var(--panel);border-right:1px solid var(--line);padding:20px 12px}}
nav h1{{font-size:18px;margin:0 8px 16px;display:flex;gap:10px;align-items:center}}
.flag{{width:27px;height:18px;border-radius:3px;background:linear-gradient(#141414 33%,#f5f5f5 33% 66%,#149954 66%);position:relative;overflow:hidden}}
.flag:after{{content:"";position:absolute;left:0;top:0;border-left:12px solid #e4312b;border-top:9px solid transparent;border-bottom:9px solid transparent}}
nav a{{display:block;padding:6px 8px;border-radius:6px;color:var(--text);text-decoration:none;font-size:13px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}}
nav a:hover{{background:var(--card)}}.m{{font:700 11px ui-monospace,Menlo,monospace;display:inline-block;width:56px}}
main{{flex:1;padding:40px 56px;max-width:980px}}main>p{{color:var(--muted)}}
section{{background:var(--card);border:1px solid var(--line);border-radius:14px;padding:24px 28px;margin:22px 0}}
h2{{margin:2px 0 12px;font-size:20px}}h4{{margin:18px 0 8px;color:var(--muted);font-size:12px;text-transform:uppercase;letter-spacing:.06em}}
.crumb{{color:var(--muted);font-size:12px}}.desc{{color:var(--muted)}}
.url{{display:flex;gap:10px;align-items:center;background:var(--bg);border:1px solid var(--line);border-radius:10px;padding:8px 10px;overflow:auto}}
.pill{{color:#fff;font:700 11px ui-monospace,Menlo,monospace;padding:3px 8px;border-radius:6px}}
code,pre{{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:13px}}
pre{{background:var(--bg);border:1px solid var(--line);border-radius:10px;padding:14px;overflow:auto;white-space:pre-wrap;word-break:break-word}}
table{{border-collapse:collapse;width:100%}}td{{border-top:1px solid var(--line);padding:7px 8px;vertical-align:top}}
footer{{color:var(--muted);font-size:12px;margin-top:40px}}
</style></head><body>
<nav><h1><span class="flag"></span>{title}</h1>{nav}</nav>
<main><h1>{title}</h1><p>{desc}</p>{vars}{body}<footer>Generated by Gaza Man 🇵🇸</footer></main>
</body></html>
"#,
        title = esc(&c.name),
        desc = esc(&c.description).replace('\n', "<br>"),
    )
}

fn schema_of(v: &Value) -> Value {
    match v {
        Value::Null => json!({ "nullable": true }),
        Value::Bool(_) => json!({ "type": "boolean" }),
        Value::Number(n) if n.is_i64() || n.is_u64() => json!({ "type": "integer" }),
        Value::Number(_) => json!({ "type": "number" }),
        Value::String(_) => json!({ "type": "string" }),
        Value::Array(a) => {
            json!({ "type": "array", "items": a.first().map(schema_of).unwrap_or(json!({})) })
        }
        Value::Object(o) => {
            let props: Map<String, Value> =
                o.iter().map(|(k, v)| (k.clone(), schema_of(v))).collect();
            json!({ "type": "object", "properties": props })
        }
    }
}

/// A basic OpenAPI 3.0 document: paths, params, headers and JSON body examples.
fn openapi(c: &Collection, vars: &[KeyValue]) -> Value {
    let requests = flat(c);
    // The common prefix of resolved URLs becomes the server.
    let resolved: Vec<String> = requests
        .iter()
        .map(|(_, r)| http::resolve(collection::url_base(&r.url), vars))
        .collect();
    let server = resolved
        .iter()
        .filter_map(|u| {
            u.find("://").map(|i| {
                let rest = &u[i + 3..];
                u[..i + 3 + rest.find('/').unwrap_or(rest.len())].to_string()
            })
        })
        .next()
        .unwrap_or_else(|| "http://localhost".into());

    let mut paths: Map<String, Value> = Map::new();
    for ((folders, r), url) in requests.iter().zip(&resolved) {
        let path = url.strip_prefix(&server).unwrap_or(url);
        let path = if path.is_empty() {
            "/".to_string()
        } else {
            path.to_string()
        };
        let path: String = path
            .split('/')
            .map(|seg| match seg.strip_prefix(':') {
                Some(name) => format!("{{{name}}}"),
                None => seg.to_string(),
            })
            .collect::<Vec<_>>()
            .join("/");
        let mut params: Vec<Value> = Vec::new();
        for v in &r.path_vars {
            params.push(json!({ "name": v.key, "in": "path", "required": true, "schema": { "type": "string" }, "example": http::resolve(&v.value, vars) }));
        }
        for p in r.params.iter().filter(|p| !p.key.is_empty()) {
            params.push(json!({ "name": p.key, "in": "query", "required": p.enabled, "schema": { "type": "string" }, "example": http::resolve(&p.value, vars) }));
        }
        for h in r.headers.iter().filter(|h| {
            h.enabled && !h.key.is_empty() && !h.key.eq_ignore_ascii_case("content-type")
        }) {
            params.push(json!({ "name": h.key, "in": "header", "schema": { "type": "string" }, "example": http::resolve(&h.value, vars) }));
        }
        let mut op = json!({
            "summary": r.name,
            "operationId": format!("{}{}", r.method.to_lowercase(), r.name.split_whitespace().map(|w| {
                let mut c = w.chars();
                c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default()
            }).collect::<String>().replace(|c: char| !c.is_alphanumeric(), "")),
            "responses": { "200": { "description": "Successful response" } },
        });
        if !r.examples.is_empty() {
            let mut responses = Map::new();
            for e in &r.examples {
                let ct = e.content_type().split(';').next().unwrap_or("text/plain").trim().to_string();
                let example = serde_json::from_str::<Value>(&e.body).unwrap_or_else(|_| Value::String(e.body.clone()));
                responses.entry(e.status.to_string()).or_insert_with(|| {
                    json!({ "description": e.name, "content": { ct: { "schema": schema_of(&example), "example": example } } })
                });
            }
            op["responses"] = Value::Object(responses);
        }
        if !folders.is_empty() {
            op["tags"] = json!([folders[0]]);
        }
        if !r.description.is_empty() {
            op["description"] = r.description.clone().into();
        }
        if !params.is_empty() {
            op["parameters"] = Value::Array(params);
        }
        let body = match r.body_type {
            BodyType::Raw => {
                let raw = http::resolve(&r.raw, vars);
                match serde_json::from_str::<Value>(&raw) {
                    Ok(v) => Some((
                        "application/json",
                        json!({ "schema": schema_of(&v), "example": v }),
                    )),
                    Err(_) => Some((
                        r.raw_lang.content_type(),
                        json!({ "schema": { "type": "string" }, "example": raw }),
                    )),
                }
            }
            BodyType::UrlEncoded | BodyType::FormData => {
                let (ct, fields) = if r.body_type == BodyType::UrlEncoded {
                    ("application/x-www-form-urlencoded", &r.urlencoded)
                } else {
                    ("multipart/form-data", &r.form)
                };
                let props: Map<String, Value> = fields
                    .iter()
                    .filter(|f| f.enabled && !f.key.is_empty())
                    .map(|f| {
                        (
                            f.key.clone(),
                            if f.file {
                                json!({ "type": "string", "format": "binary" })
                            } else {
                                json!({ "type": "string" })
                            },
                        )
                    })
                    .collect();
                Some((
                    ct,
                    json!({ "schema": { "type": "object", "properties": props } }),
                ))
            }
            _ => None,
        };
        if let Some((ct, media)) = body {
            op["requestBody"] = json!({ "content": { ct: media } });
        }
        let entry = paths.entry(path).or_insert_with(|| json!({}));
        entry[r.method.to_lowercase()] = op;
    }
    json!({
        "openapi": "3.0.3",
        "info": { "title": c.name, "description": c.description, "version": "1.0.0" },
        "servers": [{ "url": server }],
        "paths": paths,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collection::{Folder, Request};

    #[test]
    fn all_formats() {
        let mut c = Collection::new("Shop");
        c.variables
            .push(KeyValue::new("baseUrl", "https://shop.io/api"));
        let mut r = Request::new("Get product");
        r.url = "{{baseUrl}}/products/:id?lang=ar".into();
        r.sync_from_url();
        r.path_vars[0].value = "5".into();
        let mut post = Request::new("Create product");
        post.method = "POST".into();
        post.url = "{{baseUrl}}/products".into();
        post.body_type = BodyType::Raw;
        post.raw = "{\"name\":\"Olive oil\",\"price\":12.5}".into();
        c.items.push(Node::Folder(Folder {
            name: "Products".into(),
            items: vec![Node::Request(r), Node::Request(post)],
            ..Default::default()
        }));

        let vars = c.variables.clone();
        for f in Format::ALL {
            assert!(!export(&c, f, &vars).is_empty());
        }
        let spec: Value = serde_json::from_str(&export(&c, Format::OpenApi, &vars)).unwrap();
        assert_eq!(spec["servers"][0]["url"], "https://shop.io");
        assert!(spec["paths"]["/api/products/{id}"]["get"].is_object());
        assert_eq!(
            spec["paths"]["/api/products"]["post"]["requestBody"]["content"]["application/json"]["schema"]
                ["properties"]["price"]["type"],
            "number"
        );
    }
}
