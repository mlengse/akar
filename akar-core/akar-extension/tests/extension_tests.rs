use std::sync::{Arc, Mutex};

use akar_catalog::Catalog;
use akar_common::file_system::VirtualFileSystemRegistry;
use akar_extension::{Extension, ExtensionContext, ExtensionRegistry};
use akar_function::registry::{AggregateFunction, FunctionRegistry, ScalarFunction, TableFunction};

struct MockExtension {
    name: &'static str,
    should_fail: bool,
}

impl MockExtension {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            should_fail: false,
        }
    }

    fn failing(name: &'static str) -> Self {
        Self {
            name,
            should_fail: true,
        }
    }
}

impl Extension for MockExtension {
    fn name(&self) -> &'static str {
        self.name
    }

    fn load(&self, _context: &ExtensionContext) -> Result<(), String> {
        if self.should_fail {
            Err(format!("{} failed to load", self.name))
        } else {
            Ok(())
        }
    }
}

struct RegisteringExtension;

impl Extension for RegisteringExtension {
    fn name(&self) -> &'static str {
        "REGISTERING"
    }

    fn load(&self, context: &ExtensionContext) -> Result<(), String> {
        context.register_scalar_function(
            "custom_scalar",
            ScalarFunction::CustomScalar {
                name: "custom_scalar".to_string(),
                execute: Arc::new(|_args| Ok(akar_common::types::Value::Int64(42))),
            },
        );
        context.register_aggregate_function("custom_agg", AggregateFunction::Count);
        context.register_table_function(
            "custom_table",
            TableFunction::CustomTable {
                name: "custom_table".to_string(),
                execute: Arc::new(|_args, _chunk| Ok(())),
            },
        );
        Ok(())
    }
}

fn make_context() -> ExtensionContext {
    ExtensionContext::new(
        Arc::new(Mutex::new(FunctionRegistry::new())),
        Arc::new(Mutex::new(Catalog::new())),
        Arc::new(VirtualFileSystemRegistry::new()),
    )
}

#[test]
fn test_registry_new_is_empty() {
    let reg = ExtensionRegistry::new();
    assert_eq!(reg.num_registered(), 0);
    assert_eq!(reg.num_loaded(), 0);
    assert!(reg.names().is_empty());
    assert!(reg.extension_names().is_empty());
}

#[test]
fn test_registry_register_single() {
    let mut reg = ExtensionRegistry::new();
    reg.register(Box::new(MockExtension::new("TEST_EXT")));
    assert_eq!(reg.num_registered(), 1);
    assert_eq!(reg.names(), vec!["TEST_EXT"]);
    assert_eq!(reg.extension_names(), vec!["TEST_EXT"]);
    assert!(!reg.is_loaded("TEST_EXT"));
}

#[test]
fn test_registry_register_multiple() {
    let mut reg = ExtensionRegistry::new();
    reg.register(Box::new(MockExtension::new("EXT_A")));
    reg.register(Box::new(MockExtension::new("EXT_B")));
    reg.register(Box::new(MockExtension::new("EXT_C")));
    assert_eq!(reg.num_registered(), 3);
    assert_eq!(reg.names(), vec!["EXT_A", "EXT_B", "EXT_C"]);
}

#[test]
fn test_registry_load_all_success() {
    let mut reg = ExtensionRegistry::new();
    reg.register(Box::new(MockExtension::new("OK1")));
    reg.register(Box::new(MockExtension::new("OK2")));

    let ctx = make_context();
    let results = reg.load_all(&ctx);

    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|(_, r)| r.is_ok()));
    assert!(reg.is_loaded("OK1"));
    assert!(reg.is_loaded("OK2"));
    assert_eq!(reg.num_loaded(), 2);
}

#[test]
fn test_registry_load_all_with_failure() {
    let mut reg = ExtensionRegistry::new();
    reg.register(Box::new(MockExtension::new("OK")));
    reg.register(Box::new(MockExtension::failing("FAIL")));
    reg.register(Box::new(MockExtension::new("OK2")));

    let ctx = make_context();
    let results = reg.load_all(&ctx);

    assert_eq!(results.len(), 3);
    let ok_count = results.iter().filter(|(_, r)| r.is_ok()).count();
    let err_count = results.iter().filter(|(_, r)| r.is_err()).count();
    assert_eq!(ok_count, 2);
    assert_eq!(err_count, 1);

    assert!(reg.is_loaded("OK"));
    assert!(!reg.is_loaded("FAIL"));
    assert!(reg.is_loaded("OK2"));
    assert_eq!(reg.num_loaded(), 2);
}

#[test]
fn test_registry_load_all_empty() {
    let mut reg = ExtensionRegistry::new();
    let ctx = make_context();
    let results = reg.load_all(&ctx);
    assert!(results.is_empty());
}

#[test]
fn test_registry_is_loaded_unknown() {
    let reg = ExtensionRegistry::new();
    assert!(!reg.is_loaded("NONEXISTENT"));
}

#[test]
fn test_registry_load_failure_does_not_mark_loaded() {
    let mut reg = ExtensionRegistry::new();
    reg.register(Box::new(MockExtension::failing("BAD")));

    let ctx = make_context();
    reg.load_all(&ctx);

    assert!(!reg.is_loaded("BAD"));
    assert_eq!(reg.num_loaded(), 0);
}

#[test]
fn test_context_scalar_function_registration() {
    let ctx = make_context();
    ctx.register_scalar_function(
        "test_scalar",
        ScalarFunction::CustomScalar {
            name: "test_scalar".to_string(),
            execute: Arc::new(|_args| Ok(akar_common::types::Value::Int64(1))),
        },
    );

    let reg = ctx.function_registry().lock().unwrap();
    assert!(reg.get_scalar("test_scalar").is_some());
    assert!(reg.contains("test_scalar"));
}

#[test]
fn test_context_aggregate_function_registration() {
    let ctx = make_context();
    ctx.register_aggregate_function("test_agg", AggregateFunction::Sum);

    let reg = ctx.function_registry().lock().unwrap();
    assert!(reg.get_aggregate("test_agg").is_some());
}

#[test]
fn test_context_table_function_registration() {
    let ctx = make_context();
    ctx.register_table_function(
        "test_table_fn",
        TableFunction::CustomTable {
            name: "test_table_fn".to_string(),
            execute: Arc::new(|_args, _chunk| Ok(())),
        },
    );

    let reg = ctx.function_registry().lock().unwrap();
    assert!(reg.get_table("test_table_fn").is_some());
}

#[test]
fn test_context_catalog_accessor() {
    let ctx = make_context();
    let catalog = ctx.catalog();
    assert_eq!(catalog.lock().unwrap().all_entries().count(), 0);
}

#[test]
fn test_extension_load_registers_functions() {
    let ctx = make_context();
    let ext = RegisteringExtension;
    let result = ext.load(&ctx);
    assert!(result.is_ok());

    let reg = ctx.function_registry().lock().unwrap();
    assert!(reg.get_scalar("custom_scalar").is_some());
    assert!(reg.get_aggregate("custom_agg").is_some());
    assert!(reg.get_table("custom_table").is_some());
}

#[test]
fn test_extension_trait_is_object_safe() {
    let ext: Box<dyn Extension> = Box::new(MockExtension::new("OBJ_SAFE"));
    assert_eq!(ext.name(), "OBJ_SAFE");
}

#[test]
fn test_registry_names_and_extension_names_consistency() {
    let mut reg = ExtensionRegistry::new();
    reg.register(Box::new(MockExtension::new("A")));
    reg.register(Box::new(MockExtension::new("B")));

    let names = reg.names();
    let ext_names = reg.extension_names();
    assert_eq!(names.len(), ext_names.len());
    for (name, ext_name) in names.iter().zip(ext_names.iter()) {
        assert_eq!(name.as_str(), *ext_name);
    }
}
