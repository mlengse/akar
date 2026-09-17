# Deep Exploration — akar-cli

`akar-cli` is the interactive REPL shell: rustyline-based, supporting eight output formatters and a handful of dot-commands. It is the simplest host binding — a single binary that opens a database directory and loops.

## Key Components

| Component | Responsibility | Located at |
|-----------|----------------|-----------|
| `CliState` | Open database, maintain connection | `akar-core/akar-cli/src/lib.rs` |
| OutputMode | Table/Csv/Json/Line/Column/Box/Html/Latex | `akar-core/akar-cli/src/` |
| dot-commands | `.mode`, `.tables`, `.schema`, `.import`, `.export`, `.help`, `.exit`/`.quit` | `akar-core/akar-cli/src/` |
| history | Persistent readline history | dirs::data_dir()/akar/history.txt |
| single positional arg | Database directory path (optional) | `akar-core/akar-cli/src/main.rs` |

## Design Decisions

- **No argument parser.** Single optional positional arg (the database path) means no clap dependency — keeps startup time instant and the binary small (17 tests).
- **Rustyline-based, not a custom TUI.** Rustyline handles line editing, history, and Ctrl-C; the CLI stays in stdin/stdout, making it scriptable and composable.

## Why It Matters

The CLI is the easiest way to explore a database without writing Python or Rust — and the formatters matter for developers copying results into reports. `.schema` and `.tables` in particular make it the primary diagnostic tool.