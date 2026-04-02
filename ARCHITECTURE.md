# RemembrallMCP Architecture

*Last updated: March 2026*

## What is RemembrallMCP?

RemembrallMCP is a knowledge memory layer for AI agents. It gives any MCP-compatible agent persistent memory - decisions, patterns, code relationships, and organizational context that survives between sessions.

**The problem:** Every AI agent tool (Copilot, Cursor, Devin) is stateless. Every session starts from zero. Agents have no memory of past decisions, team preferences, error patterns, or how the codebase fits together.

**The solution:** RemembrallMCP is a Rust-native memory engine backed by Postgres + pgvector. It stores two kinds of knowledge:

1. **Text memories** - decisions, patterns, error fixes, preferences, guidelines
2. **Code graph** - functions, classes, imports, call chains, and impact analysis

Agents query RemembrallMCP via MCP to get relevant context before acting.

---

## System Overview

```
Source Code                   Organizational Knowledge
    |                                 |
    v                                 v
+-------------------+    +------------------------+
| Tree-sitter       |    | Ingestion Pipeline     |
| Parser            |    | (GitHub PRs, Markdown) |
| (8 languages)     |    |                        |
+--------+----------+    +-----------+------------+
         |                            |
         v                            v
+--------------------------------------------------+
|              Postgres + pgvector                  |
|                                                   |
|  remembrall.memories          remembrall.symbols          |
|  - content + embedding    - name, type, file      |
|  - semantic search (HNSW) - language, project     |
|  - full-text search       - line numbers          |
|  - scope (org/team/proj)  - signature             |
|  - fingerprint dedup                              |
|                           remembrall.relationships    |
|  remembrall.file_index        - source -> target      |
|  - mtime tracking         - Calls, Imports,       |
|  - incremental reindex      Defines, Inherits     |
|                           - confidence scoring    |
+-------------------------+------------------------+
                          |
                          v
              +-----------------------+
              |    MCP Server         |
              |    (remembrall-server)    |
              |                       |
              |  9 tools (see below)  |
              +-----------+-----------+
                          |
              +-----------+-----------+
              | Any MCP Client        |
              | Claude Code, Cursor,  |
              | Copilot, custom agents|
              +------------------------+
```

---

## Crate Structure

```
remembrallmcp/
  Cargo.toml                    # Workspace root
  install.sh                    # curl installer script
  crates/
    remembrall-core/                # Library - all logic lives here
      src/
        lib.rs                  # Module exports
        config.rs               # Config from env vars
        error.rs                # Error types
        embed.rs                # Embedder trait + FastEmbedder (fastembed/ONNX, 384-dim)
        memory/
          types.rs              # Memory, Source, Scope, MemoryType
          store.rs              # MemoryStore - CRUD + search
        graph/
          types.rs              # Symbol, Relationship, ImpactResult
          store.rs              # GraphStore - upsert + recursive CTE impact analysis
        parser/
          mod.rs                # Public API: parse_python_file, parse_ts_file, index_directory
          python.rs             # Tree-sitter Python parser
          typescript.rs         # Tree-sitter TypeScript/JS parser
          rust.rs               # Tree-sitter Rust parser
          go.rs                 # Tree-sitter Go parser
          java.rs               # Tree-sitter Java parser
          ruby.rs               # Tree-sitter Ruby parser
          kotlin.rs             # Tree-sitter Kotlin parser
          walker.rs             # Directory walker + two-phase cross-file resolution
        indexer.rs              # Incremental indexer with mtime tracking + CodeParser trait
        search.rs               # Hybrid search stub
        ingest.rs               # Ingestion stub
      src/bin/
        spike.rs                # Spike 1: memory + graph benchmarks
        spike2.rs               # Spike 2: multi-project regex indexing
        spike3.rs               # Ground truth: 10 real-world correctness tests
        parser_smoke.rs         # Tree-sitter parser validation
        kt_ast_debug.rs         # Kotlin AST debugging utility
    remembrall-server/              # MCP server + CLI binary
      src/
        lib.rs                  # 9 MCP tools (store, recall, update, delete,
                                #   ingest_github, ingest_docs, impact, lookup, index)
        main.rs                 # CLI entry point (init, serve, start, stop, status, doctor, reset, version)
        config.rs               # RemembrallConfig - loads ~/.remembrall/config.toml with env var overrides
    remembrall-python/              # PyO3 bindings (deferred until PyO3 supports Python 3.14)
      src/lib.rs
    remembrall-test-harness/        # Parser quality testing
      src/
        main.rs
        comparator.rs
        ground_truth.rs
        scorer.rs
    remembrall-recall-test/         # Search quality testing
      src/
        main.rs
        ground_truth.rs
        report.rs
        runner.rs
        scorer.rs
        seed.rs
```

---

## Core Components

### MemoryStore (`memory/store.rs`)

Postgres-backed storage for text memories with pgvector embeddings.

**Tables:** `remembrall.memories` (content, embedding, scope, tags, metadata, fingerprint, importance, expiry)

**Key operations:**
- `store(input, embedding)` - store a memory with its vector embedding
- `search_semantic(embedding, limit, min_similarity, scope)` - cosine similarity via HNSW index
- `search_fulltext(query, limit)` - Postgres tsvector search
- `search_hybrid(embedding, query)` - RRF fusion of semantic + full-text results
- `get(id)` - fetch by ID, auto-increments access_count
- `get_readonly(id)` - fetch by ID without incrementing access_count
- `update(id, content, summary, tags, importance, embedding)` - partial update
- `find_by_fingerprint(hash)` - deduplication check
- `delete(id)`, `count(scope)`

**Indexes:** HNSW (vector cosine), GIN (full-text, tags), B-tree (type, scope, fingerprint)

### GraphStore (`graph/store.rs`)

Code relationship graph stored as Postgres adjacency tables.

**Tables:**
- `remembrall.symbols` - code symbols (File, Function, Class, Method) with file path, line numbers, language, project
- `remembrall.relationships` - edges between symbols (Calls, Imports, Defines, Inherits) with confidence scores

**Key operations:**
- `upsert_symbol(symbol)` - insert or update a symbol
- `add_relationship(rel)` - insert or update an edge
- `impact_analysis(symbol_id, direction, max_depth)` - recursive CTE traversal
- `find_symbol(name, type)` - lookup by name
- `remove_file(path, project)` - cascade delete for reindexing

**Impact analysis** uses recursive CTEs with cycle detection. Traverses upstream (who calls me?), downstream (what do I call?), or both. Confidence decays multiplicatively through the chain.

### Parser (`parser/`)

Tree-sitter based source code analysis. Pure Rust, no Python involved.

**Supported languages:** Python, TypeScript, JavaScript, Rust, Go, Ruby, Java, Kotlin

**What it extracts:**
- Symbols: functions, classes, methods, files
- Relationships: function calls, imports, class inheritance, method definitions
- Metadata: signatures, line numbers, decorators

**Two-phase resolution (`walker.rs`):**
1. Parse all files independently, collect symbols and raw import metadata
2. Build path-to-UUID map from all File symbols, then resolve:
   - Relative imports (`from ..storage import X`) by walking up directories
   - Absolute imports (`import sugar.memory.store`) by suffix matching
   - Dotted method calls (`self.queue.get_next()`) by extracting final method name
   - Cross-file calls by rewriting synthetic UUIDs to real symbol UUIDs

### Indexer (`indexer.rs`)

Incremental code indexing with mtime tracking.

**Table:** `remembrall.file_index` (file_path, project, mtime, indexed_at)

**How it works:**
1. Walk directory, collect files with disk mtimes
2. Compare against stored mtimes in `file_index`
3. Parse + store only new/changed files
4. Delete symbols for files that no longer exist on disk

**`CodeParser` trait** - plug-in interface so the indexer doesn't own parsing logic. Any language can be added by implementing `parse(file_path, source, language)`.

---

## Ingestion Pipeline

Two tools solve the cold-start problem - getting useful memories into a fresh RemembrallMCP instance without manually running `remembrall_store` for every piece of knowledge.

### GitHub PR Ingestion (`remembrall_ingest_github`)

Shells out to the `gh` CLI (already authenticated on the user's machine) to fetch merged PRs from a GitHub repository. For each PR:

1. Skip if body is under 50 characters (no useful content)
2. Check content fingerprint for deduplication - skip if already ingested
3. Embed `"PR #N: title\n\nbody"` with fastembed
4. Classify memory type from title keywords (fix/bug -> ErrorPattern, refactor -> Pattern, else -> Decision)
5. Store with `source.system = "github"`, `source.identifier = PR URL`, project scope set

Requires `gh` to be installed and authenticated. Does not need a GitHub token in the environment.

### Markdown Doc Ingestion (`remembrall_ingest_docs`)

Walks a directory tree for `.md` files (skipping `node_modules`, `.git`, `target`, `vendor`, etc.). For each file:

1. Read and validate UTF-8
2. Split on `## ` (H2 headers) - each section becomes a separate memory
3. Skip sections under 200 characters
4. Check content fingerprint for deduplication
5. Classify memory type from filename (`ARCHITECTURE`, `DESIGN`, `adr-*` -> Architecture; `CONTRIBUTING`, `STYLE` -> Guideline; else -> CodeContext)
6. Set importance: Architecture=0.8, Guideline=0.7, other=0.6
7. Store with tags `["docs", "markdown", "<filename>"]`

---

## Memory Features

### Contradiction Detection

`remembrall_store` runs a similarity search at a high threshold (0.75) before storing. If near-duplicate memories are found, they are returned in the response alongside the new memory's ID. This gives the agent the option to update the existing memory instead of creating a near-duplicate.

### Memory Decay / Access Tracking

Each memory has an `access_count` that increments on `get()`. The `get_readonly()` path is used internally (e.g., contradiction checks) to avoid inflating counts.

### Memory Update

`remembrall_update` performs a partial update - only the fields supplied are changed. If `content` is updated, a new embedding is generated automatically. This avoids the delete-and-recreate pattern.

---

## MCP Server (`remembrall-server`)

The MCP server exposes RemembrallMCP's capabilities to any MCP-compatible agent (Claude Code, Cursor, etc.) over stdio transport.

**Binary:** `target/release/remembrall` (includes ONNX Runtime for embeddings)

### Tools

| Tool | Description | Key params |
|------|-------------|------------|
| `remembrall_store` | Store knowledge, decisions, patterns | `content`, `memory_type`, `tags`, `importance`, `source_identifier` |
| `remembrall_recall` | Hybrid semantic + full-text search | `query`, `limit`, `memory_types`, `tags`, `project` |
| `remembrall_update` | Update an existing memory | `id`, `content`, `summary`, `tags`, `importance` |
| `remembrall_delete` | Remove a memory by UUID | `id` |
| `remembrall_ingest_github` | Bulk-import merged PR descriptions | `repo`, `limit`, `project` |
| `remembrall_ingest_docs` | Ingest markdown files from a directory | `path`, `project` |
| `remembrall_impact` | Blast radius analysis - what breaks if you change a symbol | `symbol_name`, `direction`, `max_depth` |
| `remembrall_lookup_symbol` | Find where a function/class is defined | `name`, `symbol_type`, `project` |
| `remembrall_index` | Index a project directory to build the code graph | `path`, `project` |

### Embedding

Uses `fastembed` (ONNX Runtime) with `all-MiniLM-L6-v2` (384-dim) for in-process embedding. No external API or Python dependency. Model downloads on first run (~23 MB), or pre-downloaded by `remembrall init`.

Pluggable via the `Embedder` trait in `remembrall-core/src/embed.rs`.

### Setup for Claude Code

The `.mcp.json` in the project root configures Claude Code to use the RemembrallMCP server:

```json
{
  "mcpServers": {
    "remembrall": {
      "command": "/Users/steve/Dev/remembrallmcp/target/release/remembrall",
      "env": {
        "DATABASE_URL": "postgres://postgres:postgres@localhost:5450/remembrall"
      }
    }
  }
}
```

Once installed via `cargo install` or the curl installer, the simpler form works:

```json
{
  "mcpServers": {
    "remembrall": {
      "command": "remembrall"
    }
  }
}
```

---

## CLI (`remembrall`)

The server binary doubles as a CLI. The default behavior (no subcommand) is to run the MCP server.

| Command | Description |
|---------|-------------|
| `remembrall init` | Set up database (Docker or external Postgres), create schema, download embedding model, write config |
| `remembrall init --database-url <url>` | Init with an existing Postgres instead of Docker |
| `remembrall serve` | Run the MCP server (explicit form of the default) |
| `remembrall start` | Start the Docker database container |
| `remembrall stop` | Stop the Docker database container |
| `remembrall status` | Show memory count, symbol count, Docker state, connection |
| `remembrall doctor` | Check config, Docker, database connection, pgvector, schema, and model cache |
| `remembrall reset --force` | Drop and recreate the schema (deletes all data) |
| `remembrall version` | Print version, arch, OS, and config path |

---

## Config File

Config lives at `~/.remembrall/config.toml`. Written by `remembrall init`, loaded by every subcommand. Environment variables override config file values.

```toml
mode = "local"   # "local" (Docker), "external" (BYO Postgres)

[database]
url = "postgres://postgres:postgres@localhost:5450/remembrall"
schema = "remembrall"
pool_size = 10

[docker]
container_name = "remembrall-db"
image = "pgvector/pgvector:pg16"
port = 5450

[embedding]
model = "all-MiniLM-L6-v2"
```

**Environment variable overrides:**

| Variable | Overrides |
|----------|-----------|
| `REMEMBRALL_DATABASE_URL` or `DATABASE_URL` | `database.url` |
| `REMEMBRALL_SCHEMA` | `database.schema` |

---

## Database

**Engine:** PostgreSQL 16 + pgvector 0.8.2

**Connection:** `postgres://postgres:postgres@localhost:5450/remembrall` (default)

**Schema:** Everything lives in the `remembrall` schema. Configurable via `REMEMBRALL_SCHEMA` env var.

**Tables:**
| Table | Purpose | Key indexes |
|-------|---------|-------------|
| `memories` | Text knowledge with embeddings | HNSW (vector), GIN (full-text, tags), B-tree (scope) |
| `symbols` | Code symbols (functions, classes, etc.) | B-tree (file_path, name+type) |
| `relationships` | Edges between symbols | B-tree (source_id, target_id) |
| `file_index` | Mtime tracking for incremental indexing | PK (file_path, project) |

---

## Running It

### Prerequisites
- Rust 1.94+
- Docker (for local Postgres + pgvector, or bring your own Postgres)

### First-time setup
```bash
cargo build -p remembrall-server --release
./target/release/remembrall init
```

### Manual (without init)
```bash
# Start existing Docker container
docker start cocoindex-postgres

# Run MCP server directly
DATABASE_URL="postgres://postgres:postgres@localhost:5450/remembrall" ./target/release/remembrall
```

### Run the ground truth tests
```bash
DATABASE_URL="postgres://postgres:postgres@localhost:5450/remembrall" cargo run --bin spike3
```

Expected output: 10/10 tests pass across Sugar (Python), Revsup (Django), NomadSignal (TypeScript).

---

## Validated Performance

| Operation | Time |
|-----------|------|
| Schema init (tables + HNSW index) | 83ms |
| Store 3 memories | 7ms |
| Get by ID | 809us |
| Semantic search (pgvector HNSW) | 942us |
| Full-text search | 573-907us |
| Impact analysis (realistic) | 4-9ms |
| Impact analysis (stress, 3,698 nodes) | 33ms |
| Find symbol by name | 476us |
| Index Sugar (89 files, 1,157 symbols, 9,297 rels) | 2.3s |
| Index Revsup (92 files, 771 symbols, 1,602 rels) | 1.2s |

---

## Ground Truth (10/10)

Real questions answered correctly against real codebases:

| # | Question | Project |
|---|----------|---------|
| 1 | What methods does MemoryStore have? (17) | Sugar |
| 2 | What calls get_next_work()? (dotted method resolution) | Sugar |
| 3 | What does loop.py import? (relative import resolution) | Sugar |
| 4 | What inherits BaseEmbedder? | Sugar |
| 5 | Blast radius of store()? | Sugar |
| 6 | What are all Django models? (8 classes) | Revsup |
| 7 | What views call ForecastService? (none - correctly identified) | Revsup |
| 8 | What Django signal handlers exist? (3) | Revsup |
| 9 | What does data-adapter.ts export? (9 functions) | NomadSignal |
| 10 | What calls getCountry()? | NomadSignal |

---

## What's Next

| Item | Status | Description |
|------|--------|-------------|
| MCP server - core tools | Done | store, recall, update, delete over stdio |
| Hybrid recall | Done | RRF fusion of semantic + full-text search |
| Ingestion pipeline | Done | GitHub PR ingestion + markdown doc ingestion |
| Memory features | Done | Contradiction detection, access tracking, partial update |
| CLI | Done | init, serve, start, stop, status, doctor, reset, version |
| Config file | Done | `~/.remembrall/config.toml` with env var overrides |
| Code graph tools | Done | impact, lookup_symbol, index - all 8 languages |
| Prebuilt binaries | In progress | macOS ARM64 binary exists in `dist/` - CI release pipeline pending |
| Concurrent load test | TODO | 50 simulated agents hitting the engine |
| Incremental indexing via MCP | TODO | Wire the Indexer mtime tracking into remembrall_index |
| Memory expiry | TODO | TTL-based decay using the `expires_at` field |
