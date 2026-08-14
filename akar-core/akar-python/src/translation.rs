//! Kuzu dialect → Akar dialect translation layer (P53.1–P53.6).
//!
//! Kairos' DDL/dialect uses Kuzu syntax that Akar's grammar does not accept:
//! `CREATE NODE/REL TABLE IF NOT EXISTS`, `DROP TABLE IF EXISTS`,
//! `FLOAT[n]` typed columns, `ALTER TABLE ... ADD col TYPE DEFAULT <lit>`,
//! `CALL CREATE_VECTOR_INDEX / DROP_VECTOR_INDEX / QUERY_VECTOR_INDEX`,
//! Kuzu `CREATE VECTOR INDEX ... FOR (...) ON (...) OPTIONS {...}`, and
//! multi-statement `INSTALL vector; LOAD EXTENSION vector`.
//!
//! This module rewrites those statements into Akar-native Cypher and keeps
//! the registries needed later: column dims (`FLOAT[n]`) for `CREATE VECTOR
//! INDEX ... WITH (dims=...)`, vector-index column mapping, and table column
//! lists for `RETURN node` → property-map expansion (P53.8).
//!
//! Pure string-level translation — no database access here. The executor
//! (`lib.rs`) resolves existence checks and table schemas from the live
//! catalog and performs the final query.

use std::collections::HashMap;

/// Table schema recorded from translated Kuzu DDL.
#[derive(Debug, Clone, Default)]
pub struct TableSchema {
    /// (column_name, optional dims extracted from `FLOAT[n]`).
    pub columns: Vec<(String, Option<usize>)>,
}

impl TableSchema {
    pub fn column_names(&self) -> Vec<String> {
        self.columns.iter().map(|(n, _)| n.clone()).collect()
    }

    pub fn dims_of(&self, col: &str) -> Option<usize> {
        self.columns
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(col))
            .and_then(|(_, d)| *d)
    }

    pub fn vector_column(&self) -> Option<&str> {
        self.columns.iter().find(|(_, d)| d.is_some()).map(|(n, _)| n.as_str())
    }
}

/// Mutable translation state, shared (via `Mutex`) across all connections
/// on a `Database`.
#[derive(Debug, Default)]
pub struct Translator {
    /// Lowercased table name → schema.
    tables: HashMap<String, TableSchema>,
    /// (table_lc, index_name_lc) → vector column.
    vec_indexes: HashMap<(String, String), String>,
}

impl Translator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_table(&mut self, name: &str, columns: Vec<(String, Option<usize>)>) {
        self.tables.insert(name.to_lowercase(), TableSchema { columns });
    }

    pub fn remove_table(&mut self, name: &str) {
        self.tables.remove(&name.to_lowercase());
    }

    pub fn table(&self, name: &str) -> Option<&TableSchema> {
        self.tables.get(&name.to_lowercase())
    }

    pub fn register_vec_index(&mut self, table: &str, index_name: &str, col: &str) {
        self.vec_indexes
            .insert((table.to_lowercase(), index_name.to_lowercase()), col.to_string());
    }

    pub fn vec_index_col(&self, table: &str, index_name: &str) -> Option<&str> {
        self.vec_indexes
            .get(&(table.to_lowercase(), index_name.to_lowercase()))
            .map(String::as_str)
    }
}

/// The outcome of translating a single statement.
#[derive(Debug, Clone)]
pub enum Translated {
    /// Skip entirely — `INSTALL`/`LOAD`/`UNINSTALL EXTENSION` no-op
    /// (Akar extensions are statically compiled).
    NoOp,
    /// Execute as-is (already Akar dialect).
    Query(String),
    /// Execute, swallowing an error that contains any of the needles.
    Swallow(String, &'static [&'static str]),
    /// `CREATE NODE/REL TABLE IF NOT EXISTS` — create only if absent.
    CreateTableIfNotExists { table: String, sql: String },
    /// `DROP TABLE IF EXISTS` — drop, swallowing "not found".
    DropTableIfExists { table: String, sql: String },
    /// `CALL QUERY_VECTOR_INDEX(...) RETURN node, distance [WHERE ...]` —
    /// resolved by the executor into a brute-force `MATCH` (read-path HNSW
    /// is write-only, SPEC §18 / P52.5).
    VectorQuery {
        table: String,
        index_name: String,
        vec_expr: String,
        limit_expr: String,
        /// Column holding the vector (resolved from the index registry,
        /// the FLOAT[n] column, or `embedding`).
        vec_col: String,
        where_sql: Option<String>,
    },
}

pub const ERR_ALREADY_EXISTS: &[&str] = &["already exists"];
pub const ERR_NOT_FOUND: &[&str] = &["not found"];
/// `dims=0` is rejected by Akar's binder; on a reopened DB the index already
/// exists and only this error is reachable (Kairos swallows both).
const ERR_DIMS_ZERO: &[&str] = &["already exists", "must be greater than 0"];

/// Split a (possibly multi-statement) input on `;`, honoring string and
/// backtick quotes so separators inside literals are preserved.
pub fn split_statements(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut in_backtick = false;
    for ch in input.chars() {
        match ch {
            '\'' if !in_double && !in_backtick => in_single = !in_single,
            '"' if !in_single && !in_backtick => in_double = !in_double,
            '`' if !in_single && !in_double => in_backtick = !in_backtick,
            ';' if !in_single && !in_double && !in_backtick => {
                let t = cur.trim();
                if !t.is_empty() {
                    out.push(t.to_string());
                }
                cur.clear();
                continue;
            }
            _ => {}
        }
        cur.push(ch);
    }
    let t = cur.trim();
    if !t.is_empty() {
        out.push(t.to_string());
    }
    out
}

/// Translate one statement. Returns `Err` only on unparseable Kuzu-specific
/// input that we recognise but cannot rewrite; anything unrecognised passes
/// through unchanged.
pub fn translate(statement: &str, translator: &mut Translator) -> Result<Translated, String> {
    let s = statement.trim();
    if s.is_empty() {
        return Ok(Translated::NoOp);
    }
    if is_extension_statement(s) {
        return Ok(Translated::NoOp);
    }
    if starts_keyword(s, "CREATE NODE TABLE") || starts_keyword(s, "CREATE REL TABLE") {
        return translate_create_table(s, translator);
    }
    if starts_keyword(s, "DROP TABLE") {
        return translate_drop_table(s);
    }
    if starts_keyword(s, "ALTER TABLE") {
        return translate_alter(s);
    }
    if starts_keyword(s, "CREATE VECTOR INDEX") {
        return translate_create_vector_index_kuzu(s, translator);
    }
    if starts_keyword(s, "CALL") {
        return translate_call(s, translator);
    }
    Ok(Translated::Query(s.to_string()))
}

// ───────────────────────────── helpers ─────────────────────────────

fn is_extension_statement(s: &str) -> bool {
    let up = s.to_uppercase();
    (up.starts_with("INSTALL") || up.starts_with("UNINSTALL") || up.starts_with("LOAD EXTENSION"))
        && !up.starts_with("LOAD FROM")
}

/// Read `name` or `` `name` ``; returns (name, rest). If the token is not an
/// identifier, returns the whole string as `name` with empty rest.
fn read_identifier(s: &str) -> (String, &str) {
    let s = s.trim_start();
    if let Some(rest) = s.strip_prefix('`') {
        if let Some(end) = rest.find('`') {
            return (rest[..end].to_string(), &rest[end + 1..]);
        }
        return (rest.to_string(), "");
    }
    let end = s
        .char_indices()
        .take_while(|(_, c)| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .map(|(i, _)| i)
        .last()
        .map(|i| i + 1)
        .unwrap_or(0);
    if end == 0 {
        (s.to_string(), "")
    } else {
        let first = s.chars().next().unwrap();
        if first.is_ascii_alphabetic() || first == '_' {
            (s[..end].to_string(), &s[end..])
        } else {
            (s.to_string(), "")
        }
    }
}

fn starts_keyword(s: &str, kw: &str) -> bool {
    let up = s.to_uppercase();
    if up.len() < kw.len() {
        return false;
    }
    up.starts_with(kw) && up.as_bytes().get(kw.len()).map_or(true, |b| b.is_ascii_whitespace())
}

fn skip_keyword<'a>(s: &'a str, kw: &str) -> &'a str {
    s.get(kw.len()..).unwrap_or("").trim_start()
}

/// Find the index of the closing paren matching the one at `open` (which must
/// be `'('`). Returns `None` if unbalanced.
fn find_matching_paren(s: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut in_backtick = false;
    for (i, ch) in s.char_indices().skip(open) {
        match ch {
            '\'' if !in_double && !in_backtick => in_single = !in_single,
            '"' if !in_single && !in_backtick => in_double = !in_double,
            '`' if !in_single && !in_double => in_backtick = !in_backtick,
            '(' if !in_single && !in_double && !in_backtick => depth += 1,
            ')' if !in_single && !in_double && !in_backtick => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Split on `sep` at paren-depth 0, respecting quotes.
fn split_top_level<'a>(s: &'a str, sep: char) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut in_single = false;
    let mut in_double = false;
    let mut in_backtick = false;
    let mut start = 0usize;
    for (i, ch) in s.char_indices() {
        match ch {
            '\'' if !in_double && !in_backtick => in_single = !in_single,
            '"' if !in_single && !in_backtick => in_double = !in_double,
            '`' if !in_single && !in_double => in_backtick = !in_backtick,
            '(' if !in_single && !in_double && !in_backtick => depth += 1,
            ')' if !in_single && !in_double && !in_backtick => depth -= 1,
            c if c == sep && depth == 0 && !in_single && !in_double && !in_backtick => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('\'') && s.ends_with('\'') {
        s[1..s.len() - 1].to_string()
    } else if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s[1..s.len() - 1].to_string()
    } else if s.len() >= 2 && s.starts_with('`') && s.ends_with('`') {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

/// Extract dims from a type token like `FLOAT[384]` / `FLOAT [ 384 ]`.
fn extract_dims(type_tokens: &str) -> Option<usize> {
    let compact: String = type_tokens.chars().filter(|c| !c.is_whitespace()).collect();
    let re =
        regex::Regex::new(r"(?i)^(float|double|int8|int16|int32|int64|uint8|uint16|uint32|uint64)\[(\d+)]").unwrap();
    re.captures(&compact).and_then(|c| c.get(2))?.as_str().parse().ok()
}

/// Replace `TYPE[n]` with `TYPE[]` throughout a DDL body (Akar grammar only
/// accepts empty brackets).
fn normalize_ddl_types(body: &str) -> String {
    let re = regex::Regex::new(r"\[[0-9]+]").unwrap();
    re.replace_all(body, "[]").to_string()
}

// ───────────────────────────── CREATE TABLE ─────────────────────────────

fn translate_create_table(s: &str, translator: &mut Translator) -> Result<Translated, String> {
    let (kind, s) = if starts_keyword(s, "CREATE NODE TABLE") {
        ("NODE", skip_keyword(s, "CREATE NODE TABLE"))
    } else {
        ("REL", skip_keyword(s, "CREATE REL TABLE"))
    };
    let (has_if_not_exists, s) = if starts_keyword(s, "IF NOT EXISTS") {
        (true, skip_keyword(s, "IF NOT EXISTS"))
    } else if starts_keyword(s, "IF EXISTS") {
        (false, skip_keyword(s, "IF EXISTS"))
    } else {
        (false, s)
    };

    let (name, rest) = read_identifier(s);
    if name.is_empty() {
        return Err(format!("Cannot translate CREATE {kind} TABLE: missing table name: {s}"));
    }
    let rest = rest.trim_start();
    let Some(open) = rest.find('(') else {
        return Err(format!(
            "Cannot translate CREATE {kind} TABLE: missing column list: {s}"
        ));
    };
    let Some(close) = find_matching_paren(rest, open) else {
        return Err(format!("Cannot translate CREATE {kind} TABLE: unbalanced parens: {s}"));
    };
    let body = rest[open + 1..close].trim();
    let tail = rest[close + 1..].trim();
    if !tail.is_empty() {
        return Err(format!(
            "Cannot translate CREATE {kind} TABLE: unexpected trailing content: {tail}"
        ));
    }

    let columns = parse_column_defs(body);
    if kind == "NODE" {
        translator.register_table(&name, columns);
    }

    let sql = if kind == "NODE" {
        format!("CREATE NODE TABLE {name} ({})", normalize_ddl_types(body))
    } else {
        // Akar's binder resolves rel-table FROM/TO without stripping
        // backticks, so rebuild those identifiers clean.
        let (src, dst, rel_columns) = parse_rel_from_to(body)?;
        let rel_columns = normalize_ddl_types(rel_columns.trim());
        if rel_columns.is_empty() {
            format!("CREATE REL TABLE {name} (FROM {src} TO {dst})")
        } else {
            format!("CREATE REL TABLE {name} (FROM {src} TO {dst}, {rel_columns})")
        }
    };
    if has_if_not_exists {
        Ok(Translated::CreateTableIfNotExists { table: name, sql })
    } else {
        Ok(Translated::Query(sql))
    }
}

/// Parse `FROM <src> TO <dst>[, columns]` from a rel-table body. Returns
/// (src, dst, remainder-after-dst) with backticks stripped.
fn parse_rel_from_to(body: &str) -> Result<(String, String, String), String> {
    let body = body.trim();
    let up = body.to_uppercase();
    if !up.starts_with("FROM") {
        return Err(format!("REL TABLE body must start with FROM, got: {body}"));
    }
    let after_from = body[4..].trim_start();
    let (src, rest) = read_identifier(after_from);
    if src.is_empty() {
        return Err(format!("REL TABLE missing FROM table: {body}"));
    }
    let rest = rest.trim_start();
    if !rest.to_uppercase().starts_with("TO") {
        return Err(format!("REL TABLE missing TO table: {body}"));
    }
    let after_to = rest[2..].trim_start();
    let (dst, rest) = read_identifier(after_to);
    if dst.is_empty() {
        return Err(format!("REL TABLE missing TO table: {body}"));
    }
    let rest = rest.trim_start().trim_start_matches(',').trim_start();
    Ok((src, dst, rest.to_string()))
}

fn parse_column_defs(body: &str) -> Vec<(String, Option<usize>)> {
    let mut cols = Vec::new();
    for seg in split_top_level(body, ',') {
        let seg = seg.trim();
        if seg.is_empty() {
            continue;
        }
        let up = seg.to_uppercase();
        if up.starts_with("PRIMARY KEY") || up.starts_with("FROM") || up.starts_with("TO") {
            continue;
        }
        let (name, rest) = read_identifier(seg);
        cols.push((name, extract_dims(rest)));
    }
    cols
}

// ───────────────────────────── DROP / ALTER ─────────────────────────────

fn translate_drop_table(s: &str) -> Result<Translated, String> {
    let s = skip_keyword(s, "DROP TABLE");
    let (has_if_exists, s) = if starts_keyword(s, "IF EXISTS") {
        (true, skip_keyword(s, "IF EXISTS"))
    } else {
        (false, s)
    };
    let (name, rest) = read_identifier(s);
    if name.is_empty() {
        return Err(format!("Cannot translate DROP TABLE: missing table name: {s}"));
    }
    if !rest.trim().is_empty() {
        return Err(format!(
            "Cannot translate DROP TABLE: unexpected trailing content: {rest}"
        ));
    }
    let sql = format!("DROP TABLE {name}");
    if has_if_exists {
        Ok(Translated::DropTableIfExists { table: name, sql })
    } else {
        Ok(Translated::Query(sql))
    }
}

fn translate_alter(s: &str) -> Result<Translated, String> {
    let s = skip_keyword(s, "ALTER TABLE");
    let (table, rest) = read_identifier(s);
    if table.is_empty() {
        return Err(format!("Cannot translate ALTER TABLE: missing table name: {s}"));
    }
    let rest = rest.trim_start();
    if !starts_keyword(rest, "ADD") {
        return Ok(Translated::Query(format!("ALTER TABLE {table} {rest}")));
    }
    let rest = skip_keyword(rest, "ADD");
    let (col, after_col) = read_identifier(rest);
    if col.is_empty() {
        return Err(format!(
            "Cannot translate ALTER TABLE {table} ADD: missing column name: {rest}"
        ));
    }
    // Strip a trailing `DEFAULT <literal>` — Akar's add_column has no DEFAULT.
    let re = regex::Regex::new(r"(?i)\s+DEFAULT\b").unwrap();
    let type_part = match re.find(after_col) {
        Some(m) => after_col[..m.start()].trim().to_string(),
        None => after_col.trim().to_string(),
    };
    if type_part.is_empty() {
        return Err(format!("Cannot translate ALTER TABLE {table} ADD {col}: missing type"));
    }
    Ok(Translated::Swallow(
        format!("ALTER TABLE {table} ADD {col} {type_part}"),
        ERR_ALREADY_EXISTS,
    ))
}

// ───────────────────────────── CREATE VECTOR INDEX ─────────────────────────────

/// Kuzu: `CREATE VECTOR INDEX [IF NOT EXISTS] name FOR (m:T) ON (m.col)
/// OPTIONS {...}` → Akar syntax + swallow "already exists".
fn translate_create_vector_index_kuzu(s: &str, translator: &mut Translator) -> Result<Translated, String> {
    let s = skip_keyword(s, "CREATE VECTOR INDEX");
    let (has_if_not_exists, s) = if starts_keyword(s, "IF NOT EXISTS") {
        (true, skip_keyword(s, "IF NOT EXISTS"))
    } else {
        (false, s)
    };
    let (name, rest) = read_identifier(s);
    if name.is_empty() {
        return Err(format!("Cannot translate CREATE VECTOR INDEX: missing index name: {s}"));
    }

    // Recognise the Kuzu `FOR (...) ON (...) OPTIONS {...}` shape; anything
    // else is already Akar dialect and passes through.
    if !rest.to_uppercase().contains("FOR") {
        if has_if_not_exists {
            return Err(format!(
                "CREATE VECTOR INDEX {name} without Kuzu FOR/ON/OPTIONS shape cannot be made idempotent"
            ));
        }
        return Ok(Translated::Query(s.trim().to_string()));
    }

    let (table, col) = parse_kuzu_for_on(rest)
        .ok_or_else(|| format!("Cannot parse Kuzu CREATE VECTOR INDEX {name}: expected FOR (v:T) ON (v.col)"))?;

    let dims = translator.table(&table).and_then(|t| t.dims_of(&col));
    let metric = "cosine"; // Kuzu default metric.
    let sql = match dims {
        Some(n) => format!("CREATE VECTOR INDEX {name} ON ({table}.{col}) WITH (metric={metric}, dims={n})"),
        None => format!("CREATE VECTOR INDEX {name} ON ({table}.{col}) WITH (metric={metric}, dims=0)"),
    };
    translator.register_vec_index(&table, &name, &col);
    Ok(Translated::Swallow(
        sql,
        if dims.is_some() {
            ERR_ALREADY_EXISTS
        } else {
            ERR_DIMS_ZERO
        },
    ))
}

/// Parse `FOR ( <var> : <T> ) ON ( <var> . <col> )` into (table, col).
fn parse_kuzu_for_on(s: &str) -> Option<(String, String)> {
    let up = s.to_uppercase();
    let for_start = up.find("FOR")?;
    let after_for = &s[for_start + 3..];
    let paren = after_for.find('(')?;
    let open = paren;
    let close = find_matching_paren(after_for, open)?;
    let for_body = after_for[open + 1..close].trim();
    let (_, for_tokens) = read_identifier(for_body);
    let for_rest = for_tokens.trim();
    let (table, _) = read_identifier(for_rest.trim_start().trim_start_matches(':').trim_start());

    let after_for_paren = &after_for[close + 1..];
    let on_start = after_for_paren.to_uppercase().find("ON")?;
    let on_seg = &after_for_paren[on_start + 2..];
    let on_open = on_seg.find('(')?;
    let on_close = find_matching_paren(on_seg, on_open)?;
    let on_body = on_seg[on_open + 1..on_close].trim();
    let (col_var, col_rest) = read_identifier(on_body);
    let (col, _) = read_identifier(col_rest.trim_start().trim_start_matches('.').trim_start());
    let _ = col_var;

    if table.is_empty() || col.is_empty() {
        return None;
    }
    Some((table, col))
}

// ───────────────────────────── CALL statements ─────────────────────────────

fn translate_call(s: &str, translator: &mut Translator) -> Result<Translated, String> {
    let s = skip_keyword(s, "CALL").trim_start();
    let (func, rest) = read_call_name(s);
    let func_lc = func.to_lowercase();
    let (args_str, tail) = extract_call_args(rest)?;

    let args: Vec<&str> = split_top_level(&args_str, ',').into_iter().map(str::trim).collect();

    match func_lc.as_str() {
        "create_vector_index" => translate_call_create_vector_index(&func, &args, translator),
        "drop_vector_index" => translate_call_drop_vector_index(&func, &args),
        "query_vector_index" => translate_call_query_vector_index(&func, &args, &tail, translator),
        _ => Ok(Translated::Query(format!("CALL {func}({args_str}){tail}"))),
    }
}

fn read_call_name(s: &str) -> (String, &str) {
    let s = s.trim_start();
    let (name, rest) = read_identifier(s);
    let mut full = name;
    let mut rest = rest;
    while rest.trim_start().starts_with('.') {
        let r = rest.trim_start();
        let (seg, after) = read_identifier(&r[1..]);
        if seg.is_empty() {
            break;
        }
        full.push('.');
        full.push_str(&seg);
        rest = after;
    }
    (full, rest)
}

/// Extract the parenthesised argument list and the trailing text after `)`.
fn extract_call_args(s: &str) -> Result<(String, String), String> {
    let s = s.trim_start();
    let Some(open) = s.find('(') else {
        return Err(format!("CALL is missing '(': {s}"));
    };
    let Some(close) = find_matching_paren(s, open) else {
        return Err(format!("CALL has unbalanced parens: {s}"));
    };
    let args = s[open + 1..close].trim().to_string();
    let tail = s[close + 1..].trim().to_string();
    Ok((args, tail))
}

fn translate_call_create_vector_index(
    func: &str,
    args: &[&str],
    translator: &mut Translator,
) -> Result<Translated, String> {
    let expect = |i: usize| -> Result<String, String> {
        args.get(i)
            .map(|a| unquote(a))
            .filter(|a| !a.is_empty())
            .ok_or_else(|| format!("CALL {func}: expected positional string argument at index {i}"))
    };
    let table = expect(0)?;
    let index_name = expect(1)?;
    let col = expect(2)?;

    let metric = args
        .iter()
        .skip(3)
        .find(|a| a.to_uppercase().starts_with("METRIC"))
        .and_then(|a| {
            let idx = a.find(':').or_else(|| a.find('='))?;
            let v = unquote(&a[idx + 1..].trim_start().trim_start_matches('=').trim_start());
            Some(v)
        })
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "cosine".to_string());

    let dims = translator.table(&table).and_then(|t| t.dims_of(&col));
    translator.register_vec_index(&table, &index_name, &col);
    let sql = match dims {
        Some(n) => format!("CREATE VECTOR INDEX {index_name} ON ({table}.{col}) WITH (metric={metric}, dims={n})"),
        None => format!("CREATE VECTOR INDEX {index_name} ON ({table}.{col}) WITH (metric={metric}, dims=0)"),
    };
    Ok(Translated::Swallow(
        sql,
        if dims.is_some() {
            ERR_ALREADY_EXISTS
        } else {
            ERR_DIMS_ZERO
        },
    ))
}

fn translate_call_drop_vector_index(func: &str, args: &[&str]) -> Result<Translated, String> {
    let expect = |i: usize| -> Result<String, String> {
        args.get(i)
            .map(|a| unquote(a))
            .filter(|a| !a.is_empty())
            .ok_or_else(|| format!("CALL {func}: expected positional string argument at index {i}"))
    };
    let table = expect(0)?;
    let index_name = expect(1)?;
    Ok(Translated::Swallow(
        format!("DROP INDEX {index_name} ON {table}"),
        ERR_NOT_FOUND,
    ))
}

fn translate_call_query_vector_index(
    func: &str,
    args: &[&str],
    tail: &str,
    translator: &mut Translator,
) -> Result<Translated, String> {
    let expect = |i: usize| -> Result<String, String> {
        args.get(i)
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty())
            .ok_or_else(|| format!("CALL {func}: expected argument at index {i}"))
    };
    let table = unquote(&expect(0)?);
    let index_name = unquote(&expect(1)?);
    let vec_expr = expect(2)?;
    let limit_expr = expect(3)?;

    // tail: `RETURN node, distance [WHERE ...]`
    let (where_sql, _) = split_where(tail);

    // The node column for the cosine projection: prefer the column the index
    // was created on, else the FLOAT[n] column, else `embedding`.
    let col = translator
        .vec_index_col(&table, &index_name)
        .map(str::to_string)
        .or_else(|| {
            translator
                .table(&table)
                .and_then(|t| t.vector_column().map(str::to_string))
        })
        .unwrap_or_else(|| "embedding".to_string());

    Ok(Translated::VectorQuery {
        table,
        index_name,
        vec_expr,
        limit_expr,
        vec_col: col,
        where_sql,
    })
}

/// Split `... RETURN node, distance WHERE <cond>` into the leading return
/// text and the optional `WHERE <cond>` clause (with the keyword included).
fn split_where(s: &str) -> (Option<String>, &str) {
    // Find WHERE at paren depth 0, outside quotes.
    let mut depth = 0i32;
    let mut in_single = false;
    let mut in_double = false;
    let mut in_backtick = false;
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let ch = bytes[i] as char;
        match ch {
            '\'' if !in_double && !in_backtick => in_single = !in_single,
            '"' if !in_single && !in_backtick => in_double = !in_double,
            '`' if !in_single && !in_double => in_backtick = !in_backtick,
            '(' if !in_single && !in_double && !in_backtick => depth += 1,
            ')' if !in_single && !in_double && !in_backtick => depth -= 1,
            _ => {
                if depth == 0 && !in_single && !in_double && !in_backtick && ch.is_ascii_alphabetic() {
                    if s[i..].to_uppercase().starts_with("WHERE")
                        && s[i..]
                            .as_bytes()
                            .get(5)
                            .map_or(true, |b| b.is_ascii_whitespace() || *b == b'(')
                    {
                        let cond = s[i + 5..].trim().to_string();
                        return (Some(format!("WHERE {cond}")), &s[..i]);
                    }
                }
            }
        }
        i += 1;
    }
    (None, s)
}

// ───────────────────────────── tests ─────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn translator() -> Translator {
        Translator::new()
    }

    #[test]
    fn split_statements_multiple() {
        assert_eq!(
            split_statements("INSTALL vector; LOAD EXTENSION vector"),
            vec!["INSTALL vector", "LOAD EXTENSION vector"]
        );
        assert_eq!(
            split_statements("CREATE (m:Memory {content: 'a;b'}); RETURN 1"),
            vec!["CREATE (m:Memory {content: 'a;b'})", "RETURN 1"]
        );
        assert_eq!(split_statements("   ;  ; "), Vec::<String>::new());
    }

    #[test]
    fn extension_stmts_are_noop() {
        let mut t = translator();
        for s in [
            "INSTALL vector",
            "INSTALL EXTENSION vector",
            "LOAD EXTENSION vector",
            "UNINSTALL vector",
            "UNINSTALL EXTENSION vector",
        ] {
            assert!(matches!(translate(s, &mut t).unwrap(), Translated::NoOp), "{s}");
        }
        // LOAD FROM must NOT be a no-op (multi-db).
        assert!(matches!(
            translate("LOAD FROM 'file.parquet' (format=parquet)", &mut t).unwrap(),
            Translated::Query(_)
        ));
    }

    #[test]
    fn create_node_table_if_not_exists_registers_dims() {
        let mut t = translator();
        let stmt = "CREATE NODE TABLE IF NOT EXISTS Memory (id INT64, embedding FLOAT[384], salience DOUBLE, PRIMARY KEY (id))";
        match translate(stmt, &mut t).unwrap() {
            Translated::CreateTableIfNotExists { table, sql } => {
                assert_eq!(table, "Memory");
                assert!(sql.contains("CREATE NODE TABLE Memory"));
                assert!(sql.contains("embedding FLOAT[]"));
                assert!(!sql.contains("384"));
            }
            other => panic!("unexpected: {other:?}"),
        }
        let schema = t.table("Memory").unwrap();
        assert_eq!(schema.dims_of("embedding"), Some(384));
        assert_eq!(schema.dims_of("salience"), None);
        assert_eq!(schema.column_names(), vec!["id", "embedding", "salience"]);
    }

    #[test]
    fn create_rel_table_if_not_exists() {
        let mut t = translator();
        let stmt = "CREATE REL TABLE IF NOT EXISTS HAS_SCHEMA (FROM `Database` TO `Schema`)";
        match translate(stmt, &mut t).unwrap() {
            Translated::CreateTableIfNotExists { table, sql } => {
                assert_eq!(table, "HAS_SCHEMA");
                // FROM/TO backticks must be stripped: Akar's binder does not.
                assert_eq!(sql, "CREATE REL TABLE HAS_SCHEMA (FROM Database TO Schema)");
            }
            other => panic!("unexpected: {other:?}"),
        }

        let stmt2 = "CREATE REL TABLE IF NOT EXISTS BELONGS (FROM `A` TO `B`, weight DOUBLE)";
        match translate(stmt2, &mut t).unwrap() {
            Translated::CreateTableIfNotExists { sql, .. } => {
                assert_eq!(sql, "CREATE REL TABLE BELONGS (FROM A TO B, weight DOUBLE)");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn drop_table_if_exists() {
        let mut t = translator();
        match translate("DROP TABLE IF EXISTS Memory", &mut t).unwrap() {
            Translated::DropTableIfExists { table, sql } => {
                assert_eq!(table, "Memory");
                assert_eq!(sql, "DROP TABLE Memory");
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert!(matches!(
            translate("DROP TABLE Memory", &mut t).unwrap(),
            Translated::Query(sql) if sql == "DROP TABLE Memory"
        ));
    }

    #[test]
    fn alter_add_default_stripped() {
        let mut t = translator();
        match translate("ALTER TABLE Memory ADD protected BOOLEAN DEFAULT false", &mut t).unwrap() {
            Translated::Swallow(sql, needles) => {
                assert_eq!(sql, "ALTER TABLE Memory ADD protected BOOLEAN");
                assert_eq!(needles, ERR_ALREADY_EXISTS);
            }
            other => panic!("unexpected: {other:?}"),
        }
        match translate("ALTER TABLE Memory ADD dae_self_weight DOUBLE DEFAULT 0.4", &mut t).unwrap() {
            Translated::Swallow(sql, _) => assert_eq!(sql, "ALTER TABLE Memory ADD dae_self_weight DOUBLE"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn call_create_vector_index() {
        let mut t = translator();
        let mut stmt = "CREATE NODE TABLE IF NOT EXISTS Memory (embedding FLOAT[384], PRIMARY KEY (id))";
        let _ = translate(stmt, &mut t).unwrap();
        stmt = "CALL CREATE_VECTOR_INDEX('Memory', 'mem_vec', 'embedding', metric := 'cosine')";
        match translate(stmt, &mut t).unwrap() {
            Translated::Swallow(sql, _) => {
                assert_eq!(
                    sql,
                    "CREATE VECTOR INDEX mem_vec ON (Memory.embedding) WITH (metric=cosine, dims=384)"
                )
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert_eq!(t.vec_index_col("Memory", "mem_vec"), Some("embedding"));
    }

    #[test]
    fn call_drop_vector_index() {
        let mut t = translator();
        match translate("CALL DROP_VECTOR_INDEX('Memory', 'mem_vec')", &mut t).unwrap() {
            Translated::Swallow(sql, _) => assert_eq!(sql, "DROP INDEX mem_vec ON Memory"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn call_query_vector_index() {
        let mut t = translator();
        let mut stmt = "CREATE NODE TABLE IF NOT EXISTS Memory (id INT64, embedding FLOAT[384], prof STRING, session_id STRING, PRIMARY KEY (id))";
        let _ = translate(stmt, &mut t).unwrap();
        let _ = translate(
            "CALL CREATE_VECTOR_INDEX('Memory', 'mem_vec', 'embedding', metric := 'cosine')",
            &mut t,
        )
        .unwrap();
        stmt = "CALL QUERY_VECTOR_INDEX('Memory', 'mem_vec', $query_vec, $limit) RETURN node, distance WHERE (node.prof IS NULL OR node.prof = $prof)";
        match translate(stmt, &mut t).unwrap() {
            Translated::VectorQuery {
                table,
                index_name,
                vec_expr,
                limit_expr,
                vec_col,
                where_sql,
            } => {
                assert_eq!(table, "Memory");
                assert_eq!(index_name, "mem_vec");
                assert_eq!(vec_expr, "$query_vec");
                assert_eq!(limit_expr, "$limit");
                assert_eq!(vec_col, "embedding");
                assert_eq!(where_sql.unwrap(), "WHERE (node.prof IS NULL OR node.prof = $prof)");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn kuzu_create_vector_index_syntax() {
        let mut t = translator();
        let _ = translate(
            "CREATE NODE TABLE IF NOT EXISTS Memory (embedding FLOAT[384], PRIMARY KEY (id))",
            &mut t,
        )
        .unwrap();
        let stmt = "CREATE VECTOR INDEX IF NOT EXISTS memory_emb_idx FOR (m:Memory) ON (m.embedding) OPTIONS {index_list: [{efc: 128, M: 16}]}";
        match translate(stmt, &mut t).unwrap() {
            Translated::Swallow(sql, _) => {
                assert_eq!(
                    sql,
                    "CREATE VECTOR INDEX memory_emb_idx ON (Memory.embedding) WITH (metric=cosine, dims=384)"
                )
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn akar_syntax_passes_through() {
        let mut t = translator();
        for s in [
            "MATCH (m:Memory) RETURN m.id",
            "CREATE (m:Memory {id: 1})",
            "EXPORT DATABASE \"x\" (format=\"parquet\")",
            "CHECKPOINT",
            "CALL show_tables()",
        ] {
            assert!(matches!(translate(s, &mut t).unwrap(), Translated::Query(_)), "{s}");
        }
    }
}
