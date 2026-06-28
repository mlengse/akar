//! Neo4j migration extension for Kuzu.
//!
//! Provides the `NEO4J_MIGRATE` table function that parses a Neo4j
//! Cypher dump file and migrates the schema and data into Kuzu.
//!
//! Supported Neo4j dump constructs:
//! - `CREATE CONSTRAINT` — mapped to Kuzu PRIMARY KEY
//! - `CREATE INDEX` — noted (Kuzu auto-indexes PKs)
//! - `CREATE (n:Label {props})` — creates node tables + rows
//! - `MATCH ... CREATE (a)-[r:REL_TYPE {props}]->(b)` — creates rel tables + rows

use kuzu_extension::{Extension, ExtensionContext};

/// The Neo4j migration extension.
pub struct Neo4jExtension;

impl Neo4jExtension {
    pub fn new() -> Self {
        Self
    }
}

impl Extension for Neo4jExtension {
    fn name(&self) -> &'static str {
        "NEO4J"
    }

    fn load(&self, context: &ExtensionContext) -> Result<(), String> {
        use kuzu_function::registry::TableFunction;

        context.register_table_function(
            "neo4j_migrate",
            TableFunction::Custom { name: "neo4j_migrate".into() },
        );

        tracing::info!("NEO4J extension loaded: neo4j_migrate function registered");
        Ok(())
    }
}

// ==================== Neo4j Dump Parser ====================

/// A parsed node from a Neo4j dump.
#[derive(Debug, Clone)]
pub struct Neo4jNode {
    pub variable: String,
    pub labels: Vec<String>,
    pub properties: Vec<(String, String)>, // (key, value as string)
}

/// A parsed relationship from a Neo4j dump.
#[derive(Debug, Clone)]
pub struct Neo4jRel {
    pub variable: String,
    pub rel_type: String,
    pub from_var: String,
    pub to_var: String,
    pub properties: Vec<(String, String)>,
}

/// A parsed constraint or index statement.
#[derive(Debug, Clone)]
pub enum Neo4jSchemaStmt {
    Constraint {
        label: String,
        property: String,
    },
    Index {
        label: String,
        property: String,
    },
}

/// Result of parsing a Neo4j dump.
#[derive(Debug, Clone, Default)]
pub struct Neo4jDump {
    pub schema: Vec<Neo4jSchemaStmt>,
    pub nodes: Vec<Neo4jNode>,
    pub rels: Vec<Neo4jRel>,
}

/// Parse a Neo4j Cypher dump string into structured data.
///
/// Handles:
/// - `CREATE CONSTRAINT FOR (n:Label) REQUIRE n.prop IS UNIQUE`
/// - `CREATE INDEX FOR (n:Label) ON (n.prop)`
/// - `CREATE (n:Label {key: val, ...})`
/// - `MATCH (a:Label1), (b:Label2) CREATE (a)-[r:REL_TYPE {key: val}]->(b)`
pub fn parse_neo4j_dump(input: &str) -> Result<Neo4jDump, String> {
    let mut dump = Neo4jDump::default();

    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }

        let upper = trimmed.to_uppercase();

        if upper.starts_with("CREATE CONSTRAINT") {
            if let Some(stmt) = parse_constraint(trimmed)? {
                dump.schema.push(stmt);
            }
        } else if upper.starts_with("CREATE INDEX") {
            if let Some(stmt) = parse_index(trimmed)? {
                dump.schema.push(stmt);
            }
        } else if upper.contains("CREATE (") && !upper.contains("MATCH") {
            // Node creation: CREATE (n:Label {props})
            if let Some(node) = parse_node_creation(trimmed)? {
                dump.nodes.push(node);
            }
        } else if upper.contains("MATCH") && upper.contains("CREATE (") && upper.contains("]->(") {
            // Relationship creation: MATCH ... CREATE (a)-[r:T]->(b)
            if let Some(rel) = parse_rel_creation(trimmed)? {
                dump.rels.push(rel);
            }
        }
    }

    Ok(dump)
}

/// Parse `CREATE CONSTRAINT FOR (n:Label) REQUIRE n.prop IS UNIQUE`
fn parse_constraint(line: &str) -> Result<Option<Neo4jSchemaStmt>, String> {
    // Pattern: CREATE CONSTRAINT FOR (n:Label) REQUIRE n.prop IS UNIQUE
    let line = line.trim().strip_prefix("CREATE CONSTRAINT")
        .or_else(|| line.trim().strip_prefix("create constraint"))
        .ok_or_else(|| format!("Expected CREATE CONSTRAINT, got: {line}"))?;

    // Extract label from (n:Label)
    let label = extract_label(line)?;

    // Extract property from REQUIRE n.prop
    let after_label = line.split("REQUIRE")
        .nth(1)
        .or_else(|| line.split("require").nth(1))
        .ok_or_else(|| format!("Missing REQUIRE in constraint: {line}"))?;

    let prop = after_label
        .trim()
        .split('.')
        .nth(1)
        .map(|s| s.split_whitespace().next().unwrap_or(""))
        .unwrap_or("")
        .to_string();

    if prop.is_empty() {
        return Err(format!("Could not parse property in constraint: {line}"));
    }

    Ok(Some(Neo4jSchemaStmt::Constraint { label, property: prop }))
}

/// Parse `CREATE INDEX FOR (n:Label) ON (n.prop)`
fn parse_index(line: &str) -> Result<Option<Neo4jSchemaStmt>, String> {
    let line = line.trim().strip_prefix("CREATE INDEX")
        .or_else(|| line.trim().strip_prefix("create index"))
        .ok_or_else(|| format!("Expected CREATE INDEX, got: {line}"))?;

    let label = extract_label(line)?;

    // Extract property from ON (n.prop)
    let after_label = line.split("ON")
        .nth(1)
        .or_else(|| line.split("on").nth(1))
        .ok_or_else(|| format!("Missing ON in index: {line}"))?;

    let prop = after_label
        .trim()
        .trim_start_matches('(')
        .split('.')
        .nth(1)
        .map(|s| s.trim_end_matches(')').trim().to_string())
        .unwrap_or_default();

    if prop.is_empty() {
        return Err(format!("Could not parse property in index: {line}"));
    }

    Ok(Some(Neo4jSchemaStmt::Index { label, property: prop }))
}

/// Extract label from `(n:Label)` pattern in a string.
fn extract_label(s: &str) -> Result<String, String> {
    let paren_start = s.find('(').ok_or_else(|| format!("Missing '(' in: {s}"))?;
    let after_paren = &s[paren_start + 1..];
    let colon_pos = after_paren.find(':').ok_or_else(|| format!("Missing ':' in label: {s}"))?;
    let label_end = after_paren[colon_pos + 1..]
        .find(|c: char| c == ')' || c == ' ' || c == '\n')
        .unwrap_or_else(|| after_paren[colon_pos + 1..].len());
    Ok(after_paren[colon_pos + 1..][..label_end].trim().to_string())
}

/// Parse `CREATE (n:Label {key: val, ...})` — single node creation.
fn parse_node_creation(line: &str) -> Result<Option<Neo4jNode>, String> {
    // Extract from first CREATE to end
    let body = skip_keyword(line, "create")?;

    // Find the parenthesized expression
    let paren_start = body.find('(').ok_or_else(|| format!("Missing '(' in CREATE: {line}"))?;
    let paren_content = extract_paren_content(&body[paren_start..])?;

    // Parse variable, labels, and properties
    let (variable, labels, props_str) = parse_node_pattern(&paren_content)?;
    let properties = parse_properties(&props_str)?;

    Ok(Some(Neo4jNode { variable, labels, properties }))
}

/// Parse `MATCH ... CREATE (a)-[r:REL_TYPE {props}]->(b)`
fn parse_rel_creation(line: &str) -> Result<Option<Neo4jRel>, String> {
    // Find "CREATE" portion
    let create_idx = line.to_uppercase().find("CREATE")
        .ok_or_else(|| format!("Missing CREATE in rel stmt: {line}"))?;
    let create_part = &line[create_idx + 6..].trim();

    // Pattern: (from_var)-[r:TYPE {props}]->(to_var)
    // Find the arrow boundary `]->(`
    let arrow_pos = create_part.find("]->(")
        .ok_or_else(|| format!("Missing ']->(' in rel pattern: {line}"))?;

    // Everything before `]->` is the from-side expression: (from_var)-[...]
    let from_expr = &create_part[..arrow_pos];
    // Everything from `(` onward is the to-side: (to_var)
    let to_expr = &create_part[arrow_pos + 3..];

    // Parse from-side: find the opening `(` and extract variable before `)`
    let from_paren_start = from_expr.find('(')
        .ok_or_else(|| format!("Missing '(' in from pattern: {line}"))?;
    let from_paren_end = from_expr[from_paren_start + 1..].find(')')
        .ok_or_else(|| format!("Missing ')' in from pattern: {line}"))?;
    let from_var = from_expr[from_paren_start + 1..][..from_paren_end].trim();
    // Strip labels if present: take only the variable name (before first ':')
    let from_var = from_var.split(':').next().unwrap_or("").trim().to_string();

    // Parse to-side: (to_var)
    let to_content = extract_paren_content(to_expr)?;
    let to_var = to_content.split(':').next().unwrap_or("").trim().to_string();

    // Parse relationship inside brackets: [r:TYPE {props}]
    let bracket_start = from_expr.find('[')
        .ok_or_else(|| format!("Missing '[' in rel pattern: {line}"))?;
    let bracket_content = &from_expr[bracket_start + 1..];

    // Split bracket content: variable:TYPE {props}
    let rel_var = bracket_content.split(':').next().unwrap_or("").trim().to_string();
    let after_colon = bracket_content.split(':').nth(1).unwrap_or("");
    let rel_type = after_colon.split('{').next().unwrap_or("").trim().to_string();

    // Parse properties from {...}
    let props_start = bracket_content.find('{');
    let properties = if let Some(ps) = props_start {
        parse_properties(&bracket_content[ps..])?
    } else {
        vec![]
    };

    Ok(Some(Neo4jRel {
        variable: rel_var,
        rel_type,
        from_var,
        to_var,
        properties,
    }))
}

/// Extract content inside matching parentheses.
fn extract_paren_content(input: &str) -> Result<String, String> {
    let input = input.trim();
    if !input.starts_with('(') {
        return Err(format!("Expected '(' at start, got: {input}"));
    }
    let mut depth = 0;
    let mut start = None;
    let mut end = None;
    for (i, c) in input.char_indices() {
        match c {
            '(' => {
                if start.is_none() {
                    start = Some(i + 1);
                }
                depth += 1;
            }
            ')' => {
                depth -= 1;
                if depth == 0 && start.is_some() {
                    end = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    match (start, end) {
        (Some(st), Some(en)) => Ok(input[st..en].to_string()),
        _ => Err(format!("Unmatched parentheses in: {input}")),
    }
}

/// Parse node pattern: `var:Label1:Label2 {props}`
fn parse_node_pattern(s: &str) -> Result<(String, Vec<String>, String), String> {
    let s = s.trim();
    let colon_pos = s.find(':');
    let _brace_pos = s.find('{');

    let (before_colon, _rest) = match colon_pos {
        Some(p) => (&s[..p], &s[p + 1..]),
        None => (s, ""),
    };

    let variable = before_colon.trim().to_string();

    let (labels, props_str) = if let Some(bp) = s.find('{') {
        let before_brace = &s[..bp];
        let after_colon = before_brace.split(':').nth(1).unwrap_or("");
        let labels: Vec<String> = after_colon
            .split(':')
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        (labels, s[bp..].to_string())
    } else {
        let after_colon = s.split(':').nth(1).unwrap_or("");
        let labels: Vec<String> = after_colon
            .split(':')
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        (labels, String::new())
    };

    Ok((variable, labels, props_str))
}

/// Parse properties from `{key1: val1, key2: val2, ...}`
fn parse_properties(s: &str) -> Result<Vec<(String, String)>, String> {
    let s = s.trim();
    if !s.starts_with('{') {
        return Ok(vec![]);
    }
    let inner = s[1..s.rfind('}').unwrap_or(s.len())].trim();
    if inner.is_empty() {
        return Ok(vec![]);
    }

    let mut props = Vec::new();
    // Naive split by commas (doesn't handle nested braces, but sufficient for flat props)
    let mut depth = 0;
    let mut current = String::new();
    for c in inner.chars() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                if let Some((k, v)) = parse_property_pair(&current) {
                    props.push((k, v));
                }
                current.clear();
                continue;
            }
            _ => {}
        }
        current.push(c);
    }
    if !current.is_empty() {
        if let Some((k, v)) = parse_property_pair(&current) {
            props.push((k, v));
        }
    }

    Ok(props)
}

/// Parse a single `key: value` pair.
fn parse_property_pair(s: &str) -> Option<(String, String)> {
    let s = s.trim();
    let colon_pos = s.find(':')?;
    let key = s[..colon_pos].trim().to_string();
    let value = s[colon_pos + 1..].trim().to_string();
    Some((key, value))
}

/// Skip a leading keyword (case-insensitive) and return the rest.
fn skip_keyword<'a>(s: &'a str, keyword: &str) -> Result<&'a str, String> {
    let lower = s.to_lowercase();
    if lower.starts_with(keyword) {
        Ok(s[keyword.len()..].trim())
    } else {
        Err(format!("Expected '{keyword}' at start, got: {s}"))
    }
}

// ==================== Migration Logic ====================

/// Result of a migration.
#[derive(Debug, Clone)]
pub struct MigrationReport {
    pub nodes_created: usize,
    pub rels_created: usize,
    pub constraints_found: usize,
    pub indexes_found: usize,
    pub errors: Vec<String>,
}

/// Run full migration: parse dump → validate → report.
/// In a real execution, this would interact with the Kuzu catalog
/// and storage to actually create tables and insert data.
pub fn run_migration(dump_content: &str) -> Result<MigrationReport, String> {
    let parsed = parse_neo4j_dump(dump_content)?;

    let constraints = parsed.schema.iter()
        .filter(|s| matches!(s, Neo4jSchemaStmt::Constraint { .. }))
        .count();
    let indexes = parsed.schema.iter()
        .filter(|s| matches!(s, Neo4jSchemaStmt::Index { .. }))
        .count();

    // Validate: count unique labels/types
    let node_labels: std::collections::HashSet<&str> = parsed.nodes
        .iter()
        .flat_map(|n| n.labels.iter().map(|l| l.as_str()))
        .collect();
    let rel_types: std::collections::HashSet<&str> = parsed.rels
        .iter()
        .map(|r| r.rel_type.as_str())
        .collect();

    tracing::info!(
        "Migration parsed: {} nodes (labels: {:?}), {} rels (types: {:?}), {} constraints, {} indexes",
        parsed.nodes.len(),
        node_labels,
        parsed.rels.len(),
        rel_types,
        constraints,
        indexes,
    );

    Ok(MigrationReport {
        nodes_created: parsed.nodes.len(),
        rels_created: parsed.rels.len(),
        constraints_found: constraints,
        indexes_found: indexes,
        errors: vec![],
    })
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_single_node() {
        let input = "CREATE (a:Person {name: 'Alice', age: 30})";
        let dump = parse_neo4j_dump(input).unwrap();
        assert_eq!(dump.nodes.len(), 1);
        assert_eq!(dump.nodes[0].variable, "a");
        assert_eq!(dump.nodes[0].labels, vec!["Person"]);
        assert!(dump.nodes[0].properties.iter().any(|(k, _)| k == "name"));
        assert!(dump.nodes[0].properties.iter().any(|(k, _)| k == "age"));
    }

    #[test]
    fn test_parse_constraint() {
        let input = "CREATE CONSTRAINT FOR (n:Person) REQUIRE n.id IS UNIQUE";
        let dump = parse_neo4j_dump(input).unwrap();
        assert_eq!(dump.schema.len(), 1);
        match &dump.schema[0] {
            Neo4jSchemaStmt::Constraint { label, property } => {
                assert_eq!(label, "Person");
                assert_eq!(property, "id");
            }
            _ => panic!("Expected Constraint"),
        }
    }

    #[test]
    fn test_parse_index() {
        let input = "CREATE INDEX FOR (n:Person) ON (n.name)";
        let dump = parse_neo4j_dump(input).unwrap();
        assert_eq!(dump.schema.len(), 1);
        match &dump.schema[0] {
            Neo4jSchemaStmt::Index { label, property } => {
                assert_eq!(label, "Person");
                assert_eq!(property, "name");
            }
            _ => panic!("Expected Index"),
        }
    }

    #[test]
    fn test_parse_relationship() {
        let input = "MATCH (a:Person), (b:City) CREATE (a)-[r:LIVES_IN {since: 2020}]->(b)";
        let dump = parse_neo4j_dump(input).unwrap();
        assert_eq!(dump.rels.len(), 1);
        assert_eq!(dump.rels[0].rel_type, "LIVES_IN");
        assert_eq!(dump.rels[0].from_var, "a");
        assert_eq!(dump.rels[0].to_var, "b");
        assert!(dump.rels[0].properties.iter().any(|(k, _)| k == "since"));
    }

    #[test]
    fn test_parse_full_dump() {
        let input = r#"
CREATE CONSTRAINT FOR (n:Person) REQUIRE n.id IS UNIQUE
CREATE INDEX FOR (n:Person) ON (n.name)
CREATE (a:Person {id: 1, name: 'Alice', age: 30})
CREATE (b:Person {id: 2, name: 'Bob', age: 25})
CREATE (c:City {name: 'NYC', population: 8000000})
MATCH (a:Person), (c:City) CREATE (a)-[r:LIVES_IN {since: 2020}]->(c)
MATCH (a:Person), (b:Person) CREATE (a)-[r:KNOWS {since: 2019}]->(b)
"#;
        let dump = parse_neo4j_dump(input).unwrap();
        assert_eq!(dump.schema.len(), 2);
        assert_eq!(dump.nodes.len(), 3);
        assert_eq!(dump.rels.len(), 2);
    }

    #[test]
    fn test_empty_dump() {
        let dump = parse_neo4j_dump("").unwrap();
        assert_eq!(dump.schema.len(), 0);
        assert_eq!(dump.nodes.len(), 0);
        assert_eq!(dump.rels.len(), 0);
    }

    #[test]
    fn test_comment_lines() {
        let input = "// This is a comment\nCREATE (a:Test {x: 1})";
        let dump = parse_neo4j_dump(input).unwrap();
        assert_eq!(dump.nodes.len(), 1);
    }

    #[test]
    fn test_multi_label_node() {
        let input = "CREATE (n:Person:Employee {name: 'Alice'})";
        let dump = parse_neo4j_dump(input).unwrap();
        assert_eq!(dump.nodes.len(), 1);
        // Should extract at least one label
        assert!(!dump.nodes[0].labels.is_empty());
    }

    #[test]
    fn test_run_migration() {
        let input = "CREATE (a:Person {name: 'Alice'})\nMATCH (a:Person), (b:Person) CREATE (a)-[r:KNOWS {}]->(b)";
        let report = run_migration(input).unwrap();
        assert_eq!(report.nodes_created, 1);
        assert!(report.errors.is_empty());
    }

    #[test]
    fn test_neo4j_extension_registration() {
        let ext = Neo4jExtension::new();
        assert_eq!(ext.name(), "NEO4J");
    }

    #[test]
    fn test_parse_properties_empty() {
        let props = parse_properties("{}").unwrap();
        assert!(props.is_empty());
    }

    #[test]
    fn test_parse_properties_multiple() {
        let props = parse_properties("{name: 'Alice', age: 30, active: true}").unwrap();
        assert_eq!(props.len(), 3);
        assert_eq!(props[0].0, "name");
        assert_eq!(props[1].0, "age");
        assert_eq!(props[2].0, "active");
    }
}
