use akar_main::connection::Connection;
use akar_main::database::{Database, SystemConfig};
use std::fs;
use std::path::Path;
use std::sync::Arc;

fn write_note(root: &Path, rel_path: &str, content: &str) {
    let p = root.join(rel_path);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&p, content).unwrap();
}

#[test]
fn test_cypher_load_wiki_okf() {
    let wiki_dir = tempfile::tempdir().unwrap();
    let db_dir = tempfile::tempdir().unwrap();

    let note_a = r#"---
title: "Memory System Design"
category: "architecture"
active: true
version: 2
tags:
  - engine
  - persistence
meta:
  author:
    name: "Architect"
    team: "Akar Core"
  config:
    nodes: 50000
    hnsw:
      m: 16
      alpha: 1.2
---
# Memory System Design
This document outlines the architecture.
See also [[concepts/persistence]] and [[concepts/vector_index|HNSW Guide]].
"#;

    let note_b = r#"---
title: "Persistence Layer"
tags:
  - storage
---
# Persistence Layer
Details on page storage and buffer manager.
"#;

    write_note(wiki_dir.path(), "architecture/memory.md", note_a);
    write_note(wiki_dir.path(), "concepts/persistence.md", note_b);

    // Initialize database
    let db = Arc::new(Database::new(db_dir.path().to_path_buf(), SystemConfig::default()).unwrap());
    let conn = Connection::new(&db);

    // Execute read_markdown_wiki table function via Cypher CALL
    let wiki_path_str = wiki_dir.path().to_string_lossy().replace('\\', "/");
    let query = format!("CALL read_markdown_wiki('{wiki_path_str}')");
    let result = conn.query(&query).expect("CALL read_markdown_wiki should succeed");

    assert!(result.num_rows() >= 2, "must return note rows and relation rows");
    let _num_rows = result.num_rows();

    // Verify column headers: node, rel, metadata
    // In QueryResult, rows can be scanned
    let mut found_memory_note = false;
    let mut found_persistence_note = false;
    let mut found_rel_to_persistence = false;
    let mut found_rel_to_vector_index = false;

    for chunk in &result.chunks {
        for row_idx in 0..chunk.size {
            let node_val = chunk.get_value(0, row_idx);
            let rel_val = chunk.get_value(1, row_idx);
            let meta_val = chunk.get_value(2, row_idx);

            let node_str = match node_val {
                Some(akar_common::types::Value::String(s)) => s.clone(),
                _ => continue,
            };

            if node_str == "architecture/memory" {
                match rel_val {
                    Some(akar_common::types::Value::Null) | None => {
                        // Own note row
                        found_memory_note = true;
                        // Verify metadata has nested OKF v0.2 structure
                        if let Some(akar_common::types::Value::String(meta_json_str)) = meta_val {
                            let json: serde_json::Value =
                                serde_json::from_str(&meta_json_str).expect("metadata must be valid JSON");
                            assert_eq!(json["title"], "Memory System Design");
                            assert_eq!(json["active"], true);
                            assert_eq!(json["version"], 2.0);
                            assert_eq!(json["tags"][0], "engine");
                            assert_eq!(json["tags"][1], "persistence");
                            assert_eq!(json["meta"]["author"]["name"], "Architect");
                            assert_eq!(json["meta"]["config"]["hnsw"]["alpha"], 1.2);
                        } else {
                            panic!("expected metadata string on note row");
                        }
                    }
                    Some(akar_common::types::Value::String(target)) => {
                        if target == "concepts/persistence" {
                            found_rel_to_persistence = true;
                        } else if target == "concepts/vector_index" {
                            found_rel_to_vector_index = true;
                        }
                    }
                    _ => {}
                }
            } else if node_str == "concepts/persistence" {
                if let Some(akar_common::types::Value::Null) | None = rel_val {
                    found_persistence_note = true;
                    if let Some(akar_common::types::Value::String(meta_json_str)) = meta_val {
                        let json: serde_json::Value =
                            serde_json::from_str(&meta_json_str).expect("metadata must be valid JSON");
                        assert_eq!(json["title"], "Persistence Layer");
                        assert_eq!(json["tags"][0], "storage");
                    }
                }
            }
        }
    }

    assert!(found_memory_note, "must find architecture/memory note row");
    assert!(found_persistence_note, "must find concepts/persistence note row");
    assert!(found_rel_to_persistence, "must find link to concepts/persistence");
    assert!(found_rel_to_vector_index, "must find link to concepts/vector_index");
}
