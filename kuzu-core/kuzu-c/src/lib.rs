use std::ffi::{CStr, c_char, c_void};
use std::ptr;
use std::sync::Arc;
use kuzu_main::{Database, Connection, SystemConfig};

#[repr(C)]
pub struct kuzu_system_config {
    pub buffer_pool_size: u64,
    pub max_num_threads: u64,
    pub enable_compression: bool,
    pub read_only: bool,
    pub max_db_size: u64,
    pub auto_checkpoint: bool,
    pub checkpoint_threshold: u64,
}

#[repr(C)]
pub struct kuzu_database {
    _database: *mut c_void,
}

#[repr(C)]
pub struct kuzu_connection {
    _connection: *mut c_void,
}

#[repr(C)]
pub struct kuzu_query_result {
    _query_result: *mut c_void,
    _is_owned_by_cpp: bool,
}

#[repr(C)]
pub enum kuzu_state {
    KuzuSuccess = 0,
    KuzuError = 1,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuzu_default_system_config() -> kuzu_system_config {
    let config = SystemConfig::default();
    kuzu_system_config {
        buffer_pool_size: config.buffer_pool_size,
        max_num_threads: config.max_num_threads,
        enable_compression: config.enable_compression,
        read_only: config.read_only,
        max_db_size: config.max_db_size,
        auto_checkpoint: config.auto_checkpoint,
        checkpoint_threshold: config.checkpoint_threshold as u64,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuzu_database_init(
    database_path: *const c_char,
    system_config: kuzu_system_config,
    out_database: *mut kuzu_database,
) -> kuzu_state {
    if database_path.is_null() || out_database.is_null() {
        return kuzu_state::KuzuError;
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
            kuzu_state::KuzuSuccess
        }
        Err(_) => kuzu_state::KuzuError,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuzu_database_destroy(database: *mut kuzu_database) {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuzu_connection_init(
    database: *mut kuzu_database,
    out_connection: *mut kuzu_connection,
) -> kuzu_state {
    unsafe {
        if database.is_null() || out_connection.is_null() || (*database)._database.is_null() {
            return kuzu_state::KuzuError;
        }
        
        let db_arc_ptr = (*database)._database as *mut Arc<Database>;
        let db_ref = &*db_arc_ptr;
        
        let conn = Connection::new(db_ref);
        let conn_box = Box::new(conn);
        (*out_connection)._connection = Box::into_raw(conn_box) as *mut c_void;
        kuzu_state::KuzuSuccess
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuzu_connection_destroy(connection: *mut kuzu_connection) {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kuzu_connection_query(
    connection: *mut kuzu_connection,
    query: *const c_char,
    out_query_result: *mut kuzu_query_result,
) -> kuzu_state {
    unsafe {
        if connection.is_null() || query.is_null() || out_query_result.is_null() || (*connection)._connection.is_null() {
            return kuzu_state::KuzuError;
        }
        
        let conn_ref = &mut *((*connection)._connection as *mut Connection);
        let q_str = CStr::from_ptr(query).to_string_lossy();
        
        match conn_ref.query(&q_str) {
            Ok(result) => {
                let res_box = Box::new(result);
                (*out_query_result)._query_result = Box::into_raw(res_box) as *mut c_void;
                (*out_query_result)._is_owned_by_cpp = false;
                kuzu_state::KuzuSuccess
            }
            Err(_) => kuzu_state::KuzuError,
        }
    }
}
