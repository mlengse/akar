# Migrating to Kuzu Rust (v1.0.0)

This document provides a guide for migrating from the C++ Kuzu API to the Rust Kuzu API (`kuzu-core`), focusing on the Drop-in Replacement objective (P28).

## Key API Differences

### 1. Database Initialization
**C++:**
```cpp
auto systemConfig = kuzu::main::SystemConfig();
auto db = std::make_unique<kuzu::main::Database>("path/to/db", systemConfig);
```

**Rust:**
```rust
use kuzu_main::{Database, SystemConfig};
let config = SystemConfig::default();
let db = Database::new("path/to/db", config)?; // Returns Result
```
*Note*: Rust uses standard `Result` types for error handling rather than throwing exceptions.

### 2. Connection Management
**C++:**
```cpp
auto conn = std::make_unique<kuzu::main::Connection>(db.get());
```

**Rust:**
```rust
use kuzu_main::Connection;
let conn = Connection::new(&db); // Takes an Arc or reference
```

### 3. Query Execution & Results
**C++:**
```cpp
auto result = conn->query("MATCH (a:person) RETURN a.name");
while (result->hasNext()) {
    auto tuple = result->getNext();
    std::cout << tuple->getValue(0)->getValue<std::string>() << std::endl;
}
```

**Rust:**
```rust
let result = conn.query("MATCH (a:person) RETURN a.name")?;
for chunk in &result.chunks {
    for row in chunk.iter_rows() {
        // Field access by index or name
        if let Some(val) = chunk.fields[0].get_value(row) {
            println!("{:?}", val);
        }
    }
}
```
*Note*: The Rust API leverages chunk-based processing directly exposing `DataChunk` structures, providing closer alignment with Arrow vectors for high performance (Hybrid Arrow Migration - P27).

### 4. Prepared Statements & Parameters
**C++:**
```cpp
auto preparedStatement = conn->prepare("MATCH (a:person {name: $name}) RETURN a.age");
auto result = conn->execute(preparedStatement.get(), std::make_pair("name", "Alice"));
```

**Rust:**
```rust
let stmt = conn.prepare("MATCH (a:person {name: $name}) RETURN a.age")?;
let result = conn.execute(&stmt, vec![("name", "Alice".into())])?;
```
*Note*: Parameters are passed as dynamic types that implement `Into<Value>`. 

### 5. Error Handling
All operations that can fail return a `Result<T, kuzu_common::Error>`. Instead of `try-catch` blocks, use the `?` operator or standard Rust `match` statements for robust error handling.

## Kuzu-Migrate Tool

The upcoming `kuzu-migrate` CLI tool (P28) will assist in migrating storage files directly from the C++ disk format to the pure Rust implementations format. 
Usage will roughly follow:
```bash
cargo run --bin kuzu-migrate -- --source /path/to/cpp_db --target /path/to/rust_db
```
*(Further details will be provided when the tool is finalized)*
