<!-- mem-cli:start -->
## mem-cli context storage

Project context is stored locally per developer, outside the repository:
`${XDG_DATA_HOME:-~/.local/share}/mem/<slug>/project_context.db`.
The project slug (`rfc-cli-b3b906143933a80a`) is fixed in the `.mem-project` file.
The path can be overridden via the `MEMORY_DB_DIR` variable.

Use the `mcp mem-cli` server for context memory; if `mcp mem-cli` is not
connected, fall back to the `mem-cli` shell command.
<!-- mem-cli:end -->
