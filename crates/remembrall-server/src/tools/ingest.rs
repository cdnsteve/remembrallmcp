//! Ingest tool parameter structs and implementation helpers.
//!
//! Covers: remembrall_ingest_github, remembrall_ingest_docs.
//! The `#[tool]` wrapper methods live in `lib.rs` (required by `#[tool_router]`).

use std::sync::Arc;

use rmcp::{ErrorData as McpError, model::*, schemars};
use serde_json::json;
use walkdir::WalkDir;

use remembrall_core::{
    embed::Embedder,
    memory::{
        store::MemoryStore,
        types::{CreateMemory, MemoryType, Scope, Source},
    },
};

// ---------------------------------------------------------------------------
// Parameter structs
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct IngestGithubParams {
    #[schemars(description = "GitHub repo in owner/repo format (e.g. 'owner/repo')")]
    pub repo: String,
    #[schemars(description = "Maximum number of recent merged PRs to ingest (default 50, max 200)")]
    pub limit: Option<u32>,
    #[schemars(description = "Project name to tag memories with (defaults to the repo name)")]
    pub project: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct IngestDocsParams {
    #[schemars(description = "Path to project directory to scan for markdown files")]
    pub path: String,
    #[schemars(description = "Project name to tag memories with")]
    pub project: Option<String>,
}

// ---------------------------------------------------------------------------
// Logic helpers
// ---------------------------------------------------------------------------

pub async fn ingest_github_impl(
    memory: &Arc<MemoryStore>,
    embedder: &Arc<dyn Embedder>,
    params: IngestGithubParams,
) -> Result<CallToolResult, McpError> {
    let IngestGithubParams { repo, limit, project } = params;
    let limit = limit.unwrap_or(50).min(200);

    let parts: Vec<&str> = repo.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|p| {
            p.is_empty()
                || !p
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        })
    {
        return Err(McpError::invalid_params(
            "repo must be in 'owner/repo' format (alphanumeric, hyphens, underscores, dots only)",
            None,
        ));
    }

    let project = project.unwrap_or_else(|| {
        repo.split('/').last().unwrap_or("unknown").to_string()
    });

    let output = tokio::process::Command::new("gh")
        .args([
            "pr", "list",
            "--repo", &repo,
            "--state", "merged",
            "--limit", &limit.to_string(),
            "--json", "number,title,body,mergedAt,author,url",
        ])
        .output()
        .await
        .map_err(|e| McpError::internal_error(
            format!("Failed to run gh CLI. Is GitHub CLI installed and on PATH? Error: {e}"),
            None,
        ))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(McpError::internal_error(
            format!("gh CLI failed: {stderr}"),
            None,
        ));
    }

    let prs: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout)
        .map_err(|e| McpError::internal_error(format!("Failed to parse gh output: {e}"), None))?;

    let mut ingested = 0u32;
    let mut skipped = 0u32;
    let mut errors = 0u32;

    for pr in &prs {
        let title = pr["title"].as_str().unwrap_or("");
        let body = pr["body"].as_str().unwrap_or("");
        let number = pr["number"].as_u64().unwrap_or(0);
        let url = pr["url"].as_str().unwrap_or("");
        let author = pr["author"]
            .as_object()
            .and_then(|a| a["login"].as_str())
            .unwrap_or("unknown");

        if body.trim().len() < 50 {
            skipped += 1;
            continue;
        }

        let content = format!("PR #{number}: {title}\n\n{body}");

        let fingerprint = remembrall_core::memory::store::compute_fingerprint_pub(&content);
        match memory.find_by_fingerprint(&fingerprint).await {
            Ok(Some(_)) => {
                skipped += 1;
                continue;
            }
            Err(e) => {
                tracing::warn!("fingerprint check failed for PR #{number}: {e}");
                errors += 1;
                continue;
            }
            Ok(None) => {}
        }

        let embedder_arc = Arc::clone(embedder);
        let content_clone = content.clone();
        let embedding = match tokio::task::spawn_blocking(move || embedder_arc.embed(&content_clone)).await {
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

        match memory.store(input, embedding).await {
            Ok(_) => ingested += 1,
            Err(e) => {
                tracing::warn!("store failed for PR #{number}: {e}");
                errors += 1;
            }
        }
    }

    let text = json!({
        "repo": repo,
        "project": project,
        "total_prs": prs.len(),
        "ingested": ingested,
        "skipped": skipped,
        "errors": errors,
    })
    .to_string();

    Ok(CallToolResult::success(vec![Content::text(text)]))
}

pub async fn ingest_docs_impl(
    memory: &Arc<MemoryStore>,
    embedder: &Arc<dyn Embedder>,
    params: IngestDocsParams,
) -> Result<CallToolResult, McpError> {
    let IngestDocsParams { path, project } = params;

    let project_name = project.unwrap_or_else(|| {
        std::path::Path::new(&path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string()
    });

    const SKIP_DIRS: &[&str] = &[
        "node_modules", ".git", "vendor", "target", ".venv", "__pycache__",
        ".tox", "dist", "build", ".cache", ".next", ".nuxt",
    ];

    let md_paths: Vec<std::path::PathBuf> = WalkDir::new(&path)
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

    let files_scanned = md_paths.len();
    let mut sections_ingested: u32 = 0;
    let mut skipped: u32 = 0;
    let mut errors: u32 = 0;

    for file_path in &md_paths {
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
                skipped += 1;
                continue;
            }
        };

        let display_name = file_path
            .strip_prefix(&path)
            .unwrap_or(file_path)
            .display()
            .to_string();

        let file_stem = file_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        let memory_type = classify_memory_type(&file_stem);

        let importance = match memory_type {
            MemoryType::Architecture => 0.8,
            MemoryType::Guideline => 0.7,
            _ => 0.6,
        };

        let sections = split_markdown_sections(&content, &display_name);

        for (summary, section_content) in sections {
            if section_content.trim().len() < 200 {
                skipped += 1;
                continue;
            }

            let fingerprint =
                remembrall_core::memory::store::compute_fingerprint_pub(&section_content);
            match memory.find_by_fingerprint(&fingerprint).await {
                Ok(Some(_)) => {
                    skipped += 1;
                    continue;
                }
                Err(e) => {
                    tracing::warn!("fingerprint check failed for {display_name}: {e}");
                    errors += 1;
                    continue;
                }
                Ok(None) => {}
            }

            let embedder_arc = Arc::clone(embedder);
            let content_for_embed = section_content.clone();
            let embedding = match tokio::task::spawn_blocking(move || {
                embedder_arc.embed(&content_for_embed)
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

            match memory.store(input, embedding).await {
                Ok(_) => sections_ingested += 1,
                Err(e) => {
                    tracing::warn!("store failed for {display_name}: {e}");
                    errors += 1;
                }
            }
        }
    }

    let text = json!({
        "path": path,
        "project": project_name,
        "files_scanned": files_scanned,
        "sections_ingested": sections_ingested,
        "skipped": skipped,
        "errors": errors,
    })
    .to_string();

    Ok(CallToolResult::success(vec![Content::text(text)]))
}

// ---------------------------------------------------------------------------
// Markdown ingestion helpers
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
            if !current_lines.is_empty() {
                let body = current_lines.join("\n");
                let summary = match &current_header {
                    Some(h) => format!("{file_name}: {h}"),
                    None => file_name.to_string(),
                };
                sections.push((summary, body));
                current_lines.clear();
            }
            let title = line.trim_start_matches('#').trim().to_string();
            current_header = Some(title);
        }
        current_lines.push(line);
    }

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

/// Map filename stem to a `MemoryType`.
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
