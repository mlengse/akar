mod common;
use common::{setup_db, exec, query_values};

#[test]
fn test_unicode_emojis_in_string() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE T(id INT64, s STRING, PRIMARY KEY (id))");
    exec(&conn, "CREATE (t:T {id: 1, s: 'Hello 🌍, 😊!'})");
    let res = query_values(&conn, "MATCH (t:T) RETURN t.s");
    assert_eq!(res.trim(), "String(\"Hello 🌍, 😊!\")");
}

#[test]
fn test_unicode_non_latin_chars() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE T(id INT64, s STRING, PRIMARY KEY (id))");
    let cjk = "こんにちは, 世界. 안녕하세요";
    let query = format!("CREATE (t:T {{id: 1, s: '{}'}})", cjk);
    exec(&conn, &query);
    let res = query_values(&conn, "MATCH (t:T) RETURN t.s");
    assert_eq!(res.trim(), format!("String(\"{}\")", cjk));
}

#[test]
fn test_unicode_table_and_property_names() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE `테스트`(id INT64, `属性` STRING, PRIMARY KEY (id))");
    exec(&conn, "CREATE (t:`테스트` {id: 1, `属性`: 'val'})");
    let res = query_values(&conn, "MATCH (t:`테스트`) RETURN t.`属性`");
    assert_eq!(res.trim(), "String(\"val\")");
}

#[test]
fn test_unicode_zero_width_spaces() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE T(id INT64, s STRING, PRIMARY KEY (id))");
    exec(&conn, "CREATE (t:T {id: 1, s: 'a\u{200B}b'})");
    let res = query_values(&conn, "MATCH (t:T) RETURN t.s");
    assert!(res.contains("a\\u{200b}b") || res.contains("a\u{200B}b"));
}

#[test]
fn test_unicode_control_characters() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE T(id INT64, s STRING, PRIMARY KEY (id))");
    exec(&conn, "CREATE (t:T {id: 1, s: 'line1\nline2\t\r'})");
    let res = query_values(&conn, "MATCH (t:T) RETURN t.s");
    assert!(res.contains("line1\\nline2\\t\\r") || res.contains("line1\nline2\t\r"));
}

#[test]
fn test_unicode_substring() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE T(id INT64, s STRING, PRIMARY KEY (id))");
    exec(&conn, "CREATE (t:T {id: 1, s: '😊🌍'})");
    let res = query_values(&conn, "MATCH (t:T) RETURN substring(t.s, 1, 1)");
    assert_eq!(res.trim(), "String(\"😊\")");
}

#[test]
fn test_unicode_concat() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE T(id INT64, s1 STRING, s2 STRING, PRIMARY KEY (id))");
    exec(&conn, "CREATE (t:T {id: 1, s1: 'Hello ', s2: '🌍'})");
    let res = query_values(&conn, "MATCH (t:T) RETURN t.s1 + t.s2");
    assert_eq!(res.trim(), "String(\"Hello 🌍\")");
}

#[test]
fn test_unicode_lower_upper() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE T(id INT64, s STRING, PRIMARY KEY (id))");
    exec(&conn, "CREATE (t:T {id: 1, s: 'ÄÖÜ'})");
    let res = query_values(&conn, "MATCH (t:T) RETURN lower(t.s)");
    assert_eq!(res.trim(), "String(\"äöü\")");
}

#[test]
fn test_unicode_long_string_multi_byte() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE T(id INT64, s STRING, PRIMARY KEY (id))");
    let long_str = "😊".repeat(5);
    exec(&conn, &format!("CREATE (t:T {{id: 1, s: '{}'}})", long_str));
    let res = query_values(&conn, "MATCH (t:T) RETURN t.s");
    assert!(res.contains("😊"));
}

#[test]
fn test_unicode_mixed_scripts() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE T(id INT64, s STRING, PRIMARY KEY (id))");
    exec(&conn, "CREATE (t:T {id: 1, s: 'English 中文 Español العربية'})");
    let res = query_values(&conn, "MATCH (t:T) RETURN t.s");
    assert_eq!(res.trim(), "String(\"English 中文 Español العربية\")");
}

#[test]
fn test_unicode_diacritics() {
    let (_db, conn) = setup_db();
    exec(&conn, "CREATE NODE TABLE T(id INT64, s STRING, PRIMARY KEY (id))");
    exec(&conn, "CREATE (t:T {id: 1, s: 'éèêëàâäùûüôö'})");
    let res = query_values(&conn, "MATCH (t:T) RETURN t.s");
    assert_eq!(res.trim(), "String(\"éèêëàâäùûüôö\")");
}
