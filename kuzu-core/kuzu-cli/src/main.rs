//! Kuzu CLI — interactive and script-mode Cypher query shell.
//!
//! Usage:
//!   kuzu-cli [database_path]
//!
//! If no path is given, runs in `:memory:` mode.
//!
//! Interactive features:
//!   - Multi-line input with `;` termination
//!   - Command history via rustyline (↑↓ arrows)
//!   - Tab completion for keywords and table names
//!   - `.mode` command (table, csv, json, line, column)
//!   - `.import` / `.export` for CSV
//!   - `.tables`, `.schema`, `.help`

#![allow(unused_imports, dead_code, unused_must_use)]

use kuzu_catalog::Catalog;
use kuzu_main::{Connection, Database, SystemConfig};
use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::{CmdKind, Highlighter};
use rustyline::hint::Hinter;
use rustyline::validate::{ValidationContext, ValidationResult, Validator};
use rustyline::{Cmd, CompletionType, Config, Context, EditMode, Editor, KeyEvent};
use std::borrow::Cow;
use std::io::{self, BufRead, Write};
use std::sync::{Arc, Mutex};

// ==================== Output Modes ====================

#[derive(Debug, Clone, Copy, PartialEq)]
enum OutputMode {
    Table,
    Csv,
    Json,
    Line,
    Column,
    Box,
    Html,
    Latex,
}

impl OutputMode {
    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "table" => Some(Self::Table),
            "csv" => Some(Self::Csv),
            "json" => Some(Self::Json),
            "line" => Some(Self::Line),
            "column" => Some(Self::Column),
            "box" => Some(Self::Box),
            "html" => Some(Self::Html),
            "latex" | "tex" => Some(Self::Latex),
            _ => None,
        }
    }
}

// ==================== CLI State ====================

struct CliState {
    mode: OutputMode,
    conn: Connection,
    catalog: Arc<Mutex<Catalog>>,
}

impl CliState {
    fn new(db_path: &str) -> Result<Self, String> {
        let db = Arc::new(Database::new(db_path, SystemConfig::default())?);
        let catalog = db.catalog();
        let conn = Connection::new(&db);
        Ok(Self {
            mode: OutputMode::Box,
            conn,
            catalog,
        })
    }

    fn table_names(&self) -> Vec<String> {
        self.catalog
            .lock()
            .unwrap()
            .all_entries()
            .map(|e| e.name().to_string())
            .collect()
    }

    fn execute_dot_command(&mut self, cmd: &str, output: &mut dyn Write) -> Result<(), String> {
        let parts: Vec<&str> = cmd.trim_start_matches('.').split_whitespace().collect();
        if parts.is_empty() {
            return Ok(());
        }
        match parts[0].to_lowercase().as_str() {
            "exit" | "quit" => return Err("__EXIT__".into()),
            "help" => {
                writeln!(output, "Kuzu CLI commands:").ok();
                writeln!(output, "  .mode <mode>    Output: table|csv|json|line|column|box|html|latex").ok();
                writeln!(output, "  .tables         List tables").ok();
                writeln!(output, "  .schema         Show schemas").ok();
                writeln!(output, "  .import f t     CSV import: file table").ok();
                writeln!(output, "  .export f q     CSV export: file query").ok();
                writeln!(output, "  .help           This help").ok();
                writeln!(output, "  .exit / .quit   Exit").ok();
            }
            "tables" => {
                let names = self.table_names();
                if names.is_empty() {
                    writeln!(output, "No tables.").ok();
                } else {
                    for n in names {
                        writeln!(output, "  {n}").ok();
                    }
                }
            }
            "schema" => {
                let cat = self.catalog.lock().unwrap();
                let entries: Vec<_> = cat.all_entries().collect();
                if entries.is_empty() {
                    writeln!(output, "No tables.").ok();
                } else {
                    for e in entries {
                        let tt = if e.is_node_table() { "NODE" } else { "REL" };
                        writeln!(output, "TABLE {} ({})", e.name(), tt).ok();
                        for c in e.columns() {
                            let pk = if c.is_primary_key { " PK" } else { "" };
                            writeln!(output, "  {}: {:?}{}", c.name, c.logical_type, pk).ok();
                        }
                    }
                }
            }
            "mode" => {
                if parts.len() < 2 {
                    writeln!(output, "Usage: .mode <table|csv|json|line|column|box|html|latex>").ok();
                    writeln!(output, "Current: {:?}", self.mode).ok();
                } else if let Some(m) = OutputMode::from_str(parts[1]) {
                    self.mode = m;
                    writeln!(output, "Mode set to {:?}", m).ok();
                } else {
                    writeln!(output, "Unknown: {}. Options: table, csv, json, line, column, box, html, latex", parts[1]).ok();
                }
            }
            "import" => {
                if parts.len() < 3 {
                    writeln!(output, "Usage: .import <file.csv> <table>").ok();
                } else {
                    let sql = format!("COPY {} FROM '{}' (HEADER true)", parts[2], parts[1].replace('\\', "/"));
                    match self.conn.query(&sql) {
                        Ok(r) => {
                            let _ = writeln!(output, "{}", r.result_summary());
                        }
                        Err(e) => {
                            let _ = writeln!(output, "Error: {e}");
                        }
                    }
                }
            }
            "export" => {
                if parts.len() < 3 {
                    writeln!(output, "Usage: .export <file.csv> <query>").ok();
                } else {
                    let q = parts[2..].join(" ");
                    match self.conn.query(&q) {
                        Ok(r) => {
                            if let Err(e) = export_to_csv(&r, parts[1]) {
                                let _ = writeln!(output, "Export error: {e}");
                            } else {
                                let _ = writeln!(output, "Exported to {}", parts[1]);
                            }
                        }
                        Err(e) => {
                            let _ = writeln!(output, "Query error: {e}");
                        }
                    }
                }
            }
            other => {
                let _ = writeln!(output, "Unknown: '{other}'. Type .help");
            }
        }
        Ok(())
    }
}

// ==================== Global State Bridge ====================

use std::sync::Mutex as StdMutex;
static GLOBAL_STATE: std::sync::LazyLock<StdMutex<Option<CliState>>> = std::sync::LazyLock::new(|| StdMutex::new(None));

// ==================== Rustyline Completer ====================

struct CypherCompleter;

impl Completer for CypherCompleter {
    type Candidate = Pair;
    fn complete(&self, line: &str, pos: usize, _ctx: &Context<'_>) -> Result<(usize, Vec<Pair>), ReadlineError> {
        let before = &line[..pos];
        let word = before.split_whitespace().last().unwrap_or("");
        if word.is_empty() {
            return Ok((pos, vec![]));
        }
        let mut candidates = Vec::new();
        let keywords = [
            "MATCH", "RETURN", "WHERE", "CREATE", "DELETE", "SET", "MERGE", "CALL", "FOREACH", "IN", "AS", "ORDER",
            "BY", "LIMIT", "WITH", "UNWIND", "OPTIONAL", "EXISTS", "UNION", "ALL", "AND", "OR", "NOT", "TRUE", "FALSE",
            "NULL", "ON",
        ];
        for kw in &keywords {
            if kw.starts_with(&word.to_uppercase()) {
                candidates.push(Pair {
                    display: kw.to_string(),
                    replacement: kw.to_string(),
                });
            }
        }
        if let Ok(state) = GLOBAL_STATE.lock()
            && let Some(ref s) = *state
        {
            for name in s.table_names() {
                if name.to_lowercase().starts_with(&word.to_lowercase()) {
                    candidates.push(Pair {
                        display: name.clone(),
                        replacement: name,
                    });
                }
            }
        }
        Ok((pos - word.len(), candidates))
    }
}

impl Hinter for CypherCompleter {
    type Hint = String;
    fn hint(&self, _line: &str, _pos: usize, _ctx: &Context<'_>) -> Option<String> {
        None
    }
}

impl Highlighter for CypherCompleter {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        Cow::Borrowed(line)
    }
    fn highlight_char(&self, _line: &str, _pos: usize, _kind: CmdKind) -> bool {
        false
    }
}

impl Validator for CypherCompleter {
    fn validate(&self, ctx: &mut ValidationContext) -> Result<ValidationResult, ReadlineError> {
        let input = ctx.input().trim();
        if input.starts_with('.') || input.ends_with(';') {
            Ok(ValidationResult::Valid(None))
        } else {
            Ok(ValidationResult::Incomplete)
        }
    }
}

impl rustyline::Helper for CypherCompleter {}

// ==================== Main ====================

fn main() {
    tracing_subscriber::fmt().with_max_level(tracing::Level::WARN).init();

    let args: Vec<String> = std::env::args().collect();
    let db_path = if args.len() > 1 {
        args[1].clone()
    } else {
        ":memory:".to_string()
    };

    let state = match CliState::new(&db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    };

    // Store global state reference
    *GLOBAL_STATE.lock().unwrap() = Some(state);

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    // Try rustyline REPL if terminal available
    let is_tty = cfg!(windows) || atty_check();
    if is_tty && atty_check() {
        run_repl_rustyline(&mut stdout);
    } else {
        let state_ref = GLOBAL_STATE.lock().unwrap();
        let state = state_ref.as_ref().unwrap();
        run_script(&state.conn, stdin.lock(), &mut stdout);
    }
}

fn atty_check() -> bool {
    // Simple heuristic: if TERM is set and not "dumb", assume interactive
    std::env::var("TERM").is_ok_and(|t| t != "dumb") || std::env::var("CI").is_err()
}

// ==================== Rustyline REPL ====================

fn run_repl_rustyline(output: &mut dyn Write) {
    let config = Config::builder()
        .history_ignore_space(true)
        .completion_type(CompletionType::List)
        .edit_mode(EditMode::Emacs)
        .build();

    let mut rl = match Editor::with_config(config) {
        Ok(e) => e,
        Err(e) => {
            let _ = writeln!(output, "Editor error: {e}");
            return;
        }
    };
    rl.set_helper(Some(CypherCompleter));

    let history_file = get_history_path();
    let _ = rl.load_history(&history_file);

    writeln!(output, "Kuzu CLI v{}", env!("CARGO_PKG_VERSION")).ok();
    writeln!(output, "Enter queries (end with ;). Type .help").ok();
    writeln!(output).ok();

    loop {
        let readline = rl.readline("kuzu> ");
        match readline {
            Ok(line) => {
                let trimmed = line.trim().to_string();
                if trimmed.is_empty() {
                    continue;
                }

                if trimmed.starts_with('.') {
                    rl.add_history_entry(trimmed.as_str());
                    let mut state = GLOBAL_STATE.lock().unwrap();
                    if let Some(ref mut s) = *state {
                        match s.execute_dot_command(&trimmed, output) {
                            Ok(_) => {}
                            Err(e) if e == "__EXIT__" => {
                                let _ = writeln!(output, "Bye!");
                                break;
                            }
                            Err(e) => {
                                let _ = writeln!(output, "Error: {e}");
                            }
                        }
                    }
                    continue;
                }

                let mut full_query = trimmed;
                if !full_query.ends_with(';') {
                    loop {
                        let line2 = rl.readline("  ..> ");
                        match line2 {
                            Ok(l) => {
                                full_query.push(' ');
                                full_query.push_str(l.trim());
                                if full_query.ends_with(';') {
                                    break;
                                }
                            }
                            Err(ReadlineError::Interrupted) => {
                                let _ = writeln!(output, "^C");
                                full_query.clear();
                                break;
                            }
                            Err(ReadlineError::Eof) => break,
                            Err(e) => {
                                let _ = writeln!(output, "Error: {e:?}");
                                break;
                            }
                        }
                    }
                    if full_query.is_empty() {
                        continue;
                    }
                }

                rl.add_history_entry(full_query.as_str());
                let clean = full_query.trim_end_matches(';').trim();
                execute_query(clean, output);
            }
            Err(ReadlineError::Interrupted) => {
                let _ = writeln!(output, "^C");
                continue;
            }
            Err(ReadlineError::Eof) => {
                let _ = writeln!(output, "Bye!");
                break;
            }
            Err(e) => {
                let _ = writeln!(output, "Error: {e:?}");
                break;
            }
        }
    }
    let _ = rl.save_history(&history_file);
}

fn get_history_path() -> String {
    if let Some(dir) = dirs::data_dir() {
        let kd = dir.join("kuzu");
        let _ = std::fs::create_dir_all(&kd);
        kd.join("history.txt").to_string_lossy().to_string()
    } else {
        ".kuzu_history".into()
    }
}

// ==================== Query Execution ====================

fn execute_query(query: &str, output: &mut dyn Write) {
    let state_ref = GLOBAL_STATE.lock().unwrap();
    let state = match state_ref.as_ref() {
        Some(s) => s,
        None => return,
    };

    match query.to_lowercase().trim() {
        "exit" | "quit" => {
            let _ = writeln!(output, "Bye!");
            std::process::exit(0);
        }
        "help" => {
            drop(state_ref);
            let mut s = GLOBAL_STATE.lock().unwrap();
            if let Some(ref mut ss) = *s {
                let _ = ss.execute_dot_command(".help", output);
            }
            return;
        }
        _ => {}
    }

    let mode = state.mode;
    match state.conn.query(query) {
        Ok(result) => {
            if result.is_success() {
                format_output(&result, mode, output);
                if let Some(msg) = &result.message {
                    let _ = writeln!(output, "{msg}");
                }
            } else {
                let _ = writeln!(
                    output,
                    "Error: {}",
                    result.error_message.as_deref().unwrap_or("Unknown")
                );
            }
        }
        Err(e) => {
            let _ = writeln!(output, "Error: {e}");
        }
    }
}

// ==================== Output Formatting ====================

fn format_output(result: &kuzu_main::QueryResult, mode: OutputMode, output: &mut dyn Write) {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for chunk in &result.chunks {
        for r in 0..chunk.size {
            rows.push((0..chunk.fields.len()).map(|col| fmt_val(&chunk.get_value(col, r).unwrap_or(kuzu_common::types::Value::Null))).collect());
        }
    }
    if rows.is_empty() || rows[0].is_empty() {
        writeln!(output, "(empty)").ok();
        return;
    }

    let ncols = rows[0].len();
    let headers: Vec<String> = (0..ncols).map(|i| format!("col_{}", i)).collect();

    match mode {
        OutputMode::Table => {
            let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
            for row in &rows {
                for (i, v) in row.iter().enumerate() {
                    if i < widths.len() {
                        widths[i] = widths[i].max(v.len());
                    }
                }
            }
            let hdr = format!(
                "| {} |",
                headers
                    .iter()
                    .enumerate()
                    .map(|(i, h)| format!("{:w$}", h, w = widths[i]))
                    .collect::<Vec<_>>()
                    .join(" | ")
            );
            let sep = format!(
                "+{}+",
                widths.iter().map(|w| "-".repeat(*w)).collect::<Vec<_>>().join("-+-")
            );
            let _ = writeln!(output, "{sep}");
            let _ = writeln!(output, "{hdr}");
            let _ = writeln!(output, "{sep}");
            for row in &rows {
                let line = format!(
                    "| {} |",
                    row.iter()
                        .enumerate()
                        .map(|(i, v)| format!("{:w$}", v, w = widths[i]))
                        .collect::<Vec<_>>()
                        .join(" | ")
                );
                writeln!(output, "{line}").ok();
            }
            writeln!(output, "{sep}").ok();
            writeln!(output, "({} rows)", rows.len()).ok();
        }
        OutputMode::Box => {
            let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
            for row in &rows {
                for (i, v) in row.iter().enumerate() {
                    if i < widths.len() {
                        widths[i] = widths[i].max(v.len());
                    }
                }
            }
            
            let top_border = format!(
                "┌{}┐",
                widths.iter().map(|w| "─".repeat(*w + 2)).collect::<Vec<_>>().join("┬")
            );
            let hdr = format!(
                "│ {} │",
                headers
                    .iter()
                    .enumerate()
                    .map(|(i, h)| format!("{:w$}", h, w = widths[i]))
                    .collect::<Vec<_>>()
                    .join(" │ ")
            );
            let mid_border = format!(
                "├{}┤",
                widths.iter().map(|w| "─".repeat(*w + 2)).collect::<Vec<_>>().join("┼")
            );
            let bot_border = format!(
                "└{}┘",
                widths.iter().map(|w| "─".repeat(*w + 2)).collect::<Vec<_>>().join("┴")
            );

            let _ = writeln!(output, "{top_border}");
            let _ = writeln!(output, "{hdr}");
            let _ = writeln!(output, "{mid_border}");
            for row in &rows {
                let line = format!(
                    "│ {} │",
                    row.iter()
                        .enumerate()
                        .map(|(i, v)| format!("{:w$}", v, w = widths[i]))
                        .collect::<Vec<_>>()
                        .join(" │ ")
                );
                writeln!(output, "{line}").ok();
            }
            writeln!(output, "{bot_border}").ok();
            writeln!(output, "({} rows)", rows.len()).ok();
        }
        OutputMode::Csv => {
            for row in &rows {
                let csv: Vec<String> = row
                    .iter()
                    .map(|v| {
                        if v.contains(',') || v.contains('"') {
                            format!("\"{}\"", v.replace('"', "\"\""))
                        } else {
                            v.clone()
                        }
                    })
                    .collect();
                writeln!(output, "{}", csv.join(",")).ok();
            }
        }
        OutputMode::Json => {
            writeln!(output, "[").ok();
            for (ri, row) in rows.iter().enumerate() {
                let objs: Vec<String> = row
                    .iter()
                    .enumerate()
                    .map(|(i, v)| format!("\"col_{}\": \"{}\"", i, v.replace('"', "\\\"")))
                    .collect();
                let comma = if ri < rows.len() - 1 { "," } else { "" };
                writeln!(output, "  {{{}}}{}", objs.join(", "), comma).ok();
            }
            writeln!(output, "]").ok();
        }
        OutputMode::Line => {
            for (ri, row) in rows.iter().enumerate() {
                for (i, v) in row.iter().enumerate() {
                    writeln!(output, "  {}: {}", headers[i], v).ok();
                }
                if ri < rows.len() - 1 {
                    writeln!(output).ok();
                }
            }
            writeln!(output, "({} rows)", rows.len()).ok();
        }
        OutputMode::Html => {
            writeln!(output, "<table>").ok();
            writeln!(output, "  <thead>").ok();
            write!(output, "    <tr>").ok();
            for h in &headers {
                write!(output, "<th>{}</th>", h).ok();
            }
            writeln!(output, "</tr>").ok();
            writeln!(output, "  </thead>").ok();
            writeln!(output, "  <tbody>").ok();
            for row in &rows {
                write!(output, "    <tr>").ok();
                for v in row {
                    write!(output, "<td>{}</td>", v).ok();
                }
                writeln!(output, "</tr>").ok();
            }
            writeln!(output, "  </tbody>").ok();
            writeln!(output, "</table>").ok();
            writeln!(output, "<!-- {} rows -->", rows.len()).ok();
        }
        OutputMode::Latex => {
            writeln!(output, "\\begin{{tabular}}{{{}}}", "c ".repeat(ncols).trim()).ok();
            writeln!(
                output,
                "  {} \\\\",
                headers
                    .iter()
                    .map(|h| format!("\\textbf{{{}}}", h))
                    .collect::<Vec<_>>()
                    .join(" & ")
            )
            .ok();
            writeln!(output, "  \\hline").ok();
            for row in &rows {
                writeln!(output, "  {} \\\\", row.join(" & ")).ok();
            }
            writeln!(output, "\\end{{tabular}}").ok();
            writeln!(output, "% {} rows", rows.len()).ok();
        }
        OutputMode::Column => {
            for i in 0..ncols {
                writeln!(output, "{}:", headers[i]).ok();
                for row in &rows {
                    if let Some(v) = row.get(i) {
                        writeln!(output, "  {v}").ok();
                    }
                }
                if i < ncols - 1 {
                    writeln!(output).ok();
                }
            }
            writeln!(output, "({} rows)", rows.len()).ok();
        }
    }
}

// ==================== Export ====================

fn export_to_csv(result: &kuzu_main::QueryResult, file_path: &str) -> Result<(), String> {
    use std::fs::File;
    use std::io::Write;
    let mut f = File::create(file_path).map_err(|e| format!("Cannot create: {e}"))?;
    for chunk in &result.chunks {
        for r in 0..chunk.size {
            let row: Vec<String> = (0..chunk.fields.len()).map(|col| fmt_val(&chunk.get_value(col, r).unwrap_or(kuzu_common::types::Value::Null))).collect();
            let csv: Vec<String> = row
                .iter()
                .map(|v| {
                    if v.contains(',') || v.contains('"') {
                        format!("\"{}\"", v.replace('"', "\"\""))
                    } else {
                        v.clone()
                    }
                })
                .collect();
            writeln!(f, "{}", csv.join(",")).map_err(|e| format!("Write: {e}"))?;
        }
    }
    Ok(())
}

// ==================== Script Mode ====================

fn run_script(conn: &Connection, reader: impl BufRead, output: &mut dyn Write) {
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let t = line.trim();
        if t.is_empty() || t.starts_with("--") || t.starts_with("//") {
            continue;
        }
        match conn.query(t) {
            Ok(r) => {
                if !r.is_success() {
                    let _ = writeln!(output, "Error: {}", r.error_message.as_deref().unwrap_or("Unknown"));
                }
            }
            Err(e) => {
                let _ = writeln!(output, "Error: {e}");
            }
        }
    }
}

// ==================== Value Formatting ====================

fn fmt_val(v: &kuzu_common::types::Value) -> String {
    use kuzu_common::types::Value;
    match v {
        Value::Null => "NULL".into(),
        Value::Int64(i) => i.to_string(),
        Value::Int32(i) => i.to_string(),
        Value::Int16(i) => i.to_string(),
        Value::Double(d) => format!("{:.4}", d),
        Value::Float(f) => format!("{:.4}", f),
        Value::Bool(b) => b.to_string(),
        Value::String(s) => s.clone(),
        _ => "<val>".into(),
    }
}
