# Copilot Instructions

Akar is a pure-Rust embedded graph database. The Rust workspace lives in `akar-core/`.

Follow the operational rules in [`AGENTS.md`](../AGENTS.md): use RustRover MCP tools
(`execute_terminal_command`, `read_file`, `search_regex`, `search_symbol`,
`get_file_problems`, `lint_files`) rather than a raw PowerShell shell, and treat the
`test [akar-core]` run config as the authoritative gate signal.
