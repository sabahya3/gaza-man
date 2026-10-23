//! A small YAML reader, enough for OpenAPI / Swagger files: block mappings and sequences,
//! flow `[..]` / `{..}`, quoted and block (`|`, `>`) strings, comments, anchors and merges.
//! Produces `serde_json::Value` so the OpenAPI importer can stay JSON-only.

use std::collections::HashMap;

use serde_json::{Map, Number, Value};

struct Parser {
    lines: Vec<String>,
    i: usize,
    anchors: HashMap<String, Value>,
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

fn is_blank(line: &str) -> bool {
    let t = line.trim();
    t.is_empty() || t.starts_with('#') || t == "---" || t == "..."
}

/// Cuts a ` # comment` that's outside quotes.
fn strip_comment(s: &str) -> &str {
    let (mut single, mut double) = (false, false);
    let bytes = s.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'\'' if !double => single = !single,
            b'"' if !single && (i == 0 || bytes[i - 1] != b'\\') => double = !double,
            b'#' if !single && !double && (i == 0 || bytes[i - 1] == b' ' || bytes[i - 1] == b'\t') => return s[..i].trim_end(),
            _ => {}
        }
    }
    s.trim_end()
}

/// The byte index of the `:` that ends a mapping key, outside quotes and brackets.
fn key_colon(s: &str) -> Option<usize> {
    let (mut single, mut double, mut depth) = (false, false, 0i32);
    let bytes = s.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'\'' if !double => single = !single,
            b'"' if !single => double = !double,
            b'[' | b'{' if !single && !double => depth += 1,
            b']' | b'}' if !single && !double => depth -= 1,
            b':' if !single && !double && depth == 0 && (i + 1 == bytes.len() || bytes[i + 1] == b' ') => return Some(i),
            _ => {}
        }
    }
    None
}

fn unquote_key(k: &str) -> String {
    let k = k.trim();
    if (k.starts_with('"') && k.ends_with('"') || k.starts_with('\'') && k.ends_with('\'')) && k.len() >= 2 {
        match scalar(k) {
            Value::String(s) => s,
            v => v.to_string(),
        }
    } else {
        k.to_string()
    }
}

fn double_quoted(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some('/') => out.push('/'),
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    out.push(ch);
                }
            }
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// A plain or quoted scalar.
fn scalar(s: &str) -> Value {
    let s = s.trim();
    if let Some(inner) = s.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
        return Value::String(double_quoted(inner));
    }
    if let Some(inner) = s.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')) {
        return Value::String(inner.replace("''", "'"));
    }
    match s {
        "" | "~" | "null" | "Null" | "NULL" => return Value::Null,
        "true" | "True" | "TRUE" => return Value::Bool(true),
        "false" | "False" | "FALSE" => return Value::Bool(false),
        _ => {}
    }
    let looks_numeric = s.chars().next().is_some_and(|c| c.is_ascii_digit() || c == '-' || c == '+' || c == '.');
    if looks_numeric {
        if let Ok(i) = s.parse::<i64>() {
            return Value::Number(i.into());
        }
        if let Some(n) = s.parse::<f64>().ok().filter(|f| f.is_finite()).and_then(Number::from_f64) {
            return Value::Number(n);
        }
    }
    Value::String(s.to_string())
}

/// Parses flow collections like `[a, "b", {c: 1}]`.
fn flow(chars: &[char], pos: &mut usize) -> Value {
    let skip_ws = |pos: &mut usize| {
        while *pos < chars.len() && chars[*pos].is_whitespace() {
            *pos += 1;
        }
    };
    let read_item = |pos: &mut usize| -> String {
        let mut out = String::new();
        let mut quote: Option<char> = None;
        while *pos < chars.len() {
            let c = chars[*pos];
            match quote {
                Some(q) if c == q => {
                    out.push(c);
                    quote = None;
                }
                Some(_) => out.push(c),
                None if c == '"' || c == '\'' => {
                    out.push(c);
                    quote = Some(c);
                }
                None if matches!(c, ',' | ']' | '}') => break,
                None if c == ':' && chars.get(*pos + 1).is_none_or(|n| n.is_whitespace() || matches!(n, ',' | '}')) => break,
                None => out.push(c),
            }
            *pos += 1;
        }
        out.trim().to_string()
    };
    skip_ws(pos);
    match chars.get(*pos) {
        Some('[') => {
            *pos += 1;
            let mut list = Vec::new();
            loop {
                skip_ws(pos);
                match chars.get(*pos) {
                    None => break,
                    Some(']') => {
                        *pos += 1;
                        break;
                    }
                    Some(',') => *pos += 1,
                    _ => list.push(flow(chars, pos)),
                }
            }
            Value::Array(list)
        }
        Some('{') => {
            *pos += 1;
            let mut map = Map::new();
            loop {
                skip_ws(pos);
                match chars.get(*pos) {
                    None => break,
                    Some('}') => {
                        *pos += 1;
                        break;
                    }
                    Some(',') => *pos += 1,
                    _ => {
                        let key = unquote_key(&read_item(pos));
                        skip_ws(pos);
                        let value = if chars.get(*pos) == Some(&':') {
                            *pos += 1;
                            skip_ws(pos);
                            if matches!(chars.get(*pos), Some(',') | Some('}')) { Value::Null } else { flow(chars, pos) }
                        } else {
                            Value::Null
                        };
                        map.insert(key, value);
                    }
                }
            }
            Value::Object(map)
        }
        _ => scalar(&read_item(pos)),
    }
}

fn balanced(s: &str) -> bool {
    let (mut depth, mut quote) = (0i32, None);
    for c in s.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '[' || c == '{' => depth += 1,
            None if c == ']' || c == '}' => depth -= 1,
            None => {}
        }
    }
    depth <= 0 && quote.is_none()
}

impl Parser {
    fn next_content(&mut self) -> Option<usize> {
        while self.i < self.lines.len() && is_blank(&self.lines[self.i]) {
            self.i += 1;
        }
        (self.i < self.lines.len()).then(|| indent_of(&self.lines[self.i]))
    }

    fn block(&mut self, min_indent: usize) -> Value {
        let Some(n) = self.next_content() else { return Value::Null };
        if n < min_indent {
            return Value::Null;
        }
        let line = self.lines[self.i].trim_start().to_string();
        if line == "-" || line.starts_with("- ") {
            self.sequence(n)
        } else if key_colon(strip_comment(&line)).is_some() {
            self.mapping(n)
        } else {
            self.i += 1;
            self.inline(strip_comment(&line).to_string(), n)
        }
    }

    fn sequence(&mut self, n: usize) -> Value {
        let mut list = Vec::new();
        while let Some(ind) = self.next_content() {
            let line = self.lines[self.i].clone();
            let t = line.trim_start();
            if ind != n || !(t == "-" || t.starts_with("- ")) {
                break;
            }
            let rest = t[1..].trim_start();
            if rest.is_empty() {
                self.i += 1;
                list.push(self.block(n + 1));
            } else if key_colon(strip_comment(rest)).is_some() && !rest.starts_with('[') && !rest.starts_with('{') {
                // `- key: value` starts a mapping indented past the dash.
                let offset = n + (t.len() - rest.len());
                self.lines[self.i] = format!("{}{rest}", " ".repeat(offset));
                list.push(self.mapping(offset));
            } else {
                self.i += 1;
                list.push(self.value(rest.to_string(), n));
            }
        }
        Value::Array(list)
    }

    fn mapping(&mut self, n: usize) -> Value {
        let mut map = Map::new();
        while let Some(ind) = self.next_content() {
            if ind != n {
                break;
            }
            let line = self.lines[self.i].trim_start().to_string();
            if line.starts_with("- ") || line == "-" {
                break;
            }
            let content = strip_comment(&line).to_string();
            let Some(colon) = key_colon(&content) else { break };
            let key = unquote_key(&content[..colon]);
            let rest = content[colon + 1..].trim().to_string();
            self.i += 1;
            let value = if rest.is_empty() {
                match self.next_content() {
                    Some(c) if c > n => self.block(c),
                    // A list may sit at the same indent as its key.
                    Some(c) if c == n && self.lines[self.i].trim_start().starts_with('-') => self.sequence(n),
                    _ => Value::Null,
                }
            } else {
                // Block scalars keep their own lines (comments included), so use the raw text.
                let raw_rest = line[colon + 1..].trim().to_string();
                if raw_rest.starts_with('|') || raw_rest.starts_with('>') { self.value(raw_rest, n) } else { self.value(rest, n) }
            };
            if key == "<<" {
                // Merge key: `<<: *defaults`.
                let merge = |map: &mut Map<String, Value>, v: &Value| {
                    if let Value::Object(o) = v {
                        for (k, v) in o {
                            map.entry(k.clone()).or_insert_with(|| v.clone());
                        }
                    }
                };
                match &value {
                    Value::Array(items) => items.iter().for_each(|v| merge(&mut map, v)),
                    v => merge(&mut map, v),
                }
            } else {
                map.insert(key, value);
            }
        }
        Value::Object(map)
    }

    /// The value after `key:` or `- `, which may continue on the next lines.
    fn value(&mut self, rest: String, n: usize) -> Value {
        let rest = rest.trim().to_string();
        if let Some(anchor) = rest.strip_prefix('&') {
            let (name, after) = anchor.split_once(' ').unwrap_or((anchor, ""));
            let name = name.to_string();
            let v = if after.trim().is_empty() {
                match self.next_content() {
                    Some(c) if c > n => self.block(c),
                    Some(c) if c == n && self.lines[self.i].trim_start().starts_with('-') => self.sequence(n),
                    _ => Value::Null,
                }
            } else {
                self.value(after.to_string(), n)
            };
            self.anchors.insert(name, v.clone());
            return v;
        }
        if let Some(alias) = rest.strip_prefix('*') {
            return self.anchors.get(alias.trim()).cloned().unwrap_or(Value::Null);
        }
        if rest.starts_with('|') || rest.starts_with('>') {
            return self.block_scalar(&rest, n);
        }
        self.inline(rest, n)
    }

    fn inline(&mut self, mut rest: String, n: usize) -> Value {
        // Flow collections and quoted strings can span lines.
        let opens_flow = rest.starts_with('[') || rest.starts_with('{');
        let opens_quote = (rest.starts_with('"') || rest.starts_with('\'')) && !balanced(&rest);
        if opens_flow || opens_quote {
            while !balanced(&rest) && self.i < self.lines.len() {
                let next = self.lines[self.i].trim();
                rest.push(if opens_quote { ' ' } else { ' ' });
                rest.push_str(if opens_flow { strip_comment(next) } else { next });
                self.i += 1;
            }
            if opens_flow {
                let chars: Vec<char> = rest.chars().collect();
                return flow(&chars, &mut 0);
            }
            return scalar(&rest);
        }
        // A plain scalar continues on more-indented lines, folded with spaces.
        while self.i < self.lines.len() {
            let line = &self.lines[self.i];
            if is_blank(line) || indent_of(line) <= n {
                break;
            }
            let t = strip_comment(line.trim());
            if key_colon(t).is_some() || t.starts_with("- ") {
                break;
            }
            rest.push(' ');
            rest.push_str(t);
            self.i += 1;
        }
        scalar(&rest)
    }

    fn block_scalar(&mut self, header: &str, n: usize) -> Value {
        let folded = header.starts_with('>');
        let keep = header.contains('+');
        let strip = header.contains('-');
        let mut lines: Vec<String> = Vec::new();
        let mut block_indent = None;
        while self.i < self.lines.len() {
            let line = &self.lines[self.i];
            if line.trim().is_empty() {
                lines.push(String::new());
                self.i += 1;
                continue;
            }
            let ind = indent_of(line);
            if ind <= n {
                break;
            }
            let bi = *block_indent.get_or_insert(ind);
            if ind < bi {
                break;
            }
            lines.push(line[bi..].to_string());
            self.i += 1;
        }
        let trailing = lines.iter().rev().take_while(|l| l.is_empty()).count();
        lines.truncate(lines.len() - trailing);
        let mut text = if folded {
            let mut out = String::new();
            for (i, l) in lines.iter().enumerate() {
                if i > 0 {
                    out.push(if l.is_empty() || lines[i - 1].is_empty() || l.starts_with(' ') { '\n' } else { ' ' });
                }
                out.push_str(l);
            }
            out
        } else {
            lines.join("\n")
        };
        if !strip {
            text.push('\n');
            if keep {
                text.push_str(&"\n".repeat(trailing));
            }
        }
        Value::String(text)
    }
}

pub fn parse(text: &str) -> Result<Value, String> {
    let lines: Vec<String> = text.replace('\t', "  ").replace("\r\n", "\n").lines().map(str::to_string).collect();
    let mut p = Parser { lines, i: 0, anchors: HashMap::new() };
    let v = p.block(0);
    if p.next_content().is_some() {
        let line = p.i + 1;
        return Err(format!("Couldn't read the YAML near line {line}: {}", p.lines[p.i].trim()));
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn openapi_document() {
        let doc = r#"
openapi: 3.0.0   # version
info:
  title: "Pets: the API"
  description: |
    Line one
    Line two
  version: 1.0.0
servers:
  - url: https://pets.io/v1
tags: [pets, 'store']
paths:
  /pets/{petId}:
    get:
      summary: Get a pet
      parameters:
      - name: petId
        in: path
        required: true
        schema: {type: integer, example: 3}
      responses:
        '200':
          description: >
            A pet,
            folded.
defaults: &defaults
  limit: 10
  sort: name
listing:
  <<: *defaults
  sort: id
empty:
quoted: "a \"b\" é"
"#;
        let v = parse(doc).unwrap();
        assert_eq!(v["openapi"], "3.0.0");
        assert_eq!(v["info"]["title"], "Pets: the API");
        assert_eq!(v["info"]["description"], "Line one\nLine two\n");
        assert_eq!(v["servers"][0]["url"], "https://pets.io/v1");
        assert_eq!(v["tags"], json!(["pets", "store"]));
        let get = &v["paths"]["/pets/{petId}"]["get"];
        assert_eq!(get["parameters"][0]["in"], "path");
        assert_eq!(get["parameters"][0]["required"], true);
        assert_eq!(get["parameters"][0]["schema"]["example"], 3);
        assert_eq!(get["responses"]["200"]["description"], "A pet, folded.\n");
        assert_eq!(v["listing"], json!({ "limit": 10, "sort": "id" }));
        assert_eq!(v["empty"], Value::Null);
        assert_eq!(v["quoted"], "a \"b\" é");
    }

    #[test]
    fn imports_through_openapi() {
        let doc = "openapi: 3.0.1\ninfo:\n  title: Mini\nservers:\n- url: http://localhost:3000\npaths:\n  /users:\n    post:\n      summary: Create user\n      requestBody:\n        content:\n          application/json:\n            schema:\n              type: object\n              properties:\n                name: {type: string}\n";
        let c = crate::import::parse_openapi(&parse(doc).unwrap()).unwrap();
        assert_eq!(c.name, "Mini");
        assert_eq!(c.requests()[0].method, "POST");
        assert!(c.requests()[0].raw.contains("\"name\""));
    }
}
