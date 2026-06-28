//! Parser implementation — converts Cypher query text to AST.
//!
//! TODO: Replace with pest.rs PEG grammar parser.
//! This is a temporary placeholder to allow compilation.

use crate::ast::*;

/// Parse a Cypher query string into a Statement AST.
pub fn parse(input: &str) -> Result<Statement, String> {
    let trimmed = input.trim();

    if trimmed.to_uppercase().starts_with("CREATE NODE TABLE") {
        parse_create_node_table(trimmed)
    } else if trimmed.to_uppercase().starts_with("CREATE REL TABLE") {
        parse_create_rel_table(trimmed)
    } else if trimmed.to_uppercase().starts_with("DROP TABLE") {
        parse_drop_table(trimmed)
    } else if trimmed.to_uppercase().starts_with("MATCH") || trimmed.to_uppercase().starts_with("RETURN") {
        parse_query(trimmed)
    } else {
        Err(format!("Unsupported statement: {input}"))
    }
}

fn parse_create_node_table(input: &str) -> Result<Statement, String> {
    // Simplified: CREATE NODE TABLE name (col1 TYPE, col2 TYPE, PRIMARY KEY (col))
    let rest = input
        .strip_prefix("CREATE NODE TABLE")
        .or_else(|| input.strip_prefix("CREATE NODE TABLE"))
        .ok_or("Expected CREATE NODE TABLE")?
        .trim();

    let paren = rest.find('(').ok_or("Expected (")?;
    let name = rest[..paren].trim().to_string();
    let body = &rest[paren + 1..];
    let close = body.rfind(')').ok_or("Expected )")?;
    let cols_str = body[..close].trim();

    let mut columns = Vec::new();
    let mut primary_key = String::new();

    for part in cols_str.split(',') {
        let part = part.trim();
        if part.to_uppercase().starts_with("PRIMARY KEY") {
            let pk_start = part.find('(').ok_or("Expected ( in PRIMARY KEY")?;
            let pk_end = part.rfind(')').ok_or("Expected ) in PRIMARY KEY")?;
            primary_key = part[pk_start + 1..pk_end].trim().to_string();
        } else {
            let tokens: Vec<&str> = part.split_whitespace().collect();
            if tokens.len() >= 2 {
                columns.push(ColumnDef {
                    name: tokens[0].to_string(),
                    type_name: tokens[1..].join(" "),
                });
            }
        }
    }

    Ok(Statement::CreateNodeTable(CreateNodeTable {
        name,
        columns,
        primary_key,
    }))
}

fn parse_create_rel_table(input: &str) -> Result<Statement, String> {
    let rest = input
        .strip_prefix("CREATE REL TABLE")
        .ok_or("Expected CREATE REL TABLE")?
        .trim();
    let paren = rest.find('(').ok_or("Expected (")?;
    let name = rest[..paren].trim().to_string();
    let body = &rest[paren + 1..];
    let close = body.rfind(')').ok_or("Expected )")?;
    let cols_str = body[..close].trim();

    let mut columns = Vec::new();
    let mut from = String::new();
    let mut to = String::new();

    for part in cols_str.split(',') {
        let part = part.trim();
        if part.to_uppercase().starts_with("FROM") {
            from = part[4..].trim().to_string();
        } else if part.to_uppercase().starts_with("TO") {
            to = part[2..].trim().to_string();
        } else {
            let tokens: Vec<&str> = part.split_whitespace().collect();
            if tokens.len() >= 2 {
                columns.push(ColumnDef {
                    name: tokens[0].to_string(),
                    type_name: tokens[1..].join(" "),
                });
            }
        }
    }

    Ok(Statement::CreateRelTable(CreateRelTable {
        name,
        from,
        to,
        columns,
    }))
}

fn parse_drop_table(input: &str) -> Result<Statement, String> {
    let name = input
        .strip_prefix("DROP TABLE")
        .ok_or("Expected DROP TABLE")?
        .trim();
    Ok(Statement::DropTable(DropTable {
        name: name.to_string(),
    }))
}

fn parse_query(_input: &str) -> Result<Statement, String> {
    // Temporary placeholder — real parser with pest.rs will go here
    Ok(Statement::Query(Query {
        clauses: Vec::new(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_node_table() {
        let sql = "CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY (name))";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::CreateNodeTable(t) => {
                assert_eq!(t.name, "Person");
                assert_eq!(t.columns.len(), 2);
                assert_eq!(t.primary_key, "name");
            }
            _ => panic!("Expected CreateNodeTable"),
        }
    }

    #[test]
    fn test_drop_table() {
        let sql = "DROP TABLE Person";
        let stmt = parse(sql).unwrap();
        match stmt {
            Statement::DropTable(t) => assert_eq!(t.name, "Person"),
            _ => panic!("Expected DropTable"),
        }
    }
}
