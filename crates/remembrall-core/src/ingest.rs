//! Ingestion pipeline: bulk-import memories from GitHub PRs and markdown docs.
//!
//! These functions contain the core logic extracted from the MCP server so any
//! frontend (MCP server, CLI, HTTP API, etc.) can reuse them without duplicating
//! the implementation.
//!
//! # Thin-wrapper pattern for the MCP server
//!
//! The server tool methods (`remembrall_ingest_github`, `remembrall_ingest_docs`)
//! should resolve their optional parameters, call the corresponding function here,
//! and convert the returned `IngestResult` (or `anyhow::Error`) into an MCP
//! `CallToolResult`. Example sketch:
//!
//! ```ignore
//! async fn remembrall_ingest_github(&self, Parameters(p): Parameters<IngestGithubParams>)
//!     -> Result<CallToolResult, McpError>
//! {
//!     let project = p.project.unwrap_or_else(|| {
//!         p.repo.split('/').last().unwrap_or("unknown").to_string()
//!     });
//!     let result = ingest::ingest_github_prs(
//!         &p.repo,
//!         p.limit.map(|l| l as i32),
//!         Some(&project),
//!         &self.memory,
//!         Arc::clone(&self.embedder),
//!     ).await.map_err(|e| McpError::internal_error(e.to_string(), None))?;
//!     let text = serde_json::to_string(&result).unwrap();
//!     Ok(CallToolResult::success(vec![Content::text(text)]))
//! }
//! ```

use std::sync::Arc;

use anyhow::Result;
use serde::Serialize;
use walkdir::WalkDir;

use crate::{
    embed::Embedder,
    memory::{
        store::{compute_fingerprint_pub, MemoryStore},
        types::{CreateMemory, MemoryType, Scope, Source},
    },
};

// `Arc<dyn Embedder>` is used rather than `&dyn Embedder` because the
// `spawn_blocking` closures require `'static` bounds. Callers already hold the
// embedder as an `Arc`, so this avoids any extra allocation.

// ---------------------------------------------------------------------------
// Public result type
// ---------------------------------------------------------------------------

/// Summary returned by both ingestion functions.
#[derive(Debug, Serialize)]
pub struct IngestResult {
    /// How many items were successfully stored as new memories.
    pub memories_stored: u64,
    /// How many items were skipped (too short, duplicate fingerprint, etc.).
    pub memories_skipped: u64,
    /// How many items failed due to an error (embedding, DB write, etc.).
    pub errors: u64,
    /// Human-readable labels for items that were skipped due to duplication.
    /// Populated only when the caller asks for verbose output; otherwise empty.
    pub duplicate_labels: Vec<String>,
}

// ---------------------------------------------------------------------------
// GitHub PR ingestion
// ---------------------------------------------------------------------------

/// Ingest merged pull requests from a GitHub repository as memories.
///
/// Shells out to the `gh` CLI (must be installed and authenticated on PATH).
/// Each PR body is stored as a separate memory. PRs with bodies shorter than
/// 50 characters and already-seen content fingerprints are skipped silently.
///
/// `limit` caps how many recent merged PRs are fetched (default 50, hard max 200).
/// `project` is used to tag memories; defaults to the repository name segment.
pub async fn ingest_github_prs(
    repo: &str,
    limit: Option<i32>,
    project: Option<&str>,
    memory_store: &MemoryStore,
    embedder: Arc<dyn Embedder>,
) -> Result<IngestResult> {
    let limit = limit.unwrap_or(50).min(200).max(1) as u32;

    // Validate repo is in "owner/repo" format before passing to the shell.
    let parts: Vec<&str> = repo.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|p| {
            p.is_empty()
                || !p
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        })
    {
        anyhow::bail!(
            "repo must be in 'owner/repo' format (alphanumeric, hyphens, underscores, dots only)"
        );
    }

    let project = project
        .map(|p| p.to_string())
        .unwrap_or_else(|| repo.split('/').last().unwrap_or("unknown").to_string());

    // Shell out to gh CLI - already authenticated on the user's machine.
    let output = tokio::process::Command::new("gh")
        .args([
            "pr",
            "list",
            "--repo",
            repo,
            "--state",
            "merged",
            "--limit",
            &limit.to_string(),
            "--json",
            "number,title,body,mergedAt,author,url",
        ])
        .output()
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "Failed to run gh CLI. Is GitHub CLI installed and on PATH? Error: {e}"
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("gh CLI failed: {stderr}");
    }

    let prs: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout)
        .map_err(|e| anyhow::anyhow!("Failed to parse gh output: {e}"))?;

    let mut memories_stored: u64 = 0;
    let mut memories_skipped: u64 = 0;
    let mut errors: u64 = 0;

    for pr in &prs {
        let title = pr["title"].as_str().unwrap_or("");
        let body = pr["body"].as_str().unwrap_or("");
        let number = pr["number"].as_u64().unwrap_or(0);
        let url = pr["url"].as_str().unwrap_or("");
        let author = pr["author"]
            .as_object()
            .and_then(|a| a["login"].as_str())
            .unwrap_or("unknown");

        // Skip PRs with empty or very short bodies.
        if body.trim().len() < 50 {
            memories_skipped += 1;
            continue;
        }

        let content = format!("PR #{number}: {title}\n\n{body}");

        // Dedup by content fingerprint before touching the embedder.
        let fingerprint = compute_fingerprint_pub(&content);
        match memory_store.find_by_fingerprint(&fingerprint).await {
            Ok(Some(_)) => {
                memories_skipped += 1;
                continue;
            }
            Err(e) => {
                tracing::warn!("fingerprint check failed for PR #{number}: {e}");
                errors += 1;
                continue;
            }
            Ok(None) => {}
        }

        // Generate embedding (fastembed is sync/CPU-bound).
        let embedder_clone = Arc::clone(&embedder);
        let content_clone = content.clone();
        let embedding =
            match tokio::task::spawn_blocking(move || embedder_clone.embed(&content_clone)).await {
                Ok(Ok(emb)) => emb,
                Ok(Err(e)) => {
                    tracing::warn!("embedding failed for PR #{number}: {e}");
                    errors += 1;
                    continue;
                }
                Err(e) => {
                    tracing::warn!("spawn_blocking panicked for PR #{number}: {e}");
                    errors += 1;
                    continue;
                }
            };

        // Classify memory type by title keywords.
        let title_lower = title.to_lowercase();
        let memory_type = if title_lower.contains("fix") || title_lower.contains("bug") {
            MemoryType::ErrorPattern
        } else if title_lower.contains("refactor") {
            MemoryType::Pattern
        } else {
            MemoryType::Decision
        };

        let input = CreateMemory {
            content,
            summary: Some(format!("PR #{number}: {title}")),
            memory_type,
            source: Source {
                system: "github".to_string(),
                identifier: url.to_string(),
                author: Some(author.to_string()),
            },
            scope: Scope {
                organization: None,
                team: None,
                project: Some(project.clone()),
            },
            tags: vec!["github".to_string(), "pull-request".to_string()],
            metadata: None,
            importance: Some(0.6),
            expires_at: None,
        };

        match memory_store.store(input, embedding).await {
            Ok(_) => memories_stored += 1,
            Err(e) => {
                tracing::warn!("store failed for PR #{number}: {e}");
                errors += 1;
            }
        }
    }

    Ok(IngestResult {
        memories_stored,
        memories_skipped,
        errors,
        duplicate_labels: Vec::new(),
    })
}

// ---------------------------------------------------------------------------
// Markdown docs ingestion
// ---------------------------------------------------------------------------

/// Ingest markdown files from a directory tree as memories.
///
/// Walks `path` recursively (skipping hidden dirs and common noise dirs),
/// reads every `.md` file, splits it on H2 (`## `) headers, and stores each
/// section as a separate memory. Sections shorter than 200 characters and
/// already-seen content fingerprints are skipped.
///
/// `project` is used to tag memories; defaults to the directory's basename.
pub async fn ingest_docs(
    path: &str,
    project: Option<&str>,
    memory_store: &MemoryStore,
    embedder: Arc<dyn Embedder>,
) -> Result<IngestResult> {
    let project_name = project
        .map(|p| p.to_string())
        .unwrap_or_else(|| {
            std::path::Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string()
        });

    // Directories to skip - these are never meaningful documentation sources.
    const SKIP_DIRS: &[&str] = &[
        "node_modules",
        ".git",
        "vendor",
        "target",
        ".venv",
        "__pycache__",
        ".tox",
        "dist",
        "build",
        ".cache",
        ".next",
        ".nuxt",
    ];

    // Collect markdown file paths without following symlinks.
    let md_paths: Vec<std::path::PathBuf> = WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            if e.file_type().is_dir() {
                if name.starts_with('.') {
                    return false;
                }
                if SKIP_DIRS.iter().any(|&d| d == name.as_ref()) {
                    return false;
                }
            }
            true
        })
        .filter_map(|entry| entry.ok())
        .filter(|e| {
            e.file_type().is_file()
                && e.path()
                    .extension()
                    .map(|ext| ext.eq_ignore_ascii_case("md"))
                    .unwrap_or(false)
        })
        .map(|e| e.into_path())
        .collect();

    let mut memories_stored: u64 = 0;
    let mut memories_skipped: u64 = 0;
    let mut errors: u64 = 0;

    for file_path in &md_paths {
        // Read file, skip gracefully on UTF-8 or I/O errors.
        let raw = match std::fs::read(file_path) {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::warn!("skipping {}: read error: {e}", file_path.display());
                errors += 1;
                continue;
            }
        };
        let content = match std::str::from_utf8(&raw) {
            Ok(s) => s.to_string(),
            Err(_) => {
                tracing::debug!("skipping {} (not valid UTF-8)", file_path.display());
                memories_skipped += 1;
                continue;
            }
        };

        // Derive a short display name relative to the scanned root.
        let display_name = file_path
            .strip_prefix(path)
            .unwrap_or(file_path)
            .display()
            .to_string();

        let file_stem = file_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        // Classify memory type by filename convention.
        let memory_type = classify_memory_type(&file_stem);

        // Importance: architecture docs and ADRs are higher value.
        let importance = match memory_type {
            MemoryType::Architecture => 0.8,
            MemoryType::Guideline => 0.7,
            _ => 0.6,
        };

        let sections = split_markdown_sections(&content, &display_name);

        for (summary, section_content) in sections {
            // Skip sections that are too short to be meaningful.
            if section_content.trim().len() < 200 {
                memories_skipped += 1;
                continue;
            }

            // Dedup by content fingerprint.
            let fingerprint = compute_fingerprint_pub(&section_content);
            match memory_store.find_by_fingerprint(&fingerprint).await {
                Ok(Some(_)) => {
                    memories_skipped += 1;
                    continue;
                }
                Err(e) => {
                    tracing::warn!("fingerprint check failed for {display_name}: {e}");
                    errors += 1;
                    continue;
                }
                Ok(None) => {}
            }

            // Generate embedding (fastembed is sync/CPU-bound).
            let embedder_clone = Arc::clone(&embedder);
            let content_for_embed = section_content.clone();
            let embedding = match tokio::task::spawn_blocking(move || {
                embedder_clone.embed(&content_for_embed)
            })
            .await
            {
                Ok(Ok(emb)) => emb,
                Ok(Err(e)) => {
                    tracing::warn!("embedding failed for {display_name}: {e}");
                    errors += 1;
                    continue;
                }
                Err(e) => {
                    tracing::warn!("spawn_blocking panicked for {display_name}: {e}");
                    errors += 1;
                    continue;
                }
            };

            let tags = vec![
                "docs".to_string(),
                "markdown".to_string(),
                file_stem.to_lowercase().replace([' ', '/'], "-"),
            ];

            let input = CreateMemory {
                content: section_content,
                summary: Some(summary),
                memory_type: memory_type.clone(),
                source: Source {
                    system: "ingest_docs".to_string(),
                    identifier: file_path.display().to_string(),
                    author: None,
                },
                scope: Scope {
                    organization: None,
                    team: None,
                    project: Some(project_name.clone()),
                },
                tags,
                metadata: None,
                importance: Some(importance),
                expires_at: None,
            };

            match memory_store.store(input, embedding).await {
                Ok(_) => memories_stored += 1,
                Err(e) => {
                    tracing::warn!("store failed for {display_name}: {e}");
                    errors += 1;
                }
            }
        }
    }

    Ok(IngestResult {
        memories_stored,
        memories_skipped,
        errors,
        duplicate_labels: Vec::new(),
    })
}

// ---------------------------------------------------------------------------
// Markdown helpers
// ---------------------------------------------------------------------------

/// Split a markdown document into sections on `## ` (H2) boundaries.
///
/// Each section becomes `(summary, content)` where summary is
/// `"filename: Section Title"` and content is the section text including
/// the header line. If the file has no H2 headers the whole file is returned
/// as a single section with summary `"filename"`.
pub fn split_markdown_sections(content: &str, file_name: &str) -> Vec<(String, String)> {
    let mut sections: Vec<(String, String)> = Vec::new();
    let mut current_header: Option<String> = None;
    let mut current_lines: Vec<&str> = Vec::new();

    for line in content.lines() {
        if line.starts_with("## ") {
            // Flush current accumulator.
            if !current_lines.is_empty() {
                let body = current_lines.join("\n");
                let summary = match &current_header {
                    Some(h) => format!("{file_name}: {h}"),
                    None => file_name.to_string(),
                };
                sections.push((summary, body));
                current_lines.clear();
            }
            // Start a new section.
            let title = line.trim_start_matches('#').trim().to_string();
            current_header = Some(title);
        }
        current_lines.push(line);
    }

    // Flush the final accumulator.
    if !current_lines.is_empty() {
        let body = current_lines.join("\n");
        let summary = match &current_header {
            Some(h) => format!("{file_name}: {h}"),
            None => file_name.to_string(),
        };
        sections.push((summary, body));
    }

    sections
}

/// Map a filename stem to a `MemoryType`.
///
/// Rules (case-insensitive):
/// - `ARCHITECTURE`, `DESIGN`, files ending in `-adr` or `-decision` -> Architecture
/// - `CONTRIBUTING`, `STYLE`, `CODE_OF_CONDUCT`, files containing `guideline` -> Guideline
/// - Everything else -> CodeContext
pub fn classify_memory_type(stem: &str) -> MemoryType {
    let lower = stem.to_lowercase();
    if lower.contains("architecture")
        || lower.contains("design")
        || lower.ends_with("-adr")
        || lower.ends_with("-decision")
        || lower.starts_with("adr-")
        || lower.starts_with("adr_")
    {
        return MemoryType::Architecture;
    }
    if lower.contains("contributing")
        || lower.contains("guideline")
        || lower.contains("style")
        || lower.contains("code_of_conduct")
        || lower.contains("conduct")
    {
        return MemoryType::Guideline;
    }
    MemoryType::CodeContext
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- split_markdown_sections ---

    #[test]
    fn test_split_no_h2_returns_single_section() {
        let content = "# Title\n\nSome content here.\n\nMore content.";
        let sections = split_markdown_sections(content, "readme");
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].0, "readme");
        assert!(sections[0].1.contains("Some content here."));
    }

    #[test]
    fn test_split_two_h2_sections() {
        let content = "## Overview\n\nFirst section.\n\n## Details\n\nSecond section.";
        let sections = split_markdown_sections(content, "doc.md");
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].0, "doc.md: Overview");
        assert!(sections[0].1.contains("First section."));
        assert_eq!(sections[1].0, "doc.md: Details");
        assert!(sections[1].1.contains("Second section."));
    }

    #[test]
    fn test_split_preamble_before_first_h2() {
        let content = "Preamble text.\n\n## Section\n\nBody.";
        let sections = split_markdown_sections(content, "file");
        // Preamble becomes its own section (no header), then the H2 section.
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].0, "file"); // no header prefix
        assert_eq!(sections[1].0, "file: Section");
    }

    #[test]
    fn test_split_empty_content() {
        let sections = split_markdown_sections("", "empty");
        // No lines - no sections emitted.
        assert_eq!(sections.len(), 0);
    }

    // --- classify_memory_type ---

    #[test]
    fn test_classify_architecture() {
        assert!(matches!(
            classify_memory_type("ARCHITECTURE"),
            MemoryType::Architecture
        ));
        assert!(matches!(
            classify_memory_type("design-overview"),
            MemoryType::Architecture
        ));
        assert!(matches!(
            classify_memory_type("adr-001-use-postgres"),
            MemoryType::Architecture
        ));
        assert!(matches!(
            classify_memory_type("adr_002"),
            MemoryType::Architecture
        ));
        assert!(matches!(
            classify_memory_type("choose-redis-decision"),
            MemoryType::Architecture
        ));
    }

    #[test]
    fn test_classify_guideline() {
        assert!(matches!(
            classify_memory_type("CONTRIBUTING"),
            MemoryType::Guideline
        ));
        assert!(matches!(
            classify_memory_type("style-guide"),
            MemoryType::Guideline
        ));
        assert!(matches!(
            classify_memory_type("code_of_conduct"),
            MemoryType::Guideline
        ));
        assert!(matches!(
            classify_memory_type("guidelines"),
            MemoryType::Guideline
        ));
    }

    #[test]
    fn test_classify_code_context_default() {
        assert!(matches!(
            classify_memory_type("README"),
            MemoryType::CodeContext
        ));
        assert!(matches!(
            classify_memory_type("changelog"),
            MemoryType::CodeContext
        ));
        assert!(matches!(
            classify_memory_type("setup"),
            MemoryType::CodeContext
        ));
    }
}
