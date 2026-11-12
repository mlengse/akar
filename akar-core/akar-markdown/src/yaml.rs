//! A pure-Rust YAML subset parser for Markdown / Open Knowledge Format (OKF v0.2) frontmatter.
//!
//! Provides dependency-free nested map and list parsing without pulling in unmaintained crates.
//! Maps directly to `serde_json::Value` and recursive `FrontMatterValue`.

use serde_json::{Map, Value as JsonValue};

/// Serialize `value` to YAML.
pub fn to_string(value: &JsonValue) -> String {
    let mut out = String::new();
    emit(value, 0, &mut out);
    out
}

/// Serialize `value` to pretty JSON.
pub fn to_json_pretty(value: &JsonValue) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
}

fn emit(value: &JsonValue, indent: usize, out: &mut String) {
    let pad = " ".repeat(indent);
    match value {
        JsonValue::Object(map) => {
            if map.is_empty() {
                out.push_str(&format!("{pad}{{}}\n"));
                return;
            }
            for (key, member) in map {
                let key = quote(key);
                match member {
                    JsonValue::Object(map) if map.is_empty() => {
                        out.push_str(&format!("{pad}{key}: {{}}\n"));
                    }
                    JsonValue::Array(items) if items.is_empty() => {
                        out.push_str(&format!("{pad}{key}: []\n"));
                    }
                    JsonValue::Object(_) | JsonValue::Array(_) => {
                        out.push_str(&format!("{pad}{key}:\n"));
                        emit(member, indent + 2, out);
                    }
                    scalar => out.push_str(&format!("{pad}{key}: {}\n", render_scalar(scalar))),
                }
            }
        }
        JsonValue::Array(items) => {
            if items.is_empty() {
                out.push_str(&format!("{pad}[]\n"));
                return;
            }
            for item in items {
                match item {
                    JsonValue::Object(_) | JsonValue::Array(_) => {
                        let mut body = String::new();
                        emit(item, indent + 2, &mut body);
                        let mut lines = body.lines();
                        if let Some(first) = lines.next() {
                            out.push_str(&format!("{pad}- {}\n", first.trim_start()));
                        }
                        for line in lines {
                            out.push_str(line);
                            out.push('\n');
                        }
                    }
                    scalar => out.push_str(&format!("{pad}- {}\n", render_scalar(scalar))),
                }
            }
        }
        scalar => out.push_str(&format!("{pad}{}\n", render_scalar(scalar))),
    }
}

fn render_scalar(value: &JsonValue) -> String {
    match value {
        JsonValue::Null => "null".to_string(),
        JsonValue::Bool(flag) => flag.to_string(),
        JsonValue::Number(number) => number.to_string(),
        JsonValue::String(text) => quote(text),
        other => quote(&other.to_string()),
    }
}

fn quote(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for character in text.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            character if (character as u32) < 0x20 => {
                quoted.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted
}

// ---------------------------------------------------------------------------
// Subset parser
// ---------------------------------------------------------------------------

struct Line {
    number: usize,
    indent: usize,
    content: String,
    blank: bool,
}

/// Parse the YAML subset into JSON.
pub fn parse(input: &str) -> Result<JsonValue, String> {
    let lines: Vec<Line> = input
        .lines()
        .enumerate()
        .map(|(number, raw)| {
            let indent = raw.len() - raw.trim_start().len();
            let trimmed = raw.trim_start();
            let blank = trimmed.is_empty()
                || trimmed.starts_with('#')
                || trimmed == "---"
                || trimmed == "...";
            Line {
                number: number + 1,
                indent: if blank { usize::MAX } else { indent },
                content: if blank {
                    String::new()
                } else {
                    trimmed.to_string()
                },
                blank,
            }
        })
        .collect();
    let Some(first) = lines.iter().position(|line| !line.blank) else {
        return Ok(JsonValue::Object(Map::new()));
    };
    let mut cursor = first;
    parse_block(&lines, &mut cursor, lines[first].indent)
}

fn parse_block(lines: &[Line], cursor: &mut usize, indent: usize) -> Result<JsonValue, String> {
    if *cursor >= lines.len() {
        return Err("unexpected end of document".to_string());
    }
    if lines[*cursor].content.starts_with("- ") || lines[*cursor].content == "-" {
        parse_sequence(lines, cursor, indent)
    } else {
        parse_mapping(lines, cursor, indent)
    }
}

fn parse_mapping(lines: &[Line], cursor: &mut usize, indent: usize) -> Result<JsonValue, String> {
    let mut map = Map::new();
    while *cursor < lines.len() {
        let line = &lines[*cursor];
        if line.blank {
            *cursor += 1;
            continue;
        }
        if line.indent < indent {
            break;
        }
        require(line.indent == indent, || {
            format!(
                "line {}: inconsistent indentation ({}, expected {indent})",
                line.number, line.indent
            )
        })?;
        if line.content.starts_with("- ") || line.content == "-" {
            break;
        }
        let (key, rest) = split_mapping_line(line)?;
        let key = plain_scalar(&key);
        let value = parse_value_member(lines, cursor, indent, &rest)?;
        map.insert(key, value);
    }
    Ok(JsonValue::Object(map))
}

fn parse_value_member(
    lines: &[Line],
    cursor: &mut usize,
    key_indent: usize,
    rest: &str,
) -> Result<JsonValue, String> {
    let rest = rest.trim();
    if rest.is_empty() {
        *cursor += 1;
        if *cursor < lines.len() && lines[*cursor].indent > key_indent {
            let inner = lines[*cursor].indent;
            parse_block(lines, cursor, inner)
        } else {
            Ok(JsonValue::Null)
        }
    } else if block_marker(rest) {
        *cursor += 1;
        gather_block(lines, cursor, key_indent, rest)
    } else {
        let value = parse_scalar(rest)?;
        *cursor += 1;
        Ok(value)
    }
}

fn block_marker(text: &str) -> bool {
    matches!(text, "|" | "|-" | "|+" | ">" | ">-" | ">+")
}

fn require(condition: bool, message: impl FnOnce() -> String) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message())
    }
}

fn parse_sequence(lines: &[Line], cursor: &mut usize, indent: usize) -> Result<JsonValue, String> {
    let mut items = Vec::new();
    while *cursor < lines.len() {
        let line = &lines[*cursor];
        if line.blank {
            *cursor += 1;
            continue;
        }
        if line.indent != indent || !line.content.starts_with('-') {
            break;
        }
        if line.content == "-" {
            *cursor += 1;
            items.push(if *cursor < lines.len() && lines[*cursor].indent > indent {
                parse_block(lines, cursor, lines[*cursor].indent)?
            } else {
                JsonValue::Null
            });
            continue;
        }
        let rest = line.content[2..].trim().to_string();
        if rest.is_empty() {
            *cursor += 1;
            items.push(if *cursor < lines.len() && lines[*cursor].indent > indent {
                parse_block(lines, cursor, lines[*cursor].indent)?
            } else {
                JsonValue::Null
            });
            continue;
        }
        if block_marker(&rest) {
            *cursor += 1;
            items.push(gather_block(lines, cursor, indent, &rest)?);
            continue;
        }
        if split_colon(&rest).is_some() {
            items.push(parse_sequence_map(lines, cursor, indent)?);
            continue;
        }
        items.push(parse_scalar(&rest)?);
        *cursor += 1;
    }
    Ok(JsonValue::Array(items))
}

fn parse_sequence_map(
    lines: &[Line],
    cursor: &mut usize,
    list_indent: usize,
) -> Result<JsonValue, String> {
    let deeper = list_indent + 2;
    let dash = &lines[*cursor];
    let rest = dash.content[2..].trim().to_string();
    let mut map = Map::new();
    let (key, tail) = split_colon(&rest)
        .map(|(key, rest)| (key.to_string(), rest.trim().to_string()))
        .ok_or_else(|| format!("line {}: dash item without `key:`", dash.number))?;
    let key = plain_scalar(&key);
    parse_item_pair(lines, cursor, deeper, key, &tail, &mut map)?;
    while *cursor < lines.len() {
        let line = &lines[*cursor];
        if line.blank {
            *cursor += 1;
            continue;
        }
        if line.indent < deeper {
            break;
        }
        require(line.indent == deeper, || {
            format!(
                "line {}: inconsistent indentation ({}, expected {deeper})",
                line.number, line.indent
            )
        })?;
        if line.content.starts_with("- ") || line.content == "-" {
            break;
        }
        let (key, rest) = split_mapping_line(line)?;
        let key = plain_scalar(&key);
        parse_item_pair(lines, cursor, deeper, key, rest.trim(), &mut map)?;
    }
    Ok(JsonValue::Object(map))
}

fn parse_item_pair(
    lines: &[Line],
    cursor: &mut usize,
    key_indent: usize,
    key: String,
    rest: &str,
    map: &mut Map<String, JsonValue>,
) -> Result<(), String> {
    let value = parse_value_member(lines, cursor, key_indent, rest)?;
    map.insert(key, value);
    Ok(())
}

fn gather_block(
    lines: &[Line],
    cursor: &mut usize,
    key_indent: usize,
    marker: &str,
) -> Result<JsonValue, String> {
    if *cursor >= lines.len() || lines[*cursor].indent <= key_indent {
        return Ok(JsonValue::String(String::new()));
    }
    let base = lines[*cursor].indent;
    let mut gathered: Vec<String> = Vec::new();
    while *cursor < lines.len() {
        let line = &lines[*cursor];
        if line.blank {
            let mut peek = *cursor;
            while peek < lines.len() && lines[peek].blank {
                peek += 1;
            }
            if peek < lines.len() && lines[peek].indent > key_indent {
                gathered.push(String::new());
                *cursor += 1;
                continue;
            }
            break;
        }
        if line.indent <= key_indent {
            break;
        }
        let extra = line.indent.saturating_sub(base);
        gathered.push(format!("{}{}", " ".repeat(extra), line.content));
        *cursor += 1;
    }
    let kind = marker.chars().next().unwrap_or('|');
    let suffix = &marker[1..];
    Ok(JsonValue::String(materialize(kind, suffix, &gathered)))
}

fn materialize(kind: char, suffix: &str, gathered: &[String]) -> String {
    let mut text = String::new();
    if kind == '|' {
        for line in gathered {
            text.push_str(line);
            text.push('\n');
        }
    } else {
        let mut previous_more_indented = false;
        for (index, line) in gathered.iter().enumerate() {
            let more_indented = line.starts_with(' ');
            if index > 0 {
                if line.is_empty() {
                    text.push('\n');
                    previous_more_indented = false;
                    continue;
                }
                if previous_more_indented {
                    text.push('\n');
                } else {
                    text.push(' ');
                }
            }
            previous_more_indented = more_indented;
            text.push_str(line.trim_start());
        }
    }
    chomp(text, suffix)
}

fn chomp(mut text: String, suffix: &str) -> String {
    while text.ends_with('\n') {
        text.pop();
    }
    match suffix {
        "-" => text,
        "+" => text,
        _ => format!("{text}\n"),
    }
}

fn split_mapping_line(line: &Line) -> Result<(String, String), String> {
    match split_colon(&line.content) {
        Some((key, rest)) => Ok((key.to_string(), rest.to_string())),
        None => Err(format!(
            "line {}: expected `key: value`, got `{}`",
            line.number, line.content
        )),
    }
}

fn split_colon(text: &str) -> Option<(&str, &str)> {
    let bytes = text.as_bytes();
    let mut depth = 0_i32;
    let mut quoted = false;
    let mut previous = b'\0';
    for (index, byte) in bytes.iter().enumerate() {
        match byte {
            b'"' if previous != b'\\' => quoted = !quoted,
            b'[' | b'{' if !quoted => depth += 1,
            b']' | b'}' if !quoted => depth -= 1,
            b':' if !quoted && depth == 0 => {
                let (_, rest) = text.split_at(index + 1);
                return Some((&text[..index], rest.trim_start()));
            }
            _ => {}
        }
        previous = *byte;
    }
    None
}

fn plain_scalar(text: &str) -> String {
    match parse_scalar(text) {
        Ok(JsonValue::String(text)) => text,
        Ok(other) => other.to_string(),
        Err(_) => text.to_string(),
    }
}

fn parse_scalar(text: &str) -> Result<JsonValue, String> {
    parse_plain(text).ok_or_else(|| {
        format!(
            "unsupported scalar `{}` (re-export with --format json)",
            text.split_at(text.len().min(60)).0
        )
    })
}

fn parse_plain(text: &str) -> Option<JsonValue> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Some(JsonValue::Null);
    }
    if let Some(body) = trimmed.strip_prefix('"') {
        if trimmed.len() >= 2 && trimmed.ends_with('"') {
            return Some(JsonValue::String(unescape(&body[..body.len() - 1])));
        }
        return None;
    }
    if let Some(body) = trimmed.strip_prefix('\'') {
        if trimmed.len() >= 2 && trimmed.ends_with('\'') {
            return Some(JsonValue::String(body[..body.len() - 1].replace("''", "'")));
        }
        return None;
    }
    if trimmed.starts_with('&') || trimmed.starts_with('*') {
        return None;
    }
    let trimmed = strip_inline_comment(trimmed);
    match trimmed {
        "null" | "Null" | "NULL" | "~" => Some(JsonValue::Null),
        "true" | "True" | "TRUE" => Some(JsonValue::Bool(true)),
        "false" | "False" | "FALSE" => Some(JsonValue::Bool(false)),
        "{}" => Some(JsonValue::Object(Map::new())),
        "[]" => Some(JsonValue::Array(Vec::new())),
        other => {
            if other.starts_with('[') || other.starts_with('{') {
                return serde_json::from_str(other).ok();
            }
            if let Ok(number) = other.parse::<i64>() {
                return Some(serde_json::json!(number));
            }
            if let Ok(number) = other.parse::<f64>() {
                return serde_json::Number::from_f64(number).map(serde_json::Value::Number);
            }
            Some(serde_json::Value::String(other.to_string()))
        }
    }
}

fn strip_inline_comment(text: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut after_whitespace = true;
    for (index, character) in text.char_indices() {
        match quote {
            Some(active) => {
                if escaped {
                    escaped = false;
                } else if character == '\\' && active == '"' {
                    escaped = true;
                } else if character == active {
                    quote = None;
                }
            }
            None => {
                if character == '"' || character == '\'' {
                    quote = Some(character);
                } else if character == '#' && after_whitespace {
                    return text[..index].trim_end();
                }
            }
        }
        after_whitespace = quote.is_none() && character.is_whitespace();
    }
    text.trim_end()
}

fn unescape(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut characters = body.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }
        match characters.next() {
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('/') => out.push('/'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('0') => out.push('\0'),
            Some('u') => {
                let hex: String = characters.by_ref().take(4).collect();
                if let Ok(code) = u32::from_str_radix(&hex, 16) {
                    if let Some(ch) = char::from_u32(code) {
                        out.push(ch);
                        continue;
                    }
                }
                out.push_str("\\u");
                out.push_str(&hex);
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_yaml_nested_maps_and_sequences() {
        let yaml = r#"
title: "OKF Note"
meta:
  version: 0.2
  authors:
    - name: "Alice"
      role: "author"
    - name: "Bob"
      role: "reviewer"
tags:
  - knowledge
  - graph
count: 42
verified: true
"#;
        let parsed = parse(yaml).expect("parse okf yaml");
        assert_eq!(parsed["title"], json!("OKF Note"));
        assert_eq!(parsed["meta"]["version"], json!(0.2));
        assert_eq!(parsed["meta"]["authors"][0]["name"], json!("Alice"));
        assert_eq!(parsed["tags"][1], json!("graph"));
        assert_eq!(parsed["count"], json!(42));
        assert_eq!(parsed["verified"], json!(true));
    }
}
