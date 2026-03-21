# Engram

Knowledge memory layer for AI agents. Persistent organizational memory that any MCP-compatible agent can query.

**The problem:** Every AI agent tool (Copilot, Cursor, Devin) is stateless. Every session starts from zero. Agents have no memory of past decisions, team preferences, error patterns, or how the codebase fits together.

**The solution:** Engram gives agents persistent memory - decisions, patterns, code relationships, and organizational context that survives between sessions.

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
| `engram_update` | Update an existing memory (content, summary, tags, or importance) |
| `engram_delete` | Remove a memory by UUID |
| `engram_ingest_github` | Bulk-import merged PR descriptions from a GitHub repo |
| `engram_ingest_docs` | Scan a project directory for markdown files and ingest them as memories |
| `engram_impact` | Blast radius analysis - "what breaks if I change this?" |
| `engram_lookup_symbol` | Find where a function or class is defined |
| `engram_index` | Index a project directory (8 languages supported) |

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

### Install

```bash
# Install via cargo (prebuilt binaries coming soon)
cargo install --path crates/engram-server

# Or build from source
cargo build -p engram-server --release
```

### Initialize

```bash
engram init
```

This sets up a Docker-managed Postgres container with pgvector, creates the schema, and pre-downloads the embedding model. Config is written to `~/.engram/config.toml`.

To use an existing Postgres instead:

```bash
engram init --database-url postgres://user:pass@host/dbname
```

### Connect to Claude Code

Add to your project's `.mcp.json`:

```json
{
  "mcpServers": {
    "engram": {
      "command": "engram"
    }
  }
}
```

If running from source (not installed):

```json
{
  "mcpServers": {
    "engram": {
      "command": "/path/to/engram/target/release/engram",
      "env": {
        "DATABASE_URL": "postgres://postgres:postgres@localhost:5450/engram"
      }
    }
  }
}
```

Restart Claude Code. All 9 tools will be available automatically.

### Try it

```
> "Store a memory: We chose Postgres over MongoDB because our query patterns
   are relational. Type: decision, tags: database, architecture"

> "Recall what we know about database decisions"

> "Index this project and show me the impact of changing UserService"
```

## Cold Start

A new Engram instance has no knowledge. Use the ingestion tools to bootstrap from existing project history in minutes.

**From GitHub PR history:**

```
> engram_ingest_github repo="myorg/myrepo" limit=100
```

Fetches merged PRs via the GitHub CLI (`gh`), digests titles and bodies into memories, and tags them by project. Requires `gh` to be installed and authenticated. PRs with less than 50 characters of body are skipped. Deduplication by content fingerprint prevents re-ingestion on repeat runs.

**From markdown docs:**

```
> engram_ingest_docs path="/path/to/project"
```

Walks the directory tree, finds all `.md` files, splits them by H2 section headers, and stores each section as a searchable memory. Skips `node_modules`, `.git`, `target`, and similar directories. Good for README, ARCHITECTURE, ADRs, and any written docs.

Run both once per project. After ingestion, `engram_recall` has immediate context.

## Architecture

```
Source Code                   Organizational Knowledge
    |                                 |
    v                                 v
Tree-sitter Parsers           Ingestion Pipeline
(8 languages)                 (GitHub PRs, Markdown docs)
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
  engram-server/        # MCP server + CLI binary (engram)
  engram-test-harness/  # Parser quality testing
  engram-recall-test/   # Search quality testing
docs/
  parser-architecture.md
  test-plan.md
test-fixtures/          # Ground truth TOML files for 8 languages
tests/                  # Recall test fixtures (ground_truth.toml, seed_memories.toml)
dist/                   # Prebuilt release binaries
install.sh              # curl installer script
```

## CLI Commands

| Command | Description |
|---------|-------------|
| `engram init` | Set up database, schema, and embedding model |
| `engram serve` | Run the MCP server (default when no subcommand given) |
| `engram start` | Start the Docker database container |
| `engram stop` | Stop the Docker database container |
| `engram status` | Show memory count, symbol count, connection status |
| `engram doctor` | Check for common problems (Docker, pgvector, schema, model) |
| `engram reset --force` | Drop and recreate the schema (deletes all data) |
| `engram version` | Print version and config path |

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
