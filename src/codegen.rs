//! Turns a prepared request into ready-to-run code in many languages, like Postman's "Code".

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::http::{self, AuthFlow, Part, Prepared, PreparedBody, file_name, mime_for};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lang {
    #[default]
    Curl,
    Wget,
    HttPie,
    JsFetch,
    Axios,
    Python,
    Go,
    Php,
    Java,
    CSharp,
    Swift,
    Kotlin,
    Dart,
    Ruby,
    Rust,
    PowerShell,
    Http,
}

impl Lang {
    pub const ALL: [Lang; 17] = [
        Lang::Curl,
        Lang::JsFetch,
        Lang::Axios,
        Lang::Python,
        Lang::Go,
        Lang::Php,
        Lang::Java,
        Lang::CSharp,
        Lang::Swift,
        Lang::Kotlin,
        Lang::Dart,
        Lang::Ruby,
        Lang::Rust,
        Lang::PowerShell,
        Lang::HttPie,
        Lang::Wget,
        Lang::Http,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Lang::Curl => "cURL",
            Lang::Wget => "Shell - wget",
            Lang::HttPie => "Shell - HTTPie",
            Lang::JsFetch => "JavaScript - Fetch",
            Lang::Axios => "NodeJs - Axios",
            Lang::Python => "Python - Requests",
            Lang::Go => "Go - Native",
            Lang::Php => "PHP - cURL",
            Lang::Java => "Java - HttpClient",
            Lang::CSharp => "C# - HttpClient",
            Lang::Swift => "Swift - URLSession",
            Lang::Kotlin => "Kotlin - OkHttp",
            Lang::Dart => "Dart - http",
            Lang::Ruby => "Ruby - Net::HTTP",
            Lang::Rust => "Rust - reqwest",
            Lang::PowerShell => "PowerShell - RestMethod",
            Lang::Http => "HTTP",
        }
    }

    /// A file extension, for saving the snippet.
    pub fn extension(self) -> &'static str {
        match self {
            Lang::Curl | Lang::Wget | Lang::HttPie => "sh",
            Lang::JsFetch | Lang::Axios => "js",
            Lang::Python => "py",
            Lang::Go => "go",
            Lang::Php => "php",
            Lang::Java => "java",
            Lang::CSharp => "cs",
            Lang::Swift => "swift",
            Lang::Kotlin => "kt",
            Lang::Dart => "dart",
            Lang::Ruby => "rb",
            Lang::Rust => "rs",
            Lang::PowerShell => "ps1",
            Lang::Http => "http",
        }
    }
}

pub fn generate(lang: Lang, p: &Prepared) -> String {
    let code = match lang {
        Lang::Curl => curl(p),
        Lang::Wget => wget(p),
        Lang::HttPie => httpie(p),
        Lang::JsFetch => fetch(p),
        Lang::Axios => axios(p),
        Lang::Python => python(p),
        Lang::Go => go(p),
        Lang::Php => php(p),
        Lang::Java => java(p),
        Lang::CSharp => csharp(p),
        Lang::Swift => swift(p),
        Lang::Kotlin => kotlin(p),
        Lang::Dart => dart(p),
        Lang::Ruby => ruby(p),
        Lang::Rust => rust(p),
        Lang::PowerShell => powershell(p),
        Lang::Http => raw_http(p),
    };
    // cURL and HTTPie do these auth schemes themselves; elsewhere, say what's needed.
    match auth_note(p) {
        Some(note) if !matches!(lang, Lang::Curl) && !(lang == Lang::HttPie && matches!(p.flow, AuthFlow::Digest { .. })) => {
            let comment = match lang {
                Lang::Curl | Lang::Wget | Lang::HttPie | Lang::Python | Lang::Ruby | Lang::PowerShell | Lang::Http => "#",
                _ => "//",
            };
            format!("{comment} {note}\n{code}")
        }
        _ => code,
    }
}

/// What a snippet must add itself for auth Gaza Man does while sending.
fn auth_note(p: &Prepared) -> Option<String> {
    Some(match &p.flow {
        AuthFlow::None => return None,
        AuthFlow::Digest { username, .. } => format!("Auth: HTTP Digest as \"{username}\" (use your HTTP client's digest support)."),
        AuthFlow::Ntlm { username, .. } => format!("Auth: NTLM as \"{username}\" (use your HTTP client's NTLM support)."),
        AuthFlow::Aws { region, service, .. } => format!("Auth: sign the request with AWS Signature V4 (region {region}, service {service})."),
    })
}

// ---------------------------------------------------------------------------------------------
// Quoting helpers
// ---------------------------------------------------------------------------------------------

/// A double-quoted string with JSON escapes; valid in JS, Python, Java, C#, Go and others.
fn dq(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// Like [`dq`] but with `\u{..}` escapes, for Swift and Rust.
fn dq_braced(s: &str) -> String {
    let q = dq(s);
    let mut out = String::with_capacity(q.len());
    let mut rest = q.as_str();
    while let Some(i) = rest.find("\\u") {
        // Only a real escape if the backslash itself isn't escaped.
        let backslashes = rest[..i].chars().rev().take_while(|c| *c == '\\').count();
        out.push_str(&rest[..i]);
        if backslashes % 2 == 0 && rest.len() >= i + 6 {
            out.push_str(&format!("\\u{{{}}}", &rest[i + 2..i + 6]));
            rest = &rest[i + 6..];
        } else {
            out.push_str("\\u");
            rest = &rest[i + 2..];
        }
    }
    out.push_str(rest);
    out
}

/// Double-quoted, with `$` escaped, for Kotlin and Dart.
fn dq_dollar(s: &str) -> String {
    dq(s).replace('$', "\\$")
}

fn sh(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Single-quoted for PHP and Ruby.
fn sq(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn ps(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn header<'a>(p: &'a Prepared, name: &str) -> Option<&'a str> {
    p.headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn content_type(p: &Prepared) -> Option<&str> {
    header(p, "content-type")
}

fn headers_without_ct(p: &Prepared) -> Vec<&(String, String)> {
    p.headers
        .iter()
        .filter(|(k, _)| !k.eq_ignore_ascii_case("content-type"))
        .collect()
}

/// The raw body pretty-printed if it's JSON.
fn json_body(raw: &str) -> Option<Value> {
    serde_json::from_str::<Value>(raw)
        .ok()
        .filter(|v| v.is_object() || v.is_array())
}

fn indent(s: &str, pad: &str) -> String {
    s.lines()
        .enumerate()
        .map(|(i, l)| {
            if i == 0 {
                l.to_string()
            } else {
                format!("{pad}{l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn python_literal(v: &Value, depth: usize) -> String {
    let pad = "  ".repeat(depth + 1);
    let end = "  ".repeat(depth);
    match v {
        Value::Null => "None".into(),
        Value::Bool(b) => if *b { "True" } else { "False" }.into(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => dq(s),
        Value::Array(a) if a.is_empty() => "[]".into(),
        Value::Array(a) => {
            let items: Vec<String> = a
                .iter()
                .map(|x| format!("{pad}{}", python_literal(x, depth + 1)))
                .collect();
            format!("[\n{}\n{end}]", items.join(",\n"))
        }
        Value::Object(o) if o.is_empty() => "{}".into(),
        Value::Object(o) => {
            let items: Vec<String> = o
                .iter()
                .map(|(k, x)| format!("{pad}{}: {}", dq(k), python_literal(x, depth + 1)))
                .collect();
            format!("{{\n{}\n{end}}}", items.join(",\n"))
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Shell
// ---------------------------------------------------------------------------------------------

pub fn curl(p: &Prepared) -> String {
    let mut parts = vec![format!(
        "curl --location --request {} {}",
        p.method,
        sh(&p.url)
    )];
    match &p.flow {
        AuthFlow::None => {}
        AuthFlow::Digest { username, password } => parts.push(format!("--digest --user {}", sh(&format!("{username}:{password}")))),
        AuthFlow::Ntlm { username, password, domain, .. } => {
            let user = if domain.is_empty() { username.clone() } else { format!("{domain}\\{username}") };
            parts.push(format!("--ntlm --user {}", sh(&format!("{user}:{password}"))));
        }
        AuthFlow::Aws { access_key, secret_key, region, service, session_token } => {
            parts.push(format!("--aws-sigv4 {}", sh(&format!("aws:amz:{region}:{service}"))));
            parts.push(format!("--user {}", sh(&format!("{access_key}:{secret_key}"))));
            if !session_token.is_empty() {
                parts.push(format!("--header {}", sh(&format!("x-amz-security-token: {session_token}"))));
            }
        }
    }
    for (k, v) in &p.headers {
        parts.push(format!("--header {}", sh(&format!("{k}: {v}"))));
    }
    match &p.body {
        Some(PreparedBody::Raw(raw)) if !raw.is_empty() => {
            parts.push(format!("--data-raw {}", sh(raw)))
        }
        Some(PreparedBody::UrlEncoded(fields)) => parts.extend(
            fields
                .iter()
                .map(|(k, v)| format!("--data-urlencode {}", sh(&format!("{k}={v}")))),
        ),
        Some(PreparedBody::Multipart(parts_)) => parts.extend(parts_.iter().map(|f| {
            if f.file {
                format!("--form {}", sh(&format!("{}=@\"{}\"", f.name, f.value)))
            } else {
                format!("--form {}", sh(&format!("{}={}", f.name, f.value)))
            }
        })),
        Some(PreparedBody::File(path)) => {
            parts.push(format!("--data-binary {}", sh(&format!("@{path}"))))
        }
        _ => {}
    }
    parts.join(" \\\n  ")
}

fn wget(p: &Prepared) -> String {
    let mut parts = vec!["wget --quiet".to_string(), format!("--method={}", p.method)];
    for (k, v) in &p.headers {
        parts.push(format!("--header={}", sh(&format!("{k}: {v}"))));
    }
    let mut note = String::new();
    match &p.body {
        Some(PreparedBody::Raw(raw)) if !raw.is_empty() => {
            parts.push(format!("--body-data={}", sh(raw)))
        }
        Some(PreparedBody::UrlEncoded(f)) => {
            parts.push(format!("--body-data={}", sh(&http::encode_urlencoded(f))))
        }
        Some(PreparedBody::File(path)) => parts.push(format!("--body-file={}", sh(path))),
        Some(PreparedBody::Multipart(_)) => {
            note =
                "# wget can't build multipart/form-data bodies; use cURL or HTTPie for this one.\n"
                    .into()
        }
        _ => {}
    }
    parts.push("--output-document=-".into());
    parts.push(sh(&p.url));
    format!("{note}{}", parts.join(" \\\n  "))
}

fn httpie(p: &Prepared) -> String {
    let mut flags = vec!["http".to_string(), "--ignore-stdin".to_string()];
    if let AuthFlow::Digest { username, password } = &p.flow {
        flags.push(format!("--auth-type=digest --auth {}", sh(&format!("{username}:{password}"))));
    }
    let mut items: Vec<String> = Vec::new();
    let mut tail = String::new();
    match &p.body {
        Some(PreparedBody::Raw(raw)) if !raw.is_empty() => items.push(format!("--raw {}", sh(raw))),
        Some(PreparedBody::UrlEncoded(f)) => {
            flags.push("--form".into());
            items.extend(f.iter().map(|(k, v)| sh(&format!("{k}={v}"))));
        }
        Some(PreparedBody::Multipart(parts)) => {
            flags.push("--multipart".into());
            items.extend(parts.iter().map(|f| {
                sh(&format!(
                    "{}{}{}",
                    f.name,
                    if f.file { "@" } else { "=" },
                    f.value
                ))
            }));
        }
        Some(PreparedBody::File(path)) => {
            flags.retain(|f| f != "--ignore-stdin");
            tail = format!(" < {}", sh(path));
        }
        _ => {}
    }
    let mut parts = vec![format!("{} {} {}", flags.join(" "), p.method, sh(&p.url))];
    parts.extend(p.headers.iter().map(|(k, v)| sh(&format!("{k}:{v}"))));
    parts.extend(items);
    format!("{}{tail}", parts.join(" \\\n  "))
}

fn raw_http(p: &Prepared) -> String {
    let (_, host, _) = http::split_url(&p.url);
    let after_scheme = p.url.split_once("://").map(|(_, r)| r).unwrap_or(&p.url);
    let target = after_scheme
        .find('/')
        .map(|i| &after_scheme[i..])
        .unwrap_or("/");
    let host_port = after_scheme.split('/').next().unwrap_or(&host);
    let mut out = format!("{} {target} HTTP/1.1\nHost: {host_port}\n", p.method);
    for (k, v) in &p.headers {
        out.push_str(&format!("{k}: {v}\n"));
    }
    match &p.body {
        Some(PreparedBody::Raw(raw)) if !raw.is_empty() => out.push_str(&format!("\n{raw}")),
        Some(PreparedBody::UrlEncoded(f)) => {
            out.push_str(&format!("\n{}", http::encode_urlencoded(f)))
        }
        Some(PreparedBody::File(path)) => out.push_str(&format!("\n< {path}")),
        Some(PreparedBody::Multipart(parts)) => {
            let b = "----GazaManBoundary";
            out.push_str(&format!(
                "Content-Type: multipart/form-data; boundary={b}\n\n"
            ));
            for f in parts {
                if f.file {
                    out.push_str(&format!(
                        "--{b}\nContent-Disposition: form-data; name=\"{}\"; filename=\"{}\"\nContent-Type: {}\n\n< {}\n",
                        f.name,
                        file_name(&f.value),
                        mime_for(&f.value),
                        f.value
                    ));
                } else {
                    out.push_str(&format!(
                        "--{b}\nContent-Disposition: form-data; name=\"{}\"\n\n{}\n",
                        f.name, f.value
                    ));
                }
            }
            out.push_str(&format!("--{b}--"));
        }
        _ => {}
    }
    out
}

fn powershell(p: &Prepared) -> String {
    let mut out = String::new();
    let headers = headers_without_ct(p);
    if !headers.is_empty() {
        out.push_str(
            "$headers = New-Object \"System.Collections.Generic.Dictionary[[String],[String]]\"\n",
        );
        for (k, v) in &headers {
            out.push_str(&format!("$headers.Add({}, {})\n", ps(k), ps(v)));
        }
        out.push('\n');
    }
    let mut args = vec![
        format!("Invoke-RestMethod {}", ps(&p.url)),
        format!("-Method {}", ps(&p.method)),
    ];
    if !headers.is_empty() {
        args.push("-Headers $headers".into());
    }
    if let Some(ct) = content_type(p) {
        args.push(format!("-ContentType {}", ps(ct)));
    }
    match &p.body {
        Some(PreparedBody::Raw(raw)) if !raw.is_empty() => {
            out.push_str(&format!("$body = @'\n{raw}\n'@\n\n"));
            args.push("-Body $body".into());
        }
        Some(PreparedBody::UrlEncoded(f)) => {
            let items: Vec<String> = f
                .iter()
                .map(|(k, v)| format!("    {} = {}", ps(k), ps(v)))
                .collect();
            out.push_str(&format!("$body = @{{\n{}\n}}\n\n", items.join("\n")));
            args.push("-Body $body".into());
        }
        Some(PreparedBody::Multipart(parts)) => {
            let items: Vec<String> = parts
                .iter()
                .map(|f| {
                    if f.file {
                        format!("    {} = Get-Item -Path {}", ps(&f.name), ps(&f.value))
                    } else {
                        format!("    {} = {}", ps(&f.name), ps(&f.value))
                    }
                })
                .collect();
            out.push_str(&format!("$form = @{{\n{}\n}}\n\n", items.join("\n")));
            args.push("-Form $form".into());
        }
        Some(PreparedBody::File(path)) => args.push(format!("-InFile {}", ps(path))),
        _ => {}
    }
    out.push_str(&format!(
        "$response = {}\n$response | ConvertTo-Json -Depth 10",
        args.join(" `\n  ")
    ));
    out
}

// ---------------------------------------------------------------------------------------------
// JavaScript
// ---------------------------------------------------------------------------------------------

fn fetch(p: &Prepared) -> String {
    let mut out = String::new();
    let headers: Vec<&(String, String)> = match &p.body {
        // The browser sets the multipart boundary itself.
        Some(PreparedBody::Multipart(_)) => headers_without_ct(p),
        _ => p.headers.iter().collect(),
    };
    if !headers.is_empty() {
        out.push_str("const headers = new Headers();\n");
        for (k, v) in &headers {
            out.push_str(&format!("headers.append({}, {});\n", dq(k), dq(v)));
        }
        out.push('\n');
    }
    let has_body = match &p.body {
        Some(PreparedBody::Raw(raw)) if !raw.is_empty() => {
            match json_body(raw) {
                Some(v) => out.push_str(&format!(
                    "const body = JSON.stringify({});\n\n",
                    serde_json::to_string_pretty(&v).unwrap_or_default()
                )),
                None => out.push_str(&format!("const body = {};\n\n", dq(raw))),
            }
            true
        }
        Some(PreparedBody::UrlEncoded(f)) => {
            out.push_str("const body = new URLSearchParams();\n");
            for (k, v) in f {
                out.push_str(&format!("body.append({}, {});\n", dq(k), dq(v)));
            }
            out.push('\n');
            true
        }
        Some(PreparedBody::Multipart(parts)) => {
            out.push_str("const body = new FormData();\n");
            for f in parts {
                if f.file {
                    out.push_str(&format!(
                        "body.append({}, fileInput.files[0], {}); // {}\n",
                        dq(&f.name),
                        dq(file_name(&f.value)),
                        f.value
                    ));
                } else {
                    out.push_str(&format!(
                        "body.append({}, {});\n",
                        dq(&f.name),
                        dq(&f.value)
                    ));
                }
            }
            out.push('\n');
            true
        }
        Some(PreparedBody::File(path)) => {
            out.push_str(&format!("const body = fileInput.files[0]; // {path}\n\n"));
            true
        }
        _ => false,
    };
    out.push_str(&format!(
        "const response = await fetch({}, {{\n  method: {},\n",
        dq(&p.url),
        dq(&p.method)
    ));
    if !headers.is_empty() {
        out.push_str("  headers,\n");
    }
    if has_body {
        out.push_str("  body,\n");
    }
    out.push_str("  redirect: \"follow\",\n});\n\nconsole.log(response.status);\nconsole.log(await response.text());");
    out
}

fn axios(p: &Prepared) -> String {
    let mut out = String::from("const axios = require(\"axios\");\n");
    let mut data = None;
    let mut form_headers = false;
    match &p.body {
        Some(PreparedBody::Raw(raw)) if !raw.is_empty() => {
            match json_body(raw) {
                Some(v) => out.push_str(&format!(
                    "\nconst data = JSON.stringify({});\n",
                    serde_json::to_string_pretty(&v).unwrap_or_default()
                )),
                None => out.push_str(&format!("\nconst data = {};\n", dq(raw))),
            }
            data = Some("data");
        }
        Some(PreparedBody::UrlEncoded(f)) => {
            out.push_str("\nconst data = new URLSearchParams();\n");
            for (k, v) in f {
                out.push_str(&format!("data.append({}, {});\n", dq(k), dq(v)));
            }
            data = Some("data");
        }
        Some(PreparedBody::Multipart(parts)) => {
            out.push_str("const FormData = require(\"form-data\");\n");
            if parts.iter().any(|f| f.file) {
                out.push_str("const fs = require(\"fs\");\n");
            }
            out.push_str("\nconst data = new FormData();\n");
            for f in parts {
                if f.file {
                    out.push_str(&format!(
                        "data.append({}, fs.createReadStream({}));\n",
                        dq(&f.name),
                        dq(&f.value)
                    ));
                } else {
                    out.push_str(&format!(
                        "data.append({}, {});\n",
                        dq(&f.name),
                        dq(&f.value)
                    ));
                }
            }
            data = Some("data");
            form_headers = true;
        }
        Some(PreparedBody::File(path)) => {
            out.push_str(&format!(
                "const fs = require(\"fs\");\n\nconst data = fs.readFileSync({});\n",
                dq(path)
            ));
            data = Some("data");
        }
        _ => {}
    }
    out.push_str(&format!(
        "\nconst config = {{\n  method: {},\n  maxBodyLength: Infinity,\n  url: {},\n",
        dq(&p.method.to_lowercase()),
        dq(&p.url)
    ));
    let headers: Vec<&(String, String)> = if form_headers {
        headers_without_ct(p)
    } else {
        p.headers.iter().collect()
    };
    if !headers.is_empty() || form_headers {
        out.push_str("  headers: {\n");
        for (k, v) in headers {
            out.push_str(&format!("    {}: {},\n", dq(k), dq(v)));
        }
        if form_headers {
            out.push_str("    ...data.getHeaders(),\n");
        }
        out.push_str("  },\n");
    }
    if let Some(d) = data {
        out.push_str(&format!("  data: {d},\n"));
    }
    out.push_str("};\n\naxios\n  .request(config)\n  .then((response) => console.log(JSON.stringify(response.data, null, 2)))\n  .catch((error) => console.log(error));");
    out
}

// ---------------------------------------------------------------------------------------------
// Python, Ruby, PHP
// ---------------------------------------------------------------------------------------------

fn python(p: &Prepared) -> String {
    let mut imports = vec!["import requests"];
    let mut pre = String::new();
    let mut args = vec!["headers=headers".to_string()];
    let mut close = String::new();
    match &p.body {
        Some(PreparedBody::Raw(raw)) if !raw.is_empty() => {
            match json_body(raw) {
                Some(v) => {
                    imports.push("import json");
                    pre.push_str(&format!(
                        "payload = json.dumps({})\n",
                        python_literal(&v, 0)
                    ));
                }
                None => pre.push_str(&format!("payload = {}\n", dq(raw))),
            }
            args.push("data=payload".into());
        }
        Some(PreparedBody::UrlEncoded(f)) => {
            let items: Vec<String> = f
                .iter()
                .map(|(k, v)| format!("  ({}, {})", dq(k), dq(v)))
                .collect();
            pre.push_str(&format!("payload = [\n{}\n]\n", items.join(",\n")));
            args.push("data=payload".into());
        }
        Some(PreparedBody::Multipart(parts)) => {
            let fields: Vec<String> = parts
                .iter()
                .filter(|f| !f.file)
                .map(|f| format!("  ({}, {})", dq(&f.name), dq(&f.value)))
                .collect();
            let files: Vec<String> = parts
                .iter()
                .filter(|f| f.file)
                .map(|f| {
                    format!(
                        "  ({}, ({}, open({}, \"rb\"), {}))",
                        dq(&f.name),
                        dq(file_name(&f.value)),
                        dq(&f.value),
                        dq(mime_for(&f.value))
                    )
                })
                .collect();
            pre.push_str(&format!("payload = [\n{}\n]\n", fields.join(",\n")));
            args.push("data=payload".into());
            if !files.is_empty() {
                pre.push_str(&format!("files = [\n{}\n]\n", files.join(",\n")));
                args.push("files=files".into());
            }
        }
        Some(PreparedBody::File(path)) => {
            pre.push_str(&format!("payload = open({}, \"rb\")\n", dq(path)));
            args.push("data=payload".into());
            close = "\npayload.close()".into();
        }
        _ => {}
    }
    let headers: Vec<&(String, String)> = match &p.body {
        // requests writes the multipart Content-Type (with its boundary) itself.
        Some(PreparedBody::Multipart(_)) => headers_without_ct(p),
        _ => p.headers.iter().collect(),
    };
    let header_lines: Vec<String> = headers
        .iter()
        .map(|(k, v)| format!("  {}: {}", dq(k), dq(v)))
        .collect();
    let headers_src = if header_lines.is_empty() {
        "{}".to_string()
    } else {
        format!("{{\n{}\n}}", header_lines.join(",\n"))
    };
    format!(
        "{}\n\nurl = {}\n\n{pre}headers = {headers_src}\n\nresponse = requests.request({}, url, {})\n{close}\nprint(response.status_code)\nprint(response.text)",
        imports.join("\n"),
        dq(&p.url),
        dq(&p.method),
        args.join(", ")
    )
}

fn ruby(p: &Prepared) -> String {
    let class = match p.method.as_str() {
        "GET" => "Get",
        "POST" => "Post",
        "PUT" => "Put",
        "PATCH" => "Patch",
        "DELETE" => "Delete",
        "HEAD" => "Head",
        "OPTIONS" => "Options",
        _ => "",
    };
    let mut out = format!(
        "require \"uri\"\nrequire \"net/http\"\n\nurl = URI({})\n\nhttp = Net::HTTP.new(url.host, url.port)\nhttp.use_ssl = url.scheme == \"https\"\n\n",
        sq(&p.url)
    );
    if class.is_empty() {
        out.push_str(&format!(
            "request = Net::HTTPGenericRequest.new({}, true, true, url)\n",
            sq(&p.method)
        ));
    } else {
        out.push_str(&format!("request = Net::HTTP::{class}.new(url)\n"));
    }
    let multipart = matches!(p.body, Some(PreparedBody::Multipart(_)));
    for (k, v) in &p.headers {
        if multipart && k.eq_ignore_ascii_case("content-type") {
            continue;
        }
        out.push_str(&format!("request[{}] = {}\n", sq(k), sq(v)));
    }
    match &p.body {
        Some(PreparedBody::Raw(raw)) if !raw.is_empty() => {
            out.push_str(&format!("request.body = {}\n", sq(raw)))
        }
        Some(PreparedBody::UrlEncoded(f)) => {
            let items: Vec<String> = f
                .iter()
                .map(|(k, v)| format!("[{}, {}]", sq(k), sq(v)))
                .collect();
            out.push_str(&format!(
                "request.set_form([{}], \"application/x-www-form-urlencoded\")\n",
                items.join(", ")
            ));
        }
        Some(PreparedBody::Multipart(parts)) => {
            let items: Vec<String> = parts
                .iter()
                .map(|f| {
                    if f.file {
                        format!("[{}, File.open({})]", sq(&f.name), sq(&f.value))
                    } else {
                        format!("[{}, {}]", sq(&f.name), sq(&f.value))
                    }
                })
                .collect();
            out.push_str(&format!(
                "request.set_form([{}], \"multipart/form-data\")\n",
                items.join(", ")
            ));
        }
        Some(PreparedBody::File(path)) => {
            out.push_str(&format!("request.body = File.binread({})\n", sq(path)))
        }
        _ => {}
    }
    out.push_str("\nresponse = http.request(request)\nputs response.code\nputs response.read_body");
    out
}

fn php(p: &Prepared) -> String {
    let mut opts = vec![
        format!("CURLOPT_URL => {}", sq(&p.url)),
        "CURLOPT_RETURNTRANSFER => true".into(),
        "CURLOPT_FOLLOWLOCATION => true".into(),
        "CURLOPT_TIMEOUT => 30".into(),
        format!("CURLOPT_CUSTOMREQUEST => {}", sq(&p.method)),
    ];
    let multipart = matches!(p.body, Some(PreparedBody::Multipart(_)));
    match &p.body {
        Some(PreparedBody::Raw(raw)) if !raw.is_empty() => {
            opts.push(format!("CURLOPT_POSTFIELDS => {}", sq(raw)))
        }
        Some(PreparedBody::UrlEncoded(f)) => opts.push(format!(
            "CURLOPT_POSTFIELDS => {}",
            sq(&http::encode_urlencoded(f))
        )),
        Some(PreparedBody::Multipart(parts)) => {
            let items: Vec<String> = parts
                .iter()
                .map(|f| {
                    if f.file {
                        format!("    {} => new CURLFile({})", sq(&f.name), sq(&f.value))
                    } else {
                        format!("    {} => {}", sq(&f.name), sq(&f.value))
                    }
                })
                .collect();
            opts.push(format!(
                "CURLOPT_POSTFIELDS => array(\n{}\n  )",
                items.join(",\n")
            ));
        }
        Some(PreparedBody::File(path)) => opts.push(format!(
            "CURLOPT_POSTFIELDS => file_get_contents({})",
            sq(path)
        )),
        _ => {}
    }
    let headers: Vec<String> = p
        .headers
        .iter()
        .filter(|(k, _)| !(multipart && k.eq_ignore_ascii_case("content-type")))
        .map(|(k, v)| format!("    {}", sq(&format!("{k}: {v}"))))
        .collect();
    if !headers.is_empty() {
        opts.push(format!(
            "CURLOPT_HTTPHEADER => array(\n{}\n  )",
            headers.join(",\n")
        ));
    }
    format!(
        "<?php\n\n$curl = curl_init();\n\ncurl_setopt_array($curl, array(\n  {},\n));\n\n$response = curl_exec($curl);\n$status = curl_getinfo($curl, CURLINFO_HTTP_CODE);\n\ncurl_close($curl);\necho $status . \"\\n\";\necho $response;\n",
        opts.join(",\n  ")
    )
}

// ---------------------------------------------------------------------------------------------
// Go, Rust
// ---------------------------------------------------------------------------------------------

fn go_string(s: &str) -> String {
    if s.contains('`') {
        dq(s)
    } else {
        format!("`{s}`")
    }
}

fn go(p: &Prepared) -> String {
    let mut imports = vec!["fmt", "io", "net/http"];
    let mut setup = String::new();
    let mut payload = "nil";
    let mut after = String::new();
    match &p.body {
        Some(PreparedBody::Raw(raw)) if !raw.is_empty() => {
            imports.push("strings");
            setup.push_str(&format!(
                "\tpayload := strings.NewReader({})\n\n",
                go_string(raw)
            ));
            payload = "payload";
        }
        Some(PreparedBody::UrlEncoded(f)) => {
            imports.push("strings");
            setup.push_str(&format!(
                "\tpayload := strings.NewReader({})\n\n",
                dq(&http::encode_urlencoded(f))
            ));
            payload = "payload";
        }
        Some(PreparedBody::Multipart(parts)) => {
            imports.extend(["bytes", "mime/multipart"]);
            setup.push_str("\tvar err error\n\tpayload := &bytes.Buffer{}\n\twriter := multipart.NewWriter(payload)\n");
            for (i, f) in parts.iter().enumerate() {
                if f.file {
                    imports.extend(["os", "path/filepath"]);
                    let n = i + 1;
                    setup.push_str(&format!(
                        "\tfile{n}, err := os.Open({path})\n\tif err != nil {{\n\t\tfmt.Println(err)\n\t\treturn\n\t}}\n\tdefer file{n}.Close()\n\tpart{n}, err := writer.CreateFormFile({name}, filepath.Base({path}))\n\tif err != nil {{\n\t\tfmt.Println(err)\n\t\treturn\n\t}}\n\tif _, err = io.Copy(part{n}, file{n}); err != nil {{\n\t\tfmt.Println(err)\n\t\treturn\n\t}}\n",
                        path = dq(&f.value),
                        name = dq(&f.name)
                    ));
                } else {
                    setup.push_str(&format!(
                        "\t_ = writer.WriteField({}, {})\n",
                        dq(&f.name),
                        dq(&f.value)
                    ));
                }
            }
            setup.push_str("\tif err = writer.Close(); err != nil {\n\t\tfmt.Println(err)\n\t\treturn\n\t}\n\n");
            payload = "payload";
            after = "\treq.Header.Set(\"Content-Type\", writer.FormDataContentType())\n".into();
        }
        Some(PreparedBody::File(path)) => {
            imports.push("os");
            setup.push_str(&format!("\tpayload, err := os.Open({})\n\tif err != nil {{\n\t\tfmt.Println(err)\n\t\treturn\n\t}}\n\tdefer payload.Close()\n\n", dq(path)));
            payload = "payload";
        }
        _ => {}
    }
    imports.sort_unstable();
    imports.dedup();
    let multipart = matches!(p.body, Some(PreparedBody::Multipart(_)));
    let mut headers = String::new();
    for (k, v) in &p.headers {
        if multipart && k.eq_ignore_ascii_case("content-type") {
            continue;
        }
        headers.push_str(&format!("\treq.Header.Add({}, {})\n", dq(k), dq(v)));
    }
    format!(
        "package main\n\nimport (\n{}\n)\n\nfunc main() {{\n\turl := {}\n\tmethod := {}\n\n{setup}\tclient := &http.Client{{}}\n\treq, err := http.NewRequest(method, url, {payload})\n\tif err != nil {{\n\t\tfmt.Println(err)\n\t\treturn\n\t}}\n{headers}{after}\n\tres, err := client.Do(req)\n\tif err != nil {{\n\t\tfmt.Println(err)\n\t\treturn\n\t}}\n\tdefer res.Body.Close()\n\n\tbody, err := io.ReadAll(res.Body)\n\tif err != nil {{\n\t\tfmt.Println(err)\n\t\treturn\n\t}}\n\tfmt.Println(res.Status)\n\tfmt.Println(string(body))\n}}\n",
        imports
            .iter()
            .map(|i| format!("\t\"{i}\""))
            .collect::<Vec<_>>()
            .join("\n"),
        dq(&p.url),
        dq(&p.method),
    )
}

fn rust_string(s: &str) -> String {
    if s.contains('\n') && !s.contains("\"#") {
        format!("r#\"{s}\"#")
    } else {
        dq_braced(s)
    }
}

fn rust(p: &Prepared) -> String {
    let method = match p.method.as_str() {
        m @ ("GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS") => {
            format!("reqwest::Method::{m}")
        }
        m => format!("reqwest::Method::from_bytes(b{})?", dq(m)),
    };
    let multipart = matches!(p.body, Some(PreparedBody::Multipart(_)));
    let features = if multipart {
        "\"blocking\", \"multipart\""
    } else {
        "\"blocking\""
    };
    let mut out = format!(
        "// Cargo.toml: reqwest = {{ version = \"0.12\", features = [{features}] }}\n\nfn main() -> Result<(), Box<dyn std::error::Error>> {{\n"
    );
    if let Some(PreparedBody::Multipart(parts)) = &p.body {
        out.push_str("    let form = reqwest::blocking::multipart::Form::new()");
        for f in parts {
            if f.file {
                out.push_str(&format!(
                    "\n        .file({}, {})?",
                    dq_braced(&f.name),
                    dq_braced(&f.value)
                ));
            } else {
                out.push_str(&format!(
                    "\n        .text({}, {})",
                    dq_braced(&f.name),
                    dq_braced(&f.value)
                ));
            }
        }
        out.push_str(";\n\n");
    }
    out.push_str(&format!("    let client = reqwest::blocking::Client::new();\n    let response = client\n        .request({method}, {})\n", dq_braced(&p.url)));
    for (k, v) in &p.headers {
        if multipart && k.eq_ignore_ascii_case("content-type") {
            continue;
        }
        out.push_str(&format!(
            "        .header({}, {})\n",
            dq_braced(k),
            dq_braced(v)
        ));
    }
    match &p.body {
        Some(PreparedBody::Raw(raw)) if !raw.is_empty() => out.push_str(&format!(
            "        .body({})\n",
            indent(&rust_string(raw), "")
        )),
        Some(PreparedBody::UrlEncoded(f)) => {
            let items: Vec<String> = f
                .iter()
                .map(|(k, v)| format!("({}, {})", dq_braced(k), dq_braced(v)))
                .collect();
            out.push_str(&format!("        .form(&[{}])\n", items.join(", ")));
        }
        Some(PreparedBody::Multipart(_)) => out.push_str("        .multipart(form)\n"),
        Some(PreparedBody::File(path)) => out.push_str(&format!(
            "        .body(std::fs::read({})?)\n",
            dq_braced(path)
        )),
        _ => {}
    }
    out.push_str("        .send()?;\n\n    println!(\"{}\", response.status());\n    println!(\"{}\", response.text()?);\n    Ok(())\n}\n");
    out
}

// ---------------------------------------------------------------------------------------------
// JVM, .NET, Apple, Dart
// ---------------------------------------------------------------------------------------------

/// The parts of a multipart body as text chunks and file paths, for languages that build it by hand.
enum Chunk {
    Text(String),
    File(String),
}

fn manual_multipart(parts: &[Part], boundary: &str) -> Vec<Chunk> {
    let mut out = Vec::new();
    for f in parts {
        if f.file {
            out.push(Chunk::Text(format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: {}\r\n\r\n",
                f.name,
                file_name(&f.value),
                mime_for(&f.value)
            )));
            out.push(Chunk::File(f.value.clone()));
            out.push(Chunk::Text("\r\n".into()));
        } else {
            out.push(Chunk::Text(format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"\r\n\r\n{}\r\n",
                f.name, f.value
            )));
        }
    }
    out.push(Chunk::Text(format!("--{boundary}--\r\n")));
    out
}

const BOUNDARY: &str = "----GazaManBoundary7MA4YWxkTrZu0gW";

fn java(p: &Prepared) -> String {
    let mut imports = vec![
        "java.net.URI",
        "java.net.http.HttpClient",
        "java.net.http.HttpRequest",
        "java.net.http.HttpResponse",
    ];
    let mut setup = String::new();
    let mut extra_header = String::new();
    let publisher = match &p.body {
        Some(PreparedBody::Raw(raw)) if !raw.is_empty() => {
            format!("HttpRequest.BodyPublishers.ofString({})", dq(raw))
        }
        Some(PreparedBody::UrlEncoded(f)) => format!(
            "HttpRequest.BodyPublishers.ofString({})",
            dq(&http::encode_urlencoded(f))
        ),
        Some(PreparedBody::File(path)) => {
            imports.push("java.nio.file.Path");
            format!("HttpRequest.BodyPublishers.ofFile(Path.of({}))", dq(path))
        }
        Some(PreparedBody::Multipart(parts)) => {
            imports.extend([
                "java.io.ByteArrayOutputStream",
                "java.nio.charset.StandardCharsets",
                "java.nio.file.Files",
                "java.nio.file.Path",
            ]);
            setup.push_str("        ByteArrayOutputStream body = new ByteArrayOutputStream();\n");
            for c in manual_multipart(parts, BOUNDARY) {
                match c {
                    Chunk::Text(t) => setup.push_str(&format!(
                        "        body.write({}.getBytes(StandardCharsets.UTF_8));\n",
                        dq(&t)
                    )),
                    Chunk::File(path) => setup.push_str(&format!(
                        "        body.write(Files.readAllBytes(Path.of({})));\n",
                        dq(&path)
                    )),
                }
            }
            setup.push('\n');
            extra_header = format!(
                "            .header(\"Content-Type\", \"multipart/form-data; boundary={BOUNDARY}\")\n"
            );
            "HttpRequest.BodyPublishers.ofByteArray(body.toByteArray())".into()
        }
        _ => "HttpRequest.BodyPublishers.noBody()".into(),
    };
    imports.sort_unstable();
    imports.dedup();
    let multipart = matches!(p.body, Some(PreparedBody::Multipart(_)));
    let mut headers = String::new();
    for (k, v) in &p.headers {
        if multipart && k.eq_ignore_ascii_case("content-type") {
            continue;
        }
        headers.push_str(&format!("            .header({}, {})\n", dq(k), dq(v)));
    }
    format!(
        "{}\n\npublic class Main {{\n    public static void main(String[] args) throws Exception {{\n        HttpClient client = HttpClient.newBuilder()\n            .followRedirects(HttpClient.Redirect.NORMAL)\n            .build();\n\n{setup}        HttpRequest request = HttpRequest.newBuilder()\n            .uri(URI.create({}))\n{headers}{extra_header}            .method({}, {publisher})\n            .build();\n\n        HttpResponse<String> response = client.send(request, HttpResponse.BodyHandlers.ofString());\n        System.out.println(response.statusCode());\n        System.out.println(response.body());\n    }}\n}}\n",
        imports
            .iter()
            .map(|i| format!("import {i};"))
            .collect::<Vec<_>>()
            .join("\n"),
        dq(&p.url),
        dq(&p.method),
    )
}

fn csharp(p: &Prepared) -> String {
    let mut out = format!(
        "using System.Net.Http;\nusing System.Net.Http.Headers;\n\nvar client = new HttpClient();\nvar request = new HttpRequestMessage(new HttpMethod({}), {});\n",
        dq(&p.method),
        dq(&p.url)
    );
    for (k, v) in headers_without_ct(p) {
        out.push_str(&format!(
            "request.Headers.TryAddWithoutValidation({}, {});\n",
            dq(k),
            dq(v)
        ));
    }
    let ct = content_type(p);
    match &p.body {
        Some(PreparedBody::Raw(raw)) if !raw.is_empty() => {
            out.push_str(&format!(
                "request.Content = new StringContent({}, null, {});\n",
                dq(raw),
                dq(ct.unwrap_or("text/plain"))
            ));
        }
        Some(PreparedBody::UrlEncoded(f)) => {
            let items: Vec<String> = f
                .iter()
                .map(|(k, v)| format!("    new KeyValuePair<string, string>({}, {})", dq(k), dq(v)))
                .collect();
            out.push_str(&format!(
                "request.Content = new FormUrlEncodedContent(new[]\n{{\n{}\n}});\n",
                items.join(",\n")
            ));
        }
        Some(PreparedBody::Multipart(parts)) => {
            out.push_str("var content = new MultipartFormDataContent();\n");
            for f in parts {
                if f.file {
                    out.push_str(&format!(
                        "content.Add(new StreamContent(File.OpenRead({})), {}, {});\n",
                        dq(&f.value),
                        dq(&f.name),
                        dq(file_name(&f.value))
                    ));
                } else {
                    out.push_str(&format!(
                        "content.Add(new StringContent({}), {});\n",
                        dq(&f.value),
                        dq(&f.name)
                    ));
                }
            }
            out.push_str("request.Content = content;\n");
        }
        Some(PreparedBody::File(path)) => {
            out.push_str(&format!(
                "request.Content = new ByteArrayContent(File.ReadAllBytes({}));\nrequest.Content.Headers.ContentType = new MediaTypeHeaderValue({});\n",
                dq(path),
                dq(ct.unwrap_or("application/octet-stream"))
            ));
        }
        _ => {}
    }
    out.push_str("\nvar response = await client.SendAsync(request);\nConsole.WriteLine((int)response.StatusCode);\nConsole.WriteLine(await response.Content.ReadAsStringAsync());\n");
    out
}

fn swift(p: &Prepared) -> String {
    let mut out = format!(
        "import Foundation\n\nvar request = URLRequest(url: URL(string: {})!, timeoutInterval: 30)\nrequest.httpMethod = {}\n",
        dq_braced(&p.url),
        dq_braced(&p.method)
    );
    let multipart = matches!(p.body, Some(PreparedBody::Multipart(_)));
    for (k, v) in &p.headers {
        if multipart && k.eq_ignore_ascii_case("content-type") {
            continue;
        }
        out.push_str(&format!(
            "request.addValue({}, forHTTPHeaderField: {})\n",
            dq_braced(v),
            dq_braced(k)
        ));
    }
    match &p.body {
        Some(PreparedBody::Raw(raw)) if !raw.is_empty() => out.push_str(&format!(
            "request.httpBody = {}.data(using: .utf8)\n",
            dq_braced(raw)
        )),
        Some(PreparedBody::UrlEncoded(f)) => out.push_str(&format!(
            "request.httpBody = {}.data(using: .utf8)\n",
            dq_braced(&http::encode_urlencoded(f))
        )),
        Some(PreparedBody::File(path)) => out.push_str(&format!(
            "request.httpBody = try Data(contentsOf: URL(fileURLWithPath: {}))\n",
            dq_braced(path)
        )),
        Some(PreparedBody::Multipart(parts)) => {
            out.push_str("\nvar body = Data()\n");
            for c in manual_multipart(parts, BOUNDARY) {
                match c {
                    Chunk::Text(t) => out.push_str(&format!(
                        "body.append({}.data(using: .utf8)!)\n",
                        dq_braced(&t)
                    )),
                    Chunk::File(path) => out.push_str(&format!(
                        "body.append(try Data(contentsOf: URL(fileURLWithPath: {})))\n",
                        dq_braced(&path)
                    )),
                }
            }
            out.push_str(&format!(
                "request.setValue(\"multipart/form-data; boundary={BOUNDARY}\", forHTTPHeaderField: \"Content-Type\")\nrequest.httpBody = body\n"
            ));
        }
        _ => {}
    }
    out.push_str("\nlet (data, response) = try await URLSession.shared.data(for: request)\nif let http = response as? HTTPURLResponse {\n    print(http.statusCode)\n}\nprint(String(decoding: data, as: UTF8.self))\n");
    out
}

fn kotlin(p: &Prepared) -> String {
    let mut imports = vec!["okhttp3.OkHttpClient", "okhttp3.Request"];
    let mut setup = String::new();
    let ct = content_type(p).unwrap_or("text/plain");
    let body = match &p.body {
        Some(PreparedBody::Raw(raw)) if !raw.is_empty() => {
            imports.extend([
                "okhttp3.MediaType.Companion.toMediaType",
                "okhttp3.RequestBody.Companion.toRequestBody",
            ]);
            setup.push_str(&format!(
                "val body = {}.toRequestBody({}.toMediaType())\n",
                dq_dollar(raw),
                dq_dollar(ct)
            ));
            "body"
        }
        Some(PreparedBody::UrlEncoded(f)) => {
            imports.push("okhttp3.FormBody");
            setup.push_str("val body = FormBody.Builder()\n");
            for (k, v) in f {
                setup.push_str(&format!("    .add({}, {})\n", dq_dollar(k), dq_dollar(v)));
            }
            setup.push_str("    .build()\n");
            "body"
        }
        Some(PreparedBody::Multipart(parts)) => {
            imports.push("okhttp3.MultipartBody");
            setup.push_str("val body = MultipartBody.Builder().setType(MultipartBody.FORM)\n");
            for f in parts {
                if f.file {
                    imports.extend([
                        "java.io.File",
                        "okhttp3.MediaType.Companion.toMediaType",
                        "okhttp3.RequestBody.Companion.asRequestBody",
                    ]);
                    setup.push_str(&format!(
                        "    .addFormDataPart({}, {}, File({}).asRequestBody({}.toMediaType()))\n",
                        dq_dollar(&f.name),
                        dq_dollar(file_name(&f.value)),
                        dq_dollar(&f.value),
                        dq_dollar(mime_for(&f.value))
                    ));
                } else {
                    setup.push_str(&format!(
                        "    .addFormDataPart({}, {})\n",
                        dq_dollar(&f.name),
                        dq_dollar(&f.value)
                    ));
                }
            }
            setup.push_str("    .build()\n");
            "body"
        }
        Some(PreparedBody::File(path)) => {
            imports.extend([
                "java.io.File",
                "okhttp3.MediaType.Companion.toMediaType",
                "okhttp3.RequestBody.Companion.asRequestBody",
            ]);
            setup.push_str(&format!(
                "val body = File({}).asRequestBody({}.toMediaType())\n",
                dq_dollar(path),
                dq_dollar(ct)
            ));
            "body"
        }
        _ if matches!(p.method.as_str(), "POST" | "PUT" | "PATCH") => {
            imports.push("okhttp3.RequestBody.Companion.toRequestBody");
            setup.push_str("val body = \"\".toRequestBody(null)\n");
            "body"
        }
        _ => "null",
    };
    imports.sort_unstable();
    imports.dedup();
    let multipart = matches!(p.body, Some(PreparedBody::Multipart(_)));
    let mut out = format!(
        "{}\n\nval client = OkHttpClient()\n{setup}",
        imports
            .iter()
            .map(|i| format!("import {i}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    out.push_str(&format!(
        "val request = Request.Builder()\n    .url({})\n    .method({}, {body})\n",
        dq_dollar(&p.url),
        dq_dollar(&p.method)
    ));
    for (k, v) in &p.headers {
        if multipart && k.eq_ignore_ascii_case("content-type") {
            continue;
        }
        out.push_str(&format!(
            "    .addHeader({}, {})\n",
            dq_dollar(k),
            dq_dollar(v)
        ));
    }
    out.push_str("    .build()\n\nclient.newCall(request).execute().use { response ->\n    println(response.code)\n    println(response.body?.string())\n}\n");
    out
}

fn dart(p: &Prepared) -> String {
    let mut imports = vec!["import 'package:http/http.dart' as http;"];
    let multipart = matches!(p.body, Some(PreparedBody::Multipart(_)));
    let headers: Vec<String> = p
        .headers
        .iter()
        .filter(|(k, _)| !(multipart && k.eq_ignore_ascii_case("content-type")))
        .map(|(k, v)| format!("    {}: {}", dq_dollar(k), dq_dollar(v)))
        .collect();
    let mut body = String::new();
    let kind = if multipart {
        "MultipartRequest"
    } else {
        "Request"
    };
    match &p.body {
        Some(PreparedBody::Raw(raw)) if !raw.is_empty() => {
            body.push_str(&format!("  request.body = {};\n", dq_dollar(raw)))
        }
        Some(PreparedBody::UrlEncoded(f)) => {
            let items: Vec<String> = f
                .iter()
                .map(|(k, v)| format!("    {}: {}", dq_dollar(k), dq_dollar(v)))
                .collect();
            body.push_str(&format!(
                "  request.bodyFields = {{\n{}\n  }};\n",
                items.join(",\n")
            ));
        }
        Some(PreparedBody::Multipart(parts)) => {
            let fields: Vec<String> = parts
                .iter()
                .filter(|f| !f.file)
                .map(|f| format!("    {}: {}", dq_dollar(&f.name), dq_dollar(&f.value)))
                .collect();
            if !fields.is_empty() {
                body.push_str(&format!(
                    "  request.fields.addAll({{\n{}\n  }});\n",
                    fields.join(",\n")
                ));
            }
            for f in parts.iter().filter(|f| f.file) {
                body.push_str(&format!(
                    "  request.files.add(await http.MultipartFile.fromPath({}, {}));\n",
                    dq_dollar(&f.name),
                    dq_dollar(&f.value)
                ));
            }
        }
        Some(PreparedBody::File(path)) => {
            imports.insert(0, "import 'dart:io';");
            body.push_str(&format!(
                "  request.bodyBytes = await File({}).readAsBytes();\n",
                dq_dollar(path)
            ));
        }
        _ => {}
    }
    let mut out = format!("{}\n\nvoid main() async {{\n", imports.join("\n"));
    if !headers.is_empty() {
        out.push_str(&format!(
            "  var headers = {{\n{}\n  }};\n",
            headers.join(",\n")
        ));
    }
    out.push_str(&format!(
        "  var request = http.{kind}({}, Uri.parse({}));\n",
        dq_dollar(&p.method),
        dq_dollar(&p.url)
    ));
    out.push_str(&body);
    if !headers.is_empty() {
        out.push_str("  request.headers.addAll(headers);\n");
    }
    out.push_str("\n  http.StreamedResponse response = await request.send();\n  print(response.statusCode);\n  print(await response.stream.bytesToString());\n}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(body: Option<PreparedBody>) -> Prepared {
        Prepared {
            method: "POST".into(),
            url: "https://api.example.com/users?x=1".into(),
            headers: vec![
                ("Content-Type".into(), "application/json".into()),
                ("X-Key".into(), "it's $5".into()),
            ],
            body,
            flow: AuthFlow::None,
        }
    }

    #[test]
    fn auth_flows_in_snippets() {
        let mut p = sample(None);
        p.flow = AuthFlow::Digest { username: "u".into(), password: "p".into() };
        assert!(curl(&p).contains("--digest --user 'u:p'"));
        assert!(generate(Lang::Python, &p).starts_with("# Auth: HTTP Digest"));
        assert!(generate(Lang::Go, &p).starts_with("// Auth: HTTP Digest"));
        p.flow = AuthFlow::Aws { access_key: "AK".into(), secret_key: "SK".into(), region: "eu-west-1".into(), service: "s3".into(), session_token: String::new() };
        assert!(curl(&p).contains("--aws-sigv4 'aws:amz:eu-west-1:s3'"));
    }

    #[test]
    fn every_language_renders() {
        let bodies = [
            None,
            Some(PreparedBody::Raw("{\"name\":\"Lina\",\"ok\":true}".into())),
            Some(PreparedBody::UrlEncoded(vec![("a".into(), "1 2".into())])),
            Some(PreparedBody::Multipart(vec![
                Part {
                    name: "title".into(),
                    value: "x".into(),
                    file: false,
                },
                Part {
                    name: "photo".into(),
                    value: "/tmp/a.png".into(),
                    file: true,
                },
            ])),
            Some(PreparedBody::File("/tmp/data.bin".into())),
        ];
        for body in bodies {
            let p = sample(body);
            for lang in Lang::ALL {
                let code = generate(lang, &p);
                assert!(
                    code.contains("api.example.com"),
                    "{lang:?} is missing the URL"
                );
            }
        }
    }

    #[test]
    fn quoting() {
        assert_eq!(sh("it's"), "'it'\\''s'");
        assert_eq!(dq_braced("a\u{1}b"), "\"a\\u{0001}b\"");
        assert_eq!(dq_dollar("$x"), "\"\\$x\"");
        let py = python(&sample(Some(PreparedBody::Raw(
            "{\"ok\":true,\"n\":null}".into(),
        ))));
        assert!(py.contains("\"ok\": True"));
        assert!(py.contains("\"n\": None"));
    }
}
