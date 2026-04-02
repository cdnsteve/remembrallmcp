//! Tool modules for RemembrallMCP.
//!
//! Each module contains parameter structs and logic helper functions.
//! The `#[tool]` wrapper methods live in `lib.rs` because `#[tool_router]`
//! requires all annotated methods in a single `impl` block.

pub mod graph;
pub mod ingest;
pub mod memory;
