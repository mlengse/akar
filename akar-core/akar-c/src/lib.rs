use akar_main::{Connection, Database, SystemConfig};
use std::ffi::{CStr, c_char, c_void};
use std::ptr;
use std::sync::Arc;

#[repr(C)]
pub struct akar_system_config {
    pub buffer_pool_size: u64,
    pub max_num_threads: u64,
    pub enable_compression: bool,
    pub read_only: bool,
    pub max_db_size: u64,
    pub auto_checkpoint: bool,
    pub checkpoint_threshold: u64,
}

#[repr(C)]
pub struct akar_database {
    _database: *mut c_void,
}

#[repr(C)]
pub struct akar_connection {
    _connection: *mut c_void,
}

#[repr(C)]
pub struct akar_query_result {
    _query_result: *mut c_void,
    _is_owned_by_cpp: bool,
}

#[repr(C)]
pub enum akar_state {
    AkarSuccess = 0,
    AkarError = 1,
}

/// Returns a default system configuration for initializing an Akar database.
///
/// # Safety
///
/// This function is safe to call from any context; it does not dereference
/// any raw pointers and simply returns a struct of default configuration values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn akar_default_system_config() -> akar_system_config {
    let config = SystemConfig::default();
    akar_system_config {
        buffer_pool_size: config.buffer_pool_size,
        max_num_threads: config.max_num_threads,
        enable_compression: config.enable_compression,
        read_only: config.read_only,
        max_db_size: config.max_db_size,
        auto_checkpoint: config.auto_checkpoint,
        checkpoint_threshold: config.checkpoint_threshold as u64,
    }
}

/// Initializes an Akar database at the given path with the given configuration.
///
/// # Safety
///
/// - `database_path` must be a valid null-terminated C string.
/// - `out_database` must point to a valid, mutable `akar_database` struct
///   whose `_database` field will be set to an opaque pointer. The caller
///   must later call `akar_database_destroy` to free resources.
/// - Both pointers must remain valid for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn akar_database_init(
    database_path: *const c_char,
    system_config: akar_system_config,
    out_database: *mut akar_database,
) -> akar_state {
    if database_path.is_null() || out_database.is_null() {
        return akar_state::AkarError;
    }

    let path = unsafe { CStr::from_ptr(database_path).to_string_lossy().into_owned() };

    let config = SystemConfig {
        buffer_pool_size: system_config.buffer_pool_size,
        max_num_threads: system_config.max_num_threads,
        enable_compression: system_config.enable_compression,
        read_only: system_config.read_only,
        max_db_size: system_config.max_db_size,
        auto_checkpoint: system_config.auto_checkpoint,
        checkpoint_threshold: system_config.checkpoint_threshold as i64,
        concurrent_writes: true, // Default
        spill_threshold: 0,      // Default
    };

    match Database::new(&path, config) {
        Ok(db) => {
            let db_arc = Arc::new(db);
            let arc_ptr = Box::into_raw(Box::new(db_arc));
            unsafe {
                (*out_database)._database = arc_ptr as *mut c_void;
            }
            akar_state::AkarSuccess
        }
        Err(_) => akar_state::AkarError,
    }
}

/// Destroys an Akar database previously created by `akar_database_init`.
///
/// # Safety
///
/// - `database` must be a valid pointer to a `akar_database` previously
///   initialized by `akar_database_init`. After this call, the inner
///   pointer is nulled out; calling this function twice on the same
///   database is safe (second call is a no-op).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn akar_database_destroy(database: *mut akar_database) {
    if !database.is_null() {
        unsafe {
            let ptr = (*database)._database;
            if !ptr.is_null() {
                let _ = Box::from_raw(ptr as *mut Arc<Database>);
                (*database)._database = ptr::null_mut();
            }
        }
    }
}

/// Creates a new connection from an Akar database.
///
/// # Safety
///
/// - `database` must be a valid pointer to a `akar_database` previously
///   initialized by `akar_database_init`.
/// - `out_connection` must point to a valid, mutable `akar_connection` struct
///   whose `_connection` field will be set to an opaque pointer. The caller
///   must later call `akar_connection_destroy` to free resources.
/// - Both pointers must remain valid for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn akar_connection_init(
    database: *mut akar_database,
    out_connection: *mut akar_connection,
) -> akar_state {
    unsafe {
        if database.is_null() || out_connection.is_null() || (*database)._database.is_null() {
            return akar_state::AkarError;
        }

        let db_arc_ptr = (*database)._database as *mut Arc<Database>;
        let db_ref = &*db_arc_ptr;

        let conn = Connection::new(db_ref);
        let conn_box = Box::new(conn);
        (*out_connection)._connection = Box::into_raw(conn_box) as *mut c_void;
        akar_state::AkarSuccess
    }
}

/// Destroys a connection previously created by `akar_connection_init`.
///
/// # Safety
///
/// - `connection` must be a valid pointer to a `akar_connection` previously
///   initialized by `akar_connection_init`. After this call, the inner
///   pointer is nulled out; calling this function twice on the same
///   connection is safe (second call is a no-op).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn akar_connection_destroy(connection: *mut akar_connection) {
    if !connection.is_null() {
        unsafe {
            let ptr = (*connection)._connection;
            if !ptr.is_null() {
                let _ = Box::from_raw(ptr as *mut Connection);
                (*connection)._connection = ptr::null_mut();
            }
        }
    }
}

/// Executes a Cypher query on the given connection.
///
/// # Safety
///
/// - `connection` must be a valid pointer to a `akar_connection` previously
///   initialized by `akar_connection_init`.
/// - `query` must be a valid null-terminated C string.
/// - `out_query_result` must point to a valid, mutable `akar_query_result`
///   struct that will be populated with the query result.
/// - All pointers must remain valid for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn akar_connection_query(
    connection: *mut akar_connection,
    query: *const c_char,
    out_query_result: *mut akar_query_result,
) -> akar_state {
    unsafe {
        if connection.is_null() || query.is_null() || out_query_result.is_null() || (*connection)._connection.is_null()
        {
            return akar_state::AkarError;
        }

        let conn_ref = &mut *((*connection)._connection as *mut Connection);
        let q_str = CStr::from_ptr(query).to_string_lossy();

        match conn_ref.query(&q_str) {
            Ok(result) => {
                let res_box = Box::new(result);
                (*out_query_result)._query_result = Box::into_raw(res_box) as *mut c_void;
                (*out_query_result)._is_owned_by_cpp = false;
                akar_state::AkarSuccess
            }
            Err(_) => akar_state::AkarError,
        }
    }
}
