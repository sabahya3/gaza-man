//! Data files for the collection runner: each row (CSV) or object (JSON) is one iteration,
//! and its columns become variables, like Postman's data files.

/// Rows of `(column, value)`.
pub type Rows = Vec<Vec<(String, String)>>;

/// RFC 4180 CSV: quoted fields, doubled quotes, commas and newlines inside quotes.
fn csv_records(text: &str) -> Vec<Vec<String>> {
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                chars.next();
                field.push('"');
            }
            ('"', true) => quoted = false,
            ('"', false) if field.is_empty() => quoted = true,
            (',', false) => record.push(std::mem::take(&mut field)),
            ('\r', false) => {}
            ('\n', false) => {
                record.push(std::mem::take(&mut field));
                if !(record.len() == 1 && record[0].is_empty()) {
                    records.push(std::mem::take(&mut record));
                }
                record.clear();
            }
            (c, _) => field.push(c),
        }
    }
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    records
}

pub fn parse_csv(text: &str) -> Result<Rows, String> {
    let mut records = csv_records(text).into_iter();
    let header = records.next().ok_or("The CSV file is empty")?;
    let header: Vec<String> = header.into_iter().map(|h| h.trim().to_string()).collect();
    Ok(records
        .map(|r| header.iter().cloned().zip(r.into_iter().chain(std::iter::repeat(String::new()))).collect())
        .collect())
}

pub fn parse_json(text: &str) -> Result<Rows, String> {
    let value: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("Invalid JSON: {e}"))?;
    let list = value.as_array().ok_or("A JSON data file must be an array of objects")?;
    list.iter()
        .map(|item| {
            item.as_object()
                .map(|o| o.iter().map(|(k, v)| (k.clone(), crate::tests::plain(v))).collect())
                .ok_or_else(|| "Every item in the JSON array must be an object".to_string())
        })
        .collect()
}

pub fn load(path: &str) -> Result<Rows, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("Could not read {path}: {e}"))?;
    let rows = if path.to_ascii_lowercase().ends_with(".json") || text.trim_start().starts_with('[') {
        parse_json(&text)?
    } else {
        parse_csv(&text)?
    };
    if rows.is_empty() {
        return Err("The data file has no rows".into());
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv() {
        let rows = parse_csv("name,city,note\r\nLina,Gaza,\"says \"\"hi\"\", twice\"\nOmar,\"Khan\nYounis\"\n\n").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][2], ("note".to_string(), "says \"hi\", twice".to_string()));
        assert_eq!(rows[1][1].1, "Khan\nYounis");
        assert_eq!(rows[1][2].1, "", "short rows are padded");
    }

    #[test]
    fn json() {
        let rows = parse_json(r#"[{"id": 1, "ok": true}, {"id": 2, "name": "x"}]"#).unwrap();
        assert_eq!(rows[0], vec![("id".to_string(), "1".to_string()), ("ok".to_string(), "true".to_string())]);
        assert!(parse_json("{}").is_err());
    }
}
