use std::sync::{Arc, Mutex};

use akar_catalog::Catalog;
use akar_common::file_system::VirtualFileSystemRegistry;
use akar_extension::{Extension, ExtensionContext};
use akar_function::registry::FunctionRegistry;
use akar_postgres::PostgresExtension;

fn make_context() -> ExtensionContext {
    ExtensionContext::new(
        Arc::new(Mutex::new(FunctionRegistry::new())),
        Arc::new(Mutex::new(Catalog::new())),
        Arc::new(VirtualFileSystemRegistry::new()),
    )
}

#[test]
fn test_postgres_extension_name() {
    let ext = PostgresExtension::new();
    assert_eq!(ext.name(), "POSTGRES");
}

#[test]
fn test_postgres_extension_default() {
    let ext = PostgresExtension;
    assert_eq!(ext.name(), "POSTGRES");
}

#[test]
fn test_postgres_load_registers_function() {
    let ext = PostgresExtension::new();
    let ctx = make_context();
    let result = ext.load(&ctx);
    assert!(result.is_ok());

    let reg = ctx.function_registry().lock().unwrap();
    assert!(reg.get_scalar("sql_query").is_some());
}

#[test]
fn test_postgres_placeholder_returns_error() {
    let ext = PostgresExtension::new();
    let ctx = make_context();
    ext.load(&ctx).unwrap();

    let reg = ctx.function_registry().lock().unwrap();
    let func = reg.get_scalar("sql_query").unwrap();
    if let akar_function::registry::ScalarFunction::CustomScalar { execute, .. } = func {
        let result = execute(&[]);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("not available"));
    } else {
        panic!("Expected CustomScalar variant");
    }
}

#[test]
fn test_postgres_placeholder_wrong_args() {
    let ext = PostgresExtension::new();
    let ctx = make_context();
    ext.load(&ctx).unwrap();

    let reg = ctx.function_registry().lock().unwrap();
    let func = reg.get_scalar("sql_query").unwrap();
    if let akar_function::registry::ScalarFunction::CustomScalar { execute, .. } = func {
        let result = execute(&[akar_common::types::Value::String("host=localhost".into())]);
        assert!(result.is_err());
    } else {
        panic!("Expected CustomScalar variant");
    }
}

#[test]
fn test_postgres_load_idempotent() {
    let ext = PostgresExtension::new();
    let ctx = make_context();
    ext.load(&ctx).unwrap();
    ext.load(&ctx).unwrap();

    let reg = ctx.function_registry().lock().unwrap();
    assert!(reg.get_scalar("sql_query").is_some());
}
