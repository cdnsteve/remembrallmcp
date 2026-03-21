# Engram

Knowledge memory layer for AI agents. Rust core, Postgres + pgvector backend, MCP protocol.

## Quick Reference

- **Language:** Rust 1.94+, edition 2024
- **Workspace:** `crates/engram-core` (library), `crates/engram-server` (MCP server + CLI)
- **Database:** `postgres://postgres:postgres@localhost:5450/engram` (Docker: `cocoindex-postgres`)
- **Schema:** `engram` (configurable via `ENGRAM_SCHEMA`)
- **Architecture doc:** `ARCHITECTURE.md` in project root

## Build & Run

```bash
# Start database
docker start cocoindex-postgres

# Build everything
cargo build

# Build MCP server + CLI (release)
cargo build -p engram-server --release

# Run MCP server manually (stdio)
DATABASE_URL="postgres://postgres:postgres@localhost:5450/engram" ./target/release/engram

# Run ground truth tests (validates everything works)
DATABASE_URL="postgres://postgres:postgres@localhost:5450/engram" cargo run --bin spike3
```

## CLI Commands

```bash
engram init                          # Set up Docker DB, schema, embedding model, write config
engram init --database-url <url>     # Init with existing Postgres
engram serve                         # Run MCP server (explicit form; no-arg is same)
engram start                         # Start Docker database container
engram stop                          # Stop Docker database container
engram status                        # Memory count, symbol count, connection status
engram doctor                        # Check Docker, pgvector, schema, and model cache
engram reset --force                 # Drop and recreate schema (deletes all data)
engram version                       # Print version, arch, OS, and config path
```

## MCP Server

Binary: `target/release/engram`. Configured in `.mcp.json` for Claude Code.

**Tools (9 total):**
- `engram_store` - store decisions, patterns, knowledge
- `engram_recall` - hybrid semantic + full-text search
- `engram_update` - partial update of an existing memory
- `engram_delete` - remove a memory by UUID
- `engram_ingest_github` - bulk-import merged PRs from a GitHub repo (via `gh` CLI)
- `engram_ingest_docs` - ingest markdown files from a directory
- `engram_impact` - blast radius analysis on code symbols
- `engram_lookup_symbol` - find where a function/class is defined
- `engram_index` - index a project directory to build the code graph

**Embedding:** fastembed (ONNX Runtime, all-MiniLM-L6-v2, 384-dim). In-process, no external API. Model downloads on first run (~23 MB), or pre-downloaded by `engram init`.

## Project Structure

```
crates/
  engram-core/src/
    memory/store.rs      # MemoryStore - CRUD + semantic/fulltext/hybrid search
    memory/types.rs      # Memory, Source, Scope, MemoryType enums
    graph/store.rs       # GraphStore - symbols, relationships, impact analysis (recursive CTEs)
    graph/types.rs       # Symbol, Relationship, ImpactResult, Direction
    parser/python.rs     # Tree-sitter Python parser
    parser/typescript.rs # Tree-sitter TypeScript/JS parser
    parser/rust.rs       # Tree-sitter Rust parser
    parser/go.rs         # Tree-sitter Go parser
    parser/java.rs       # Tree-sitter Java parser
    parser/ruby.rs       # Tree-sitter Ruby parser
    parser/kotlin.rs     # Tree-sitter Kotlin parser
    parser/walker.rs     # Directory walker + two-phase cross-file resolution
    indexer.rs           # Incremental indexer with mtime tracking + CodeParser trait
    config.rs            # Config from env vars
    embed.rs             # Embedder trait + FastEmbedder (fastembed/ONNX, 384-dim)
    search.rs            # Hybrid search stub (logic lives in memory/store.rs)
    ingest.rs            # Ingestion stub (logic lives in engram-server/src/lib.rs)
  engram-server/
    src/lib.rs           # MCP server - 9 tools
    src/main.rs          # CLI entry point (init, serve, start, stop, status, doctor, reset, version)
    src/config.rs        # EngramConfig - loads ~/.engram/config.toml with env var overrides
  engram-python/         # PyO3 bindings (deferred)
install.sh               # curl installer script
dist/                    # Prebuilt release binaries
```

## Config File

`~/.engram/config.toml` - written by `engram init`, loaded by all subcommands. Env vars override:
- `ENGRAM_DATABASE_URL` or `DATABASE_URL` overrides `database.url`
- `ENGRAM_SCHEMA` overrides `database.schema`

## Database Tables (all in `engram` schema)

- `memories` - text knowledge with pgvector embeddings, scope, tags, fingerprint dedup
- `symbols` - code symbols (File, Function, Class, Method) with file/line/language/project
- `relationships` - edges (Calls, Imports, Defines, Inherits) with confidence scores
- `file_index` - mtime tracking for incremental reindexing

## Key Patterns

- **Two-phase resolution:** Parser collects all files first, then resolves imports and cross-file calls against the full symbol set
- **Impact analysis:** Recursive CTEs with cycle detection, confidence decay through the chain
- **Incremental indexing:** Compare disk mtime vs stored mtime, only reparse changed files
- **Content fingerprinting:** Normalized hash for memory deduplication
- **Contradiction detection:** `engram_store` searches at 0.75 similarity before storing; near-duplicates are returned in the response
- **Ingestion:** `engram_ingest_github` shells to `gh` CLI; `engram_ingest_docs` walks directories for `.md` files, splits on H2 headers

## Conventions

- No `unwrap()` in library code - use `Result<T>` with `thiserror`/`anyhow`
- All database operations go through `MemoryStore` or `GraphStore` - no raw SQL elsewhere
- Schema name is never hardcoded - always use `self.schema` with format strings
- Spike binaries (`src/bin/spike*.rs`) are throwaway validation code, not production
- Tree-sitter parsing is all Rust - no Python in the pipeline
- Ingestion tool logic (GitHub, docs) lives in `engram-server/src/lib.rs` as MCP tool implementations, not in `engram-core/src/ingest.rs` (that file is a stub)
