use crate::registry::*;
use kuzu_common::types::Value;
use super::{get_cached_regex};


// ==================== String ====================

fn soundex_impl(s: &str) -> String {
    let mut chars = s.chars().filter(|c| c.is_ascii_alphabetic());
    let first_char = match chars.next() {
        Some(c) => c.to_ascii_uppercase(),
        None => return "".to_string(),
    };

    let mut result = String::with_capacity(4);
    result.push(first_char);

    let get_code = |c: char| -> char {
        match c.to_ascii_uppercase() {
            'B' | 'F' | 'P' | 'V' => '1',
            'C' | 'G' | 'J' | 'K' | 'Q' | 'S' | 'X' | 'Z' => '2',
            'D' | 'T' => '3',
            'L' => '4',
            'M' | 'N' => '5',
            'R' => '6',
            _ => '0',
        }
    };

    let mut prev_code = get_code(first_char);
    for c in chars {
        let code = get_code(c);
        if code != '0' && code != prev_code {
            result.push(code);
            if result.len() == 4 {
                break;
            }
        }
        if c.to_ascii_uppercase() != 'H' && c.to_ascii_uppercase() != 'W' {
            prev_code = code;
        }
    }

    while result.len() < 4 {
        result.push('0');
    }
    result
}

pub(crate) fn evaluate_string(op: StringOp, args: &[Value]) -> Result<Value, String> {
    if args.is_empty() {
        return Err("String function requires arguments".into());
    }

    match op {
        StringOp::Concat => {
            let s: String = args
                .iter()
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    Value::Null => "NULL".into(),
                    other => format!("{:?}", other),
                })
                .collect();
            Ok(Value::String(s))
        }
        StringOp::Contains => {
            let s = get_string(&args[0])?;
            let pat = get_string(&args[1])?;
            Ok(Value::Bool(s.contains(&pat)))
        }
        StringOp::StartsWith => {
            let s = get_string(&args[0])?;
            let pat = get_string(&args[1])?;
            Ok(Value::Bool(s.starts_with(&pat)))
        }
        StringOp::EndsWith => {
            let s = get_string(&args[0])?;
            let pat = get_string(&args[1])?;
            Ok(Value::Bool(s.ends_with(&pat)))
        }
        StringOp::Like => {
            let s = get_string(&args[0])?;
            let pat = get_string(&args[1])?;
            // Convert SQL LIKE pattern to regex pattern
            let mut regex_str = String::with_capacity(pat.len() + 2);
            regex_str.push('^');
            for ch in pat.chars() {
                match ch {
                    '%' => regex_str.push_str(".*"),
                    '_' => regex_str.push('.'),
                    // Escape regex metacharacters
                    '.' | '\\' | '+' | '*' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' => {
                        regex_str.push('\\');
                        regex_str.push(ch);
                    }
                    other => regex_str.push(other),
                }
            }
            regex_str.push('$');
            let re = get_cached_regex(&regex_str)?;
            Ok(Value::Bool(re.is_match(&s)))
        }
        StringOp::ToUpper => {
            let s = get_string(&args[0])?;
            Ok(Value::String(s.to_uppercase()))
        }
        StringOp::ToLower => {
            let s = get_string(&args[0])?;
            Ok(Value::String(s.to_lowercase()))
        }
        StringOp::Trim => {
            let s = get_string(&args[0])?;
            Ok(Value::String(s.trim().to_string()))
        }
        StringOp::LTrim => {
            let s = get_string(&args[0])?;
            Ok(Value::String(s.trim_start().to_string()))
        }
        StringOp::RTrim => {
            let s = get_string(&args[0])?;
            Ok(Value::String(s.trim_end().to_string()))
        }
        StringOp::Length => {
            let s = get_string(&args[0])?;
            Ok(Value::Int64(s.len() as i64))
        }
        StringOp::Reverse => {
            let s = get_string(&args[0])?;
            Ok(Value::String(s.chars().rev().collect()))
        }
        StringOp::Repeat => {
            let s = get_string(&args[0])?;
            let n = match &args[1] {
                Value::Int64(x) => *x as usize,
                _ => return Err("Repeat count must be integer".into()),
            };
            Ok(Value::String(s.repeat(n)))
        }
        StringOp::Replace => {
            let s = get_string(&args[0])?;
            let from = get_string(&args[1])?;
            let to = get_string(&args[2])?;
            Ok(Value::String(s.replace(&from, &to)))
        }
        StringOp::Substring => {
            let s = get_string(&args[0])?;
            // Cypher uses 1-based indexing
            let start = match &args[1] {
                Value::Int64(x) => {
                    if *x < 1 {
                        return Err("Substring start must be >= 1".into());
                    }
                    (*x - 1) as usize
                }
                _ => return Err("Start must be integer".into()),
            };
            let len = if args.len() > 2 {
                match &args[2] {
                    Value::Int64(x) => Some(*x as usize),
                    _ => None,
                }
            } else {
                None
            };
            let result = match len {
                Some(l) => s.chars().skip(start).take(l).collect(),
                None => s.chars().skip(start).collect(),
            };
            Ok(Value::String(result))
        }
        StringOp::RegexMatches => {
            let s = get_string(&args[0])?;
            let pat = get_string(&args[1])?;
            let re = get_cached_regex(&pat)?;
            Ok(Value::Bool(re.is_match(&s)))
        }
        StringOp::RegexReplace => {
            let s = get_string(&args[0])?;
            let pat = get_string(&args[1])?;
            let repl = get_string(&args[2])?;
            let re = get_cached_regex(&pat)?;
            Ok(Value::String(re.replace_all(&s, repl).to_string()))
        }
        StringOp::Split => {
            let s = get_string(&args[0])?;
            let delim = if args.len() > 1 {
                get_string(&args[1])?
            } else {
                ",".to_string()
            };
            let parts: Vec<Value> = s.split(&delim).map(|p| Value::String(p.to_string())).collect();
            Ok(Value::List(parts))
        }
        StringOp::Head => {
            let s = get_string(&args[0])?;
            let n = if args.len() > 1 {
                match &args[1] {
                    Value::Int64(x) => *x as usize,
                    _ => 1,
                }
            } else {
                1
            };
            Ok(Value::String(s.chars().take(n).collect()))
        }
        StringOp::Tail => {
            let s = get_string(&args[0])?;
            let n = if args.len() > 1 {
                match &args[1] {
                    Value::Int64(x) => *x as usize,
                    _ => 1,
                }
            } else {
                1
            };
            let chars: String = s.chars().collect();
            let start = chars.len().saturating_sub(n);
            Ok(Value::String(chars.chars().skip(start).collect()))
        }
        StringOp::Left => {
            let s = get_string(&args[0])?;
            let n = match &args[1] {
                Value::Int64(x) => *x as usize,
                _ => return Err("left requires integer length".into()),
            };
            Ok(Value::String(s.chars().take(n).collect()))
        }
        StringOp::Right => {
            let s = get_string(&args[0])?;
            let n = match &args[1] {
                Value::Int64(x) => *x as usize,
                _ => return Err("right requires integer length".into()),
            };
            let chars: Vec<char> = s.chars().collect();
            let start = chars.len().saturating_sub(n);
            Ok(Value::String(chars[start..].iter().collect()))
        }
        StringOp::Lpad => {
            let s = get_string(&args[0])?;
            let len = match &args[1] {
                Value::Int64(x) => *x as usize,
                _ => return Err("lpad requires integer length".into()),
            };
            let pad = if args.len() >= 3 {
                get_string(&args[2])?
            } else {
                " ".into()
            };
            if s.len() >= len {
                return Ok(Value::String(s[..len].to_string()));
            }
            let pad_needed = len - s.len();
            let pad_repeat = pad.repeat((pad_needed / pad.len()) + 1);
            Ok(Value::String(format!("{}{}", &pad_repeat[..pad_needed], s)))
        }
        StringOp::Rpad => {
            let s = get_string(&args[0])?;
            let len = match &args[1] {
                Value::Int64(x) => *x as usize,
                _ => return Err("rpad requires integer length".into()),
            };
            let pad = if args.len() >= 3 {
                get_string(&args[2])?
            } else {
                " ".into()
            };
            if s.len() >= len {
                return Ok(Value::String(s[..len].to_string()));
            }
            let pad_needed = len - s.len();
            let pad_repeat = pad.repeat((pad_needed / pad.len()) + 1);
            Ok(Value::String(format!("{}{}", s, &pad_repeat[..pad_needed])))
        }
        // --- String basic (C++ port) ---
        StringOp::InitCap => {
            let s = get_string(&args[0])?;
            let lower = s.to_lowercase();
            let mut chars = lower.chars();
            match chars.next() {
                None => Ok(Value::String(String::new())),
                Some(c) => Ok(Value::String(c.to_uppercase().collect::<String>() + chars.as_str())),
            }
        }
        StringOp::ConcatWs => {
            if args.len() < 2 {
                return Err("concat_ws requires at least 2 arguments (separator + strings)".into());
            }
            let separator = get_string(&args[0])?;
            let mut result = String::new();
            let mut first = true;
            for arg in args.iter().skip(1) {
                match arg {
                    Value::Null => {
                        // Skip NULL elements (no separator before or after)
                        continue;
                    }
                    Value::String(s) => {
                        if !first {
                            result.push_str(&separator);
                        }
                        result.push_str(s);
                        first = false;
                    }
                    _ => {
                        if !first {
                            result.push_str(&separator);
                        }
                        result.push_str(&format!("{:?}", arg));
                        first = false;
                    }
                }
            }
            Ok(Value::String(result))
        }
        StringOp::SplitPart => {
            if args.len() < 3 {
                return Err("split_part requires 3 arguments (string, delimiter, index)".into());
            }
            let s = get_string(&args[0])?;
            let delim = get_string(&args[1])?;
            let idx = match &args[2] {
                Value::Int64(x) => *x,
                _ => return Err("split_part index must be integer".into()),
            };
            // 1-based index, matching C++ semantics
            let parts: Vec<&str> = s.split(&delim).collect();
            if idx <= 0 || (idx as usize) > parts.len() {
                Ok(Value::String(String::new()))
            } else {
                Ok(Value::String(parts[(idx - 1) as usize].to_string()))
            }
        }
        StringOp::ArrayExtract => {
            if args.len() < 2 {
                return Err("array_extract requires 2 arguments (string, index)".into());
            }
            let s = get_string(&args[0])?;
            let idx = match &args[1] {
                Value::Int64(x) => *x,
                _ => return Err("array_extract index must be integer".into()),
            };
            let chars: Vec<char> = s.chars().collect();
            if idx == 0 || chars.is_empty() {
                Ok(Value::String(String::new()))
            } else if idx > 0 {
                // 1-based: clamp to string length
                let pos = (idx as usize).saturating_sub(1).min(chars.len() - 1);
                Ok(Value::String(chars[pos].to_string()))
            } else {
                // Negative: from end (-1 = last char)
                let abs_idx = (-idx) as usize;
                if abs_idx > chars.len() {
                    Ok(Value::String(String::new()))
                } else {
                    let pos = chars.len() - abs_idx;
                    Ok(Value::String(chars[pos].to_string()))
                }
            }
        }
        // --- Regex string functions (C++ port) ---
        StringOp::RegexpFullMatch => {
            let s = get_string(&args[0])?;
            let pat = get_string(&args[1])?;
            let re = get_cached_regex(&pat)?;
            Ok(Value::Bool(
                re.find(&s).is_some_and(|m| m.start() == 0 && m.end() == s.len()),
            ))
        }
        StringOp::RegexpExtract => {
            let s = get_string(&args[0])?;
            let pat = get_string(&args[1])?;
            let group = if args.len() > 2 {
                match &args[2] {
                    Value::Int64(x) => *x as usize,
                    _ => return Err("RegexpExtract group must be integer".into()),
                }
            } else {
                0
            };
            let re = get_cached_regex(&pat)?;
            let result = re
                .captures(&s)
                .and_then(|caps| caps.get(group))
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
            Ok(Value::String(result))
        }
        StringOp::RegexpExtractAll => {
            let s = get_string(&args[0])?;
            let pat = get_string(&args[1])?;
            let group = if args.len() > 2 {
                match &args[2] {
                    Value::Int64(x) => *x as usize,
                    _ => return Err("RegexpExtractAll group must be integer".into()),
                }
            } else {
                0
            };
            let re = get_cached_regex(&pat)?;
            let matches: Vec<Value> = re
                .captures_iter(&s)
                .filter_map(|caps| caps.get(group))
                .map(|m| Value::String(m.as_str().to_string()))
                .collect();
            Ok(Value::List(matches))
        }
        StringOp::RegexpSplitToArray => {
            let s = get_string(&args[0])?;
            let pat = get_string(&args[1])?;
            let re = get_cached_regex(&pat)?;
            let parts: Vec<Value> = re.split(&s).map(|p| Value::String(p.to_string())).collect();
            Ok(Value::List(parts))
        }
        StringOp::Levenshtein => {
            let a = get_string(&args[0])?;
            let b = get_string(&args[1])?;
            let a_chars: Vec<char> = a.chars().collect();
            let b_chars: Vec<char> = b.chars().collect();
            let n = b_chars.len();
            let mut prev_row: Vec<usize> = (0..=n).collect();
            let mut curr_row = vec![0usize; n + 1];
            for (i, ca) in a_chars.iter().enumerate() {
                curr_row[0] = i + 1;
                for (j, cb) in b_chars.iter().enumerate() {
                    let cost = if ca == cb { 0 } else { 1 };
                    curr_row[j + 1] = (curr_row[j] + 1).min(prev_row[j + 1] + 1).min(prev_row[j] + cost);
                }
                std::mem::swap(&mut prev_row, &mut curr_row);
            }
            Ok(Value::Int64(prev_row[n] as i64))
        }
        StringOp::Soundex => {
            let s = get_string(&args[0])?;
            Ok(Value::String(soundex_impl(&s)))
        }
    }
}

pub(crate) fn get_string(v: &Value) -> Result<String, String> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Null => Ok("NULL".into()),
        _ => Err(format!("Expected string, got {:?}", v.logical_type())),
    }
}
