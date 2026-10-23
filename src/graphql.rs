//! GraphQL schema support: fetching it with an introspection query, and suggesting fields
//! while a query is typed, based on where the cursor is.

use std::collections::HashMap;

use serde_json::Value;

/// The standard introspection query, trimmed to what the editor uses.
pub const INTROSPECTION_QUERY: &str = "query IntrospectionQuery {
  __schema {
    queryType { name }
    mutationType { name }
    subscriptionType { name }
    types {
      kind name description
      fields(includeDeprecated: true) { name description args { name type { ...TypeRef } } type { ...TypeRef } }
      inputFields { name type { ...TypeRef } }
      enumValues(includeDeprecated: true) { name }
    }
  }
}
fragment TypeRef on __Type { kind name ofType { kind name ofType { kind name ofType { kind name } } } }";

#[derive(Clone, Debug, Default)]
pub struct Field {
    pub name: String,
    /// As written in GraphQL: `[User!]!`.
    pub type_label: String,
    /// The named type underneath lists and non-nulls: `User`.
    pub base: String,
    pub args: Vec<(String, String)>,
    pub description: String,
}

#[derive(Clone, Debug, Default)]
pub struct TypeInfo {
    pub fields: Vec<Field>,
}

#[derive(Clone, Debug, Default)]
pub struct Schema {
    pub query: Option<String>,
    pub mutation: Option<String>,
    pub subscription: Option<String>,
    pub types: HashMap<String, TypeInfo>,
}

impl Schema {
    pub fn fields(&self, type_name: &str) -> &[Field] {
        self.types.get(type_name).map(|t| t.fields.as_slice()).unwrap_or(&[])
    }

    fn has_fields(&self, type_name: &str) -> bool {
        !self.fields(type_name).is_empty()
    }

    /// Types written by the user (not the `__Schema` / `__Type` built-ins).
    pub fn user_type_count(&self) -> usize {
        self.types.keys().filter(|k| !k.starts_with("__")).count()
    }
}

fn type_ref(t: &Value) -> (String, String) {
    match t["kind"].as_str() {
        Some("NON_NULL") => {
            let (label, base) = type_ref(&t["ofType"]);
            (format!("{label}!"), base)
        }
        Some("LIST") => {
            let (label, base) = type_ref(&t["ofType"]);
            (format!("[{label}]"), base)
        }
        _ => {
            let name = t["name"].as_str().unwrap_or("?").to_string();
            (name.clone(), name)
        }
    }
}

/// Reads the result of [`INTROSPECTION_QUERY`].
pub fn parse_introspection(response: &Value) -> Result<Schema, String> {
    let schema = &response["data"]["__schema"];
    if !schema.is_object() {
        let why = response["errors"][0]["message"].as_str().unwrap_or("the response has no data.__schema");
        return Err(format!("The server didn't return a schema: {why}"));
    }
    let name = |v: &Value| v["name"].as_str().map(str::to_string);
    let mut types = HashMap::new();
    for t in schema["types"].as_array().into_iter().flatten() {
        let Some(type_name) = name(t) else { continue };
        let fields = t["fields"]
            .as_array()
            .or_else(|| t["inputFields"].as_array())
            .into_iter()
            .flatten()
            .map(|f| {
                let (type_label, base) = type_ref(&f["type"]);
                Field {
                    name: f["name"].as_str().unwrap_or("").to_string(),
                    type_label,
                    base,
                    args: f["args"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|a| (a["name"].as_str().unwrap_or("").to_string(), type_ref(&a["type"]).0))
                        .collect(),
                    description: f["description"].as_str().unwrap_or("").to_string(),
                }
            })
            .collect();
        types.insert(
            type_name,
            TypeInfo { fields },
        );
    }
    Ok(Schema {
        query: name(&schema["queryType"]),
        mutation: name(&schema["mutationType"]),
        subscription: name(&schema["subscriptionType"]),
        types,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub name: String,
    pub detail: String,
    pub description: String,
    /// Inserting it should open a `{ }` selection.
    pub has_fields: bool,
}

/// What to suggest at `cursor` (a char index): the byte range of the word being typed, and the
/// matching fields of the type the cursor is in. `force` also suggests with nothing typed yet.
pub fn complete(schema: &Schema, text: &str, cursor: usize, force: bool) -> Option<(std::ops::Range<usize>, Vec<Suggestion>)> {
    let at = text.char_indices().nth(cursor).map(|(i, _)| i).unwrap_or(text.len());
    let before = &text[..at];
    let word_start = before.rfind(|c: char| !(c.is_alphanumeric() || c == '_')).map(|i| i + 1).unwrap_or(0);
    let prefix = &before[word_start..];
    if prefix.is_empty() && !force {
        return None;
    }
    if prefix.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }

    // Walk the text before the word, keeping a stack of the types whose `{ }` we're inside.
    let mut stack: Vec<Option<String>> = Vec::new();
    let mut parens = 0usize;
    let mut last_ident = String::new();
    let mut prev_ident = String::new();
    let mut root: Option<String> = None;
    let mut type_condition: Option<String> = None;
    let mut chars = before[..word_start].chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '#' => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            '"' => {
                let mut escaped = false;
                for c in chars.by_ref() {
                    match c {
                        '\\' if !escaped => escaped = true,
                        '"' if !escaped => break,
                        _ => escaped = false,
                    }
                }
            }
            '(' => parens += 1,
            ')' => parens = parens.saturating_sub(1),
            '{' if parens == 0 => {
                let ty = if let Some(t) = type_condition.take() {
                    Some(t)
                } else if stack.is_empty() {
                    root.take().or_else(|| schema.query.clone())
                } else {
                    stack
                        .last()
                        .cloned()
                        .flatten()
                        .and_then(|parent| schema.fields(&parent).iter().find(|f| f.name == last_ident).map(|f| f.base.clone()))
                };
                stack.push(ty);
                last_ident.clear();
            }
            '}' if parens == 0 => {
                stack.pop();
                last_ident.clear();
            }
            c if c.is_alphanumeric() || c == '_' => {
                let mut ident = String::from(c);
                while let Some(&n) = chars.peek() {
                    if n.is_alphanumeric() || n == '_' {
                        ident.push(n);
                        chars.next();
                    } else {
                        break;
                    }
                }
                if parens > 0 {
                    continue;
                }
                if stack.is_empty() {
                    match ident.as_str() {
                        "query" => root = schema.query.clone(),
                        "mutation" => root = schema.mutation.clone(),
                        "subscription" => root = schema.subscription.clone(),
                        _ if prev_ident == "on" => type_condition = Some(ident.clone()),
                        _ => {}
                    }
                } else if prev_ident == "on" {
                    type_condition = Some(ident.clone());
                }
                prev_ident = ident.clone();
                last_ident = ident;
            }
            _ => {}
        }
    }
    if parens > 0 {
        return None;
    }

    let lower = prefix.to_lowercase();
    let mut out: Vec<Suggestion> = if stack.is_empty() {
        let mut ops = vec![("query", &schema.query), ("mutation", &schema.mutation), ("subscription", &schema.subscription)];
        ops.retain(|(_, t)| t.is_some());
        ops.into_iter()
            .map(|(k, t)| Suggestion { name: k.into(), detail: t.clone().unwrap_or_default(), description: String::new(), has_fields: true })
            .chain(std::iter::once(Suggestion { name: "fragment".into(), detail: "keyword".into(), description: String::new(), has_fields: false }))
            .collect()
    } else {
        let ty = stack.last().cloned().flatten()?;
        schema
            .fields(&ty)
            .iter()
            .map(|f| Suggestion {
                name: f.name.clone(),
                detail: f.type_label.clone(),
                description: f.description.clone(),
                has_fields: schema.has_fields(&f.base),
            })
            .chain(std::iter::once(Suggestion { name: "__typename".into(), detail: "String!".into(), description: String::new(), has_fields: false }))
            .collect()
    };
    // Matches at the start first, then anywhere in the name.
    out.retain(|s| {
        s.name.to_lowercase().contains(&lower)
            && s.name != prefix
            // Built-ins like `__typename` only when asked for, so they don't crowd the list.
            && (!s.name.starts_with("__") || prefix.is_empty() || prefix.starts_with('_'))
    });
    out.sort_by_key(|s| !s.name.to_lowercase().starts_with(&lower));
    out.truncate(12);
    (!out.is_empty()).then_some((word_start..at, out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn t(kind: &str, name: Option<&str>, of: Option<Value>) -> Value {
        json!({ "kind": kind, "name": name, "ofType": of })
    }

    fn schema() -> Schema {
        let named = |n: &str| t("OBJECT", Some(n), None);
        let scalar = |n: &str| t("SCALAR", Some(n), None);
        let resp = json!({ "data": { "__schema": {
            "queryType": { "name": "Query" }, "mutationType": { "name": "Mutation" }, "subscriptionType": null,
            "types": [
                { "kind": "OBJECT", "name": "Query", "fields": [
                    { "name": "students", "description": "All students", "args": [], "type": t("NON_NULL", None, Some(t("LIST", None, Some(named("Student"))))) },
                    { "name": "student", "args": [ { "name": "id", "type": t("NON_NULL", None, Some(scalar("ID"))) } ], "type": named("Student") }
                ] },
                { "kind": "OBJECT", "name": "Mutation", "fields": [
                    { "name": "addStudent", "args": [], "type": named("Student") }
                ] },
                { "kind": "OBJECT", "name": "Student", "fields": [
                    { "name": "id", "args": [], "type": scalar("ID") },
                    { "name": "name", "args": [], "type": scalar("String") },
                    { "name": "course", "args": [], "type": named("Course") }
                ] },
                { "kind": "OBJECT", "name": "Course", "fields": [ { "name": "title", "args": [], "type": scalar("String") } ] },
                { "kind": "SCALAR", "name": "String" }
            ]
        } } });
        parse_introspection(&resp).unwrap()
    }

    fn names(text: &str, force: bool) -> Vec<String> {
        let cursor = text.chars().count();
        complete(&schema(), text, cursor, force).map(|(_, s)| s.into_iter().map(|s| s.name).collect()).unwrap_or_default()
    }

    #[test]
    fn parses_types() {
        let s = schema();
        assert_eq!(s.query.as_deref(), Some("Query"));
        let students = &s.fields("Query")[0];
        assert_eq!(students.type_label, "[Student]!");
        assert_eq!(students.base, "Student");
        assert_eq!(s.fields("Query")[1].args, vec![("id".to_string(), "ID!".to_string())]);
        assert!(parse_introspection(&json!({ "errors": [ { "message": "no" } ] })).is_err());
    }

    #[test]
    fn completes_by_context() {
        assert_eq!(names("{ stu", false), vec!["students", "student"]);
        assert_eq!(names("query { students { na", false), vec!["name"]);
        assert_eq!(names("query Q { student(id: 1) { course { ti", false), vec!["title"]);
        assert_eq!(names("mutation { add", false), vec!["addStudent"]);
        assert_eq!(names("qu", false), vec!["query"]);
        // Nested braces close properly, comments and strings are skipped.
        assert_eq!(names("{ students { course { title } # {\n n", false), vec!["name"]);
        // Inside arguments there's nothing to suggest, and an empty word needs `force`.
        assert!(names("{ student(i", false).is_empty());
        assert!(names("{ students { ", false).is_empty());
        assert_eq!(names("{ students { ", true).len(), 4);
        // Unknown fields don't crash, they just suggest nothing.
        assert!(names("{ nope { x", false).is_empty());
        let (range, _) = complete(&schema(), "{ students { na }", 15, false).unwrap();
        assert_eq!(range, 13..15);
    }
}
