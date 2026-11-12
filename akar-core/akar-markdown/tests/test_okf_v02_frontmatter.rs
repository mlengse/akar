use akar_markdown::frontmatter::{FrontMatter, FrontMatterValue};
use std::collections::BTreeMap;

#[test]
fn test_okf_v02_nested_frontmatter_parsing() {
    let raw = r#"---
title: "OKF v0.2 Knowledge Document"
version: 0.2
active: true
tags:
  - knowledge
  - graph
  - memory
spec:
  author:
    name: "Akar Architect"
    role: "Core Engine"
  properties:
    dimensions: 1536
    clusters:
      primary: "embeddings"
      secondary: "sparse"
---
# Content
Document body.
"#;

    let fm = FrontMatter::parse(raw).expect("frontmatter should parse successfully");

    // Title scalar
    assert_eq!(fm.title(), Some("OKF v0.2 Knowledge Document"));

    // Number and Bool
    match fm.get("version") {
        Some(FrontMatterValue::Number(n)) => assert!((n - 0.2).abs() < 1e-6),
        other => panic!("expected Number(0.2), got {other:?}"),
    }
    match fm.get("active") {
        Some(FrontMatterValue::Bool(b)) => assert!(b),
        other => panic!("expected Bool(true), got {other:?}"),
    }

    // List of tags
    let tags = fm.tags();
    assert_eq!(tags, vec!["knowledge", "graph", "memory"]);

    // Nested map navigation via get_nested
    let author_name = fm.get_nested(&["spec", "author", "name"]);
    assert_eq!(author_name.and_then(|v| v.as_str()), Some("Akar Architect"));

    let primary_cluster = fm.get_nested(&["spec", "properties", "clusters", "primary"]);
    assert_eq!(primary_cluster.and_then(|v| v.as_str()), Some("embeddings"));

    let dimensions = fm.get_nested(&["spec", "properties", "dimensions"]);
    match dimensions {
        Some(FrontMatterValue::Number(d)) => assert_eq!(*d as u32, 1536),
        other => panic!("expected dimensions to be Number(1536), got {other:?}"),
    }
}

#[test]
fn test_okf_v02_json_roundtrip() {
    let mut map = BTreeMap::new();
    map.insert("nested_key".into(), FrontMatterValue::Scalar("nested_val".into()));
    let fmv = FrontMatterValue::Map(map);

    let json_val = fmv.to_json();
    assert_eq!(json_val["nested_key"], "nested_val");

    let restored = FrontMatterValue::from_json(json_val);
    assert_eq!(fmv, restored);
}
