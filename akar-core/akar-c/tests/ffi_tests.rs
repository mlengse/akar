use akar_c::*;
use std::ffi::CString;
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_db_path() -> (CString, u64) {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("akar_c_test_{}_{}", std::process::id(), id));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("test.db");
    (CString::new(path.to_string_lossy().as_bytes()).unwrap(), id)
}

fn cleanup(id: u64) {
    let dir = std::env::temp_dir().join(format!("akar_c_test_{}_{}", std::process::id(), id));
    let _ = std::fs::remove_dir_all(dir);
}

unsafe fn init_db(path: &CString) -> akar_database {
    let config = unsafe { akar_default_system_config() };
    let mut db = akar_database {
        _database: ptr::null_mut(),
    };
    let state = unsafe { akar_database_init(path.as_ptr(), config, &mut db) };
    assert_eq!(state, akar_state::AkarSuccess);
    db
}

unsafe fn init_conn(db: &mut akar_database) -> akar_connection {
    let mut conn = akar_connection {
        _connection: ptr::null_mut(),
    };
    let state = unsafe { akar_connection_init(db, &mut conn) };
    assert_eq!(state, akar_state::AkarSuccess);
    conn
}

#[test]
fn test_default_config_returns_valid_values() {
    let config = unsafe { akar_default_system_config() };
    assert!(config.max_db_size > 0);
    assert!(config.checkpoint_threshold > 0);
    assert!(config.enable_compression);
}

#[test]
fn test_database_init_and_destroy() {
    let (path, id) = temp_db_path();
    let mut db = unsafe { init_db(&path) };
    unsafe { akar_database_destroy(&mut db) };
    cleanup(id);
}

#[test]
fn test_database_init_null_path_returns_error() {
    let config = unsafe { akar_default_system_config() };
    let mut db = akar_database {
        _database: ptr::null_mut(),
    };
    let state = unsafe { akar_database_init(ptr::null(), config, &mut db) };
    assert_eq!(state, akar_state::AkarError);
}

#[test]
fn test_database_init_null_out_returns_error() {
    let (path, id) = temp_db_path();
    let config = unsafe { akar_default_system_config() };
    let state = unsafe { akar_database_init(path.as_ptr(), config, ptr::null_mut()) };
    assert_eq!(state, akar_state::AkarError);
    cleanup(id);
}

#[test]
fn test_database_destroy_null_is_safe() {
    unsafe { akar_database_destroy(ptr::null_mut()) };
}

#[test]
fn test_database_destroy_double_is_safe() {
    let (path, id) = temp_db_path();
    let mut db = unsafe { init_db(&path) };
    unsafe {
        akar_database_destroy(&mut db);
        akar_database_destroy(&mut db);
    }
    cleanup(id);
}

#[test]
fn test_connection_init_and_destroy() {
    let (path, id) = temp_db_path();
    let mut db = unsafe { init_db(&path) };
    let mut conn = unsafe { init_conn(&mut db) };
    unsafe { akar_connection_destroy(&mut conn) };
    unsafe { akar_database_destroy(&mut db) };
    cleanup(id);
}

#[test]
fn test_connection_init_null_database_returns_error() {
    let mut conn = akar_connection {
        _connection: ptr::null_mut(),
    };
    let state = unsafe { akar_connection_init(ptr::null_mut(), &mut conn) };
    assert_eq!(state, akar_state::AkarError);
}

#[test]
fn test_connection_init_null_out_returns_error() {
    let (path, id) = temp_db_path();
    let mut db = unsafe { init_db(&path) };
    let state = unsafe { akar_connection_init(&mut db, ptr::null_mut()) };
    assert_eq!(state, akar_state::AkarError);
    unsafe { akar_database_destroy(&mut db) };
    cleanup(id);
}

#[test]
fn test_connection_destroy_null_is_safe() {
    unsafe { akar_connection_destroy(ptr::null_mut()) };
}

#[test]
fn test_query_create_table_and_insert() {
    let (path, id) = temp_db_path();
    let mut db = unsafe { init_db(&path) };
    let mut conn = unsafe { init_conn(&mut db) };

    let create = CString::new("CREATE NODE TABLE Person(id INT64, name STRING, age INT64, PRIMARY KEY(id))").unwrap();
    let mut result = akar_query_result {
        _query_result: ptr::null_mut(),
        _is_owned_by_cpp: false,
    };
    let state = unsafe { akar_connection_query(&mut conn, create.as_ptr(), &mut result) };
    assert_eq!(state, akar_state::AkarSuccess);
    assert!(!result._query_result.is_null());
    unsafe { akar_query_result_destroy(&mut result) };
    assert!(result._query_result.is_null());

    let insert = CString::new("CREATE (p:Person {id: 1, name: 'Alice', age: 30})").unwrap();
    let state = unsafe { akar_connection_query(&mut conn, insert.as_ptr(), &mut result) };
    assert_eq!(state, akar_state::AkarSuccess);
    unsafe { akar_query_result_destroy(&mut result) };

    let query = CString::new("MATCH (p:Person) RETURN p.name").unwrap();
    let state = unsafe { akar_connection_query(&mut conn, query.as_ptr(), &mut result) };
    assert_eq!(state, akar_state::AkarSuccess);
    unsafe { akar_query_result_destroy(&mut result) };

    unsafe { akar_database_destroy(&mut db) };
    cleanup(id);
}

#[test]
fn test_query_result_destroy_null_is_safe() {
    let mut result = akar_query_result {
        _query_result: ptr::null_mut(),
        _is_owned_by_cpp: false,
    };
    unsafe { akar_query_result_destroy(&mut result) };
}

#[test]
fn test_query_result_destroy_double_is_safe() {
    let (path, id) = temp_db_path();
    let mut db = unsafe { init_db(&path) };
    let mut conn = unsafe { init_conn(&mut db) };

    let q = CString::new("RETURN 1").unwrap();
    let mut result = akar_query_result {
        _query_result: ptr::null_mut(),
        _is_owned_by_cpp: false,
    };
    let state = unsafe { akar_connection_query(&mut conn, q.as_ptr(), &mut result) };
    assert_eq!(state, akar_state::AkarSuccess);
    assert!(!result._query_result.is_null());

    unsafe {
        akar_query_result_destroy(&mut result);
        akar_query_result_destroy(&mut result);
    }
    assert!(result._query_result.is_null());

    unsafe { akar_database_destroy(&mut db) };
    cleanup(id);
}

#[test]
fn test_query_error_leaves_no_stale_result() {
    let (path, id) = temp_db_path();
    let mut db = unsafe { init_db(&path) };
    let mut conn = unsafe { init_conn(&mut db) };

    // Syntax error must return AkarError AND leave the result struct null so a
    // later destroy is a safe no-op.
    let bad = CString::new("THIS IS NOT CYPHER").unwrap();
    let mut result = akar_query_result {
        _query_result: ptr::null_mut(),
        _is_owned_by_cpp: false,
    };
    let state = unsafe { akar_connection_query(&mut conn, bad.as_ptr(), &mut result) };
    assert_eq!(state, akar_state::AkarError);
    assert!(result._query_result.is_null());
    unsafe { akar_query_result_destroy(&mut result) };

    unsafe { akar_database_destroy(&mut db) };
    cleanup(id);
}

#[test]
fn test_query_null_connection_returns_error() {
    let q = CString::new("RETURN 1").unwrap();
    let mut result = akar_query_result {
        _query_result: ptr::null_mut(),
        _is_owned_by_cpp: false,
    };
    let state = unsafe { akar_connection_query(ptr::null_mut(), q.as_ptr(), &mut result) };
    assert_eq!(state, akar_state::AkarError);
}

#[test]
fn test_query_null_query_returns_error() {
    let (path, id) = temp_db_path();
    let mut db = unsafe { init_db(&path) };
    let mut conn = unsafe { init_conn(&mut db) };

    let mut result = akar_query_result {
        _query_result: ptr::null_mut(),
        _is_owned_by_cpp: false,
    };
    let state = unsafe { akar_connection_query(&mut conn, ptr::null(), &mut result) };
    assert_eq!(state, akar_state::AkarError);

    unsafe { akar_database_destroy(&mut db) };
    cleanup(id);
}

#[test]
fn test_query_null_out_result_returns_error() {
    let (path, id) = temp_db_path();
    let mut db = unsafe { init_db(&path) };
    let mut conn = unsafe { init_conn(&mut db) };

    let q = CString::new("RETURN 1").unwrap();
    let state = unsafe { akar_connection_query(&mut conn, q.as_ptr(), ptr::null_mut()) };
    assert_eq!(state, akar_state::AkarError);

    unsafe { akar_database_destroy(&mut db) };
    cleanup(id);
}
