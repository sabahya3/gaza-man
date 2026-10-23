//! No-code tests and variable captures: checking a response, and saving parts of it into
//! variables. They also export as real Postman `pm.test` scripts.

use serde_json::Value;

use crate::collection::{Assertion, Capture, Op, Scope, Source};
use crate::http::Outcome;

#[derive(Clone, Debug)]
pub struct TestResult {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

/// A path like `data.items[0].id`, `$.user["first name"]` or `items.length`.
fn tokens(path: &str) -> Vec<String> {
    let path = path.trim();
    let path = path.strip_prefix('$').unwrap_or(path);
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '.' => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            '[' => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                let mut inner = String::new();
                for c in chars.by_ref() {
                    if c == ']' {
                        break;
                    }
                    inner.push(c);
                }
                out.push(
                    inner
                        .trim()
                        .trim_matches(|c| c == '"' || c == '\'')
                        .to_string(),
                );
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Looks up a path in a JSON value. An empty path is the whole value.
pub fn json_path(root: &Value, path: &str) -> Option<Value> {
    let mut cur = root.clone();
    for t in tokens(path) {
        cur = match &cur {
            Value::Object(o) => o.get(&t).cloned()?,
            Value::Array(a) if t == "length" => Value::from(a.len()),
            Value::Array(a) => {
                let i: i64 = t.parse().ok()?;
                let i = if i < 0 { a.len() as i64 + i } else { i };
                a.get(usize::try_from(i).ok()?).cloned()?
            }
            Value::String(s) if t == "length" => Value::from(s.chars().count()),
            _ => return None,
        };
    }
    Some(cur)
}

/// The value a source points at, if the response has it.
pub fn extract(outcome: &Outcome, source: Source, path: &str) -> Option<Value> {
    let Outcome::Response {
        status,
        body,
        time_ms,
        ..
    } = outcome
    else {
        return None;
    };
    match source {
        Source::Status => Some(Value::from(*status)),
        Source::Time => Some(Value::from(*time_ms as u64)),
        Source::Body => Some(Value::String(body.clone())),
        Source::Header => outcome
            .header(path.trim())
            .map(|v| Value::String(v.to_string())),
        Source::Json => serde_json::from_str::<Value>(body)
            .ok()
            .and_then(|v| json_path(&v, path)),
    }
}

/// A value as a variable would hold it: strings without quotes, everything else as JSON.
pub fn plain(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn equals(actual: &Value, expected: &str) -> bool {
    if let Ok(e) = serde_json::from_str::<Value>(expected) {
        if &e == actual {
            return true;
        }
        if let (Some(a), Some(b)) = (number(actual), number(&e)) {
            return a == b;
        }
    }
    plain(actual) == expected
}

fn contains(actual: &Value, expected: &str) -> bool {
    match actual {
        Value::Array(items) => items.iter().any(|i| equals(i, expected)),
        Value::Object(o) => o.contains_key(expected),
        other => plain(other).contains(expected),
    }
}

pub fn describe(a: &Assertion) -> String {
    let subject = match a.source {
        Source::Json => format!(
            "JSON {}",
            if a.path.trim().is_empty() {
                "body"
            } else {
                a.path.trim()
            }
        ),
        Source::Header => format!("Header {}", a.path.trim()),
        s => s.label().to_string(),
    };
    if a.op.needs_value() {
        format!("{subject} {} {}", a.op.label(), a.expected)
    } else {
        format!("{subject} {}", a.op.label())
    }
}

fn short(v: &Value) -> String {
    let s = plain(v);
    if s.chars().count() > 80 {
        format!("{}…", s.chars().take(80).collect::<String>())
    } else {
        s
    }
}

pub fn check(a: &Assertion, outcome: &Outcome) -> TestResult {
    let name = describe(a);
    if let Outcome::Error { message, .. } = outcome {
        return TestResult {
            name,
            passed: false,
            detail: format!("No response: {message}"),
        };
    }
    let actual = extract(outcome, a.source, &a.path);
    let expected = a.expected.trim();
    let passed = match (a.op, &actual) {
        (Op::Exists, v) => v.as_ref().is_some_and(|v| !v.is_null()),
        (Op::NotExists, v) => v.as_ref().is_none_or(Value::is_null),
        (_, None) => false,
        (Op::Equals, Some(v)) => equals(v, expected),
        (Op::NotEquals, Some(v)) => !equals(v, expected),
        (Op::GreaterThan, Some(v)) => {
            matches!((number(v), expected.parse::<f64>()), (Some(a), Ok(b)) if a > b)
        }
        (Op::LessThan, Some(v)) => {
            matches!((number(v), expected.parse::<f64>()), (Some(a), Ok(b)) if a < b)
        }
        (Op::Contains, Some(v)) => contains(v, expected),
        (Op::NotContains, Some(v)) => !contains(v, expected),
    };
    let detail = match &actual {
        Some(v) => format!("got {}", short(v)),
        None => "not found in the response".into(),
    };
    TestResult {
        name,
        passed,
        detail,
    }
}

pub fn run(assertions: &[Assertion], outcome: &Outcome) -> Vec<TestResult> {
    assertions
        .iter()
        .filter(|a| a.enabled)
        .map(|a| check(a, outcome))
        .collect()
}

/// The `(scope, variable, value)` updates the captures produce for this response.
pub fn captures(list: &[Capture], outcome: &Outcome) -> Vec<(Scope, String, String)> {
    list.iter()
        .filter(|c| c.enabled && !c.var.trim().is_empty())
        .filter_map(|c| {
            extract(outcome, c.source, &c.path)
                .map(|v| (c.scope, c.var.trim().to_string(), plain(&v)))
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Postman script export
// ---------------------------------------------------------------------------------------------

fn js_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// `data.items[0]["first name"]` as a JavaScript accessor chain.
fn js_access(path: &str) -> String {
    tokens(path)
        .iter()
        .map(|t| {
            if t.parse::<i64>().is_ok() {
                format!("[{t}]")
            } else if t
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
                && t.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
            {
                format!(".{t}")
            } else {
                format!("[{}]", js_string(t))
            }
        })
        .collect()
}

fn js_subject(source: Source, path: &str) -> String {
    match source {
        Source::Status => "pm.response.code".into(),
        Source::Time => "pm.response.responseTime".into(),
        Source::Body => "pm.response.text()".into(),
        Source::Header => format!("pm.response.headers.get({})", js_string(path.trim())),
        Source::Json => format!("pm.response.json(){}", js_access(path)),
    }
}

/// A JavaScript literal for the expected value: JSON when it parses, otherwise a string.
fn js_literal(expected: &str) -> String {
    match serde_json::from_str::<Value>(expected.trim()) {
        Ok(v) => v.to_string(),
        Err(_) => js_string(expected),
    }
}

/// Postman test script equivalent to these assertions and captures.
pub fn postman_script(assertions: &[Assertion], capture_list: &[Capture]) -> String {
    let mut out = String::new();
    for a in assertions.iter().filter(|a| a.enabled) {
        let subject = js_subject(a.source, &a.path);
        let e = js_literal(&a.expected);
        let check = match a.op {
            Op::Equals => format!("pm.expect({subject}).to.eql({e})"),
            Op::NotEquals => format!("pm.expect({subject}).to.not.eql({e})"),
            Op::GreaterThan => format!("pm.expect(Number({subject})).to.be.above({e})"),
            Op::LessThan => format!("pm.expect(Number({subject})).to.be.below({e})"),
            Op::Contains => format!("pm.expect({subject}).to.include({e})"),
            Op::NotContains => format!("pm.expect({subject}).to.not.include({e})"),
            Op::Exists => format!("pm.expect({subject}).to.exist"),
            Op::NotExists => format!("pm.expect({subject}).to.not.exist"),
        };
        out.push_str(&format!(
            "pm.test({}, function () {{\n    {check};\n}});\n",
            js_string(&describe(a))
        ));
    }
    for c in capture_list
        .iter()
        .filter(|c| c.enabled && !c.var.trim().is_empty())
    {
        let target = match c.scope {
            Scope::Environment => "pm.environment",
            Scope::Collection => "pm.collectionVariables",
            Scope::Global => "pm.globals",
        };
        out.push_str(&format!(
            "{target}.set({}, {});\n",
            js_string(c.var.trim()),
            js_subject(c.source, &c.path)
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn response(body: &str) -> Outcome {
        Outcome::Response {
            status: 201,
            status_text: "Created".into(),
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: body.into(),
            bytes: body.as_bytes().to_vec(),
            time_ms: 120,
        }
    }

    #[test]
    fn paths() {
        let v = json!({ "data": { "items": [{ "id": 7 }, { "id": 9 }], "first name": "Lina" } });
        assert_eq!(json_path(&v, "data.items[1].id"), Some(json!(9)));
        assert_eq!(json_path(&v, "$.data.items.length"), Some(json!(2)));
        assert_eq!(json_path(&v, "data[\"first name\"]"), Some(json!("Lina")));
        assert_eq!(json_path(&v, "data.items[-1].id"), Some(json!(9)));
        assert_eq!(json_path(&v, "data.missing"), None);
    }

    #[test]
    fn assertions() {
        let r = response(r#"{"token":"abc","count":3,"tags":["a","b"]}"#);
        let a = |source, path: &str, op, expected: &str| Assertion {
            enabled: true,
            source,
            path: path.into(),
            op,
            expected: expected.into(),
        };
        assert!(check(&a(Source::Status, "", Op::Equals, "201"), &r).passed);
        assert!(check(&a(Source::Time, "", Op::LessThan, "500"), &r).passed);
        assert!(check(&a(Source::Json, "count", Op::GreaterThan, "2"), &r).passed);
        assert!(check(&a(Source::Json, "token", Op::Equals, "abc"), &r).passed);
        assert!(check(&a(Source::Json, "tags", Op::Contains, "b"), &r).passed);
        assert!(check(&a(Source::Json, "nope", Op::NotExists, ""), &r).passed);
        assert!(check(&a(Source::Header, "content-type", Op::Contains, "json"), &r).passed);
        assert!(!check(&a(Source::Json, "count", Op::Equals, "4"), &r).passed);
    }

    #[test]
    fn captures_and_script() {
        let r = response(r#"{"user":{"token":"xyz"}}"#);
        let c = vec![Capture {
            var: "token".into(),
            path: "user.token".into(),
            ..Default::default()
        }];
        assert_eq!(
            captures(&c, &r),
            vec![(Scope::Environment, "token".to_string(), "xyz".to_string())]
        );
        let script = postman_script(&[Assertion::default()], &c);
        assert!(script.contains("pm.expect(pm.response.code).to.eql(200)"));
        assert!(script.contains("pm.environment.set(\"token\", pm.response.json().user.token);"));
    }
}
