# Akar WASM — WebAssembly Bindings

Node.js WebAssembly bindings for the Akar embedded graph database.

## Quick Start

```bash
# Build for Node.js
wasm-pack build --target nodejs

# Run WASM tests
wasm-pack test --node
```

## API

### AkarDatabase

```js
import { AkarDatabase } from './pkg/akar_wasm.js';

// In-memory database
const db = new AkarDatabase(':memory:');

// Persistent database
const db = new AkarDatabase('/path/to/db');
```

### AkarConnection

```js
import { AkarConnection } from './pkg/akar_wasm.js';

const conn = new AkarConnection(db);

// DDL
conn.query('CREATE NODE TABLE Person(name STRING, age INT64, PRIMARY KEY(name))');

// DML
conn.query("CREATE (:Person {name: 'Alice', age: 30})");

// Query
const result = conn.query('MATCH (p:Person) RETURN p.name, p.age ORDER BY p.age');
console.log(result.getNumRows()); // 1

// Iterate rows
while (result.hasNext()) {
    const row = result.getNext();
    console.log(row); // { 'p.name': 'Alice', 'p.age': 30 }
}

// Column metadata
const cols = result.getColumnNames();
// ['p.name', 'p.age']
```

### Prepared Statements

```js
const stmt = conn.prepare('CREATE (:Person {name: $name, age: $age})');
conn.execute(stmt, { name: 'Bob', age: 25 });
```

## Browser Target

```bash
# Build for browser (requires bundler like webpack)
wasm-pack build --target bundler

# Or for direct ES module import
wasm-pack build --target web
```

## Building

```bash
# Install wasm-pack
cargo install wasm-pack

# Add WASM target
rustup target add wasm32-unknown-unknown

# Build
wasm-pack build --target nodejs

# Test
wasm-pack test --node
```

## NPM Package

```bash
cd akar-wasm/pkg
npm publish --access public
```

## License

GPL-3.0-or-later — see [LICENSE](../../LICENSE)
