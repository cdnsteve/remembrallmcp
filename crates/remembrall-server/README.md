# remembrall-server

MCP server for [RemembrallMCP](https://github.com/cdnsteve/remembrallmcp) - persistent knowledge memory and code dependency graph for AI agents.

## MCP Tools

**Memory:** `remembrall_store`, `remembrall_recall`, `remembrall_update`, `remembrall_delete`, `remembrall_ingest_github`, `remembrall_ingest_docs`

**Code Intelligence:** `remembrall_index`, `remembrall_impact`, `remembrall_lookup_symbol`

## Quick Start

```bash
# Install
cargo install remembrall-server

# Initialize (sets up Postgres + schema + embedding model)
remembrall init

# Add to .mcp.json
# { "mcpServers": { "remembrall": { "command": "remembrall" } } }
```

See the [full documentation](https://github.com/cdnsteve/remembrallmcp) for Docker Compose setup, benchmarks, and configuration.

## License

MIT
