# Engram

Knowledge memory layer for AI agents. Persistent organizational memory that any MCP-compatible agent can query.

**The problem:** Every AI agent tool (Copilot, Cursor, Devin) is stateless. Every session starts from zero. Agents have no memory of past decisions, team preferences, error patterns, or how the codebase fits together.

**The solution:** Engram gives agents persistent memory - decisions, patterns, code relationships, and organizational context that survives between sessions.

## How it works

Engram runs as an MCP server that any agent can connect to. It stores two kinds of knowledge:

1. **Text memories** - decisions, patterns, error fixes, guidelines, architecture notes
2. **Code graph** - functions, classes, imports, call chains, and impact analysis across 8 languages

```
Agent starts a task
  |
  engram_recall("authentication middleware patterns")
  -> Returns 3 relevant memories from past sessions
  |
  engram_index("/path/to/project", "myapp")
  -> Builds code dependency graph
  |
  engram_impact("AuthMiddleware", direction="upstream")
  -> Shows 12 files that depend on AuthMiddleware
  |
  Agent makes the change with full context
  |
  engram_store("Switched from JWT to session tokens because...")
  -> Stores the decision for future agents
```

## MCP Tools

| Tool | Description |
|------|-------------|
| `engram_recall` | Search memories - hybrid semantic + full-text with RRF fusion |
| `engram_store` | Store decisions, patterns, knowledge with vector embeddings |
| `engram_impact` | Blast radius analysis - "what breaks if I change this?" |
| `engram_lookup_symbol` | Find where a function or class is defined |
| `engram_index` | Index a project directory (8 languages supported) |
| `engram_delete` | Remove a memory by UUID |

## Supported Languages

| Language | Extensions | Quality Score |
|----------|-----------|---------------|
| Python | .py | A (94.1) |
| Java | .java | A (92.6) |
| JavaScript | .js, .jsx | A (92.0) |
| Rust | .rs | A (91.0) |
| Go | .go | A (90.7) |
| Ruby | .rb | B (87.9) |
| TypeScript | .ts, .tsx | B (84.3) |
| Kotlin | .kt, .kts | B (82.9) |

Scores measured against real open-source projects (Click, Gson, Axios, bat, Cobra, Sidekiq, Hono, Exposed) using automated ground truth tests.

## Quick Start

### Prerequisites

- Rust 1.94+
- Docker (for Postgres + pgvector)

### Setup

```bash
# Start Postgres with pgvector
docker run -d --name engram-postgres \
  -e POSTGRES_PASSWORD=postgres \
  -p 5450:5432 \
  pgvector/pgvector:pg16

# Create the database
docker exec engram-postgres psql -U postgres -c "CREATE DATABASE engram;"

# Build
cargo build -p engram-server --release
```

### Connect to Claude Code

Add to your project's `.mcp.json`:

```json
{
  "mcpServers": {
    "engram": {
      "command": "/path/to/engram/target/release/engram-mcp",
      "env": {
        "DATABASE_URL": "postgres://postgres:postgres@localhost:5450/engram"
      }
    }
  }
}
```

Restart Claude Code. The 6 tools will be available automatically.

### Try it

```
> "Store a memory: We chose Postgres over MongoDB because our query patterns
   are relational. Type: decision, tags: database, architecture"

> "Recall what we know about database decisions"

> "Index this project and show me the impact of changing UserService"
```

## Architecture

```
Source Code                   Organizational Knowledge
    |                                 |
    v                                 v
Tree-sitter Parsers           Ingestion Pipeline
(8 languages)                 (store via MCP)
    |                                 |
    v                                 v
+--------------------------------------------------+
|              Postgres + pgvector                  |
|                                                   |
|  memories (text + embeddings + metadata)          |
|  symbols (functions, classes, methods)            |
|  relationships (calls, imports, inherits)         |
+--------------------------------------------------+
                          |
                    MCP Server (stdio)
                          |
              Claude Code / Cursor / any MCP client
```

- **Parsing:** tree-sitter (Rust bindings, no Python in the pipeline)
- **Embeddings:** fastembed (all-MiniLM-L6-v2, 384-dim, in-process ONNX)
- **Search:** Hybrid RRF (semantic cosine + full-text tsvector)
- **Graph queries:** Recursive CTEs with cycle detection
- **Transport:** stdio via rmcp

## Project Structure

```
crates/
  engram-core/          # Library - parsers, memory store, graph store, embedder
  engram-server/        # MCP server binary (engram-mcp)
  engram-test-harness/  # Parser quality testing
  engram-recall-test/   # Search quality testing
docs/
  parser-architecture.md
  test-plan.md
test-fixtures/          # Ground truth TOML files for 8 languages
```

## Performance

| Operation | Time |
|-----------|------|
| Memory store | 7ms |
| Semantic search (HNSW) | <1ms |
| Full-text search | <1ms |
| Hybrid recall (end-to-end) | ~25ms |
| Impact analysis | 4-9ms |
| Symbol lookup | <1ms |
| Index 89 Python files | 2.3s |

## License

MIT
