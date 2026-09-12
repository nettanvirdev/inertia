//! `glob` and `grep`.
//!
//! Both walk the workspace through the ripgrep crates, which means `.gitignore`
//! is honoured. That is not a nicety: a search that returns four thousand hits
//! from `node_modules` or `target` is worse than no search at all, because it
//! buries the real answer and costs a fortune in context.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use globset::{Glob, GlobMatcher};
use ignore::WalkBuilder;
use inertia_core::tool::{PermissionRequest, Tool, ToolContext, ToolOutcome, ToolSource};
use inertia_core::{Error, Result};
use serde_json::{json, Value};

/// Results beyond this are dropped, with the count reported so the model knows
/// to narrow rather than assuming it saw everything.
const MAX_RESULTS: usize = 200;
/// Files larger than this are not searched line by line.
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

fn walker(root: &Path) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .parents(true);
    builder
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
        // Forward slashes regardless of platform, so a pattern the model
        // writes works the same way on Windows.
        .replace('\\', "/")
}

// ── glob ────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct GlobTool;

#[async_trait]
impl Tool for GlobTool {
    fn id(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "Find files by name pattern, for example \"src/**/*.rs\" or \"**/*.test.ts\". \
         Returns paths, not contents. Files ignored by git are skipped. Use grep to \
         search inside files."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern, matched against paths relative to the working folder."
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search under. Defaults to the working folder."
                }
            },
            "required": ["pattern"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        let target = args
            .get("pattern")
            .and_then(Value::as_str)
            .unwrap_or(inertia_core::permission::ANY);
        // Listing filenames is as low-consequence as reading, and shares the
        // key so one grant covers looking around.
        PermissionRequest::new("read", target).with_always(inertia_core::permission::ANY)
    }

    fn render(&self, args: &Value) -> Option<String> {
        args.get("pattern")
            .and_then(Value::as_str)
            .map(|p| format!("Finding {p}"))
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutcome> {
        let pattern = args
            .get("pattern")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidInput("pattern is required.".into()))?;

        let matcher: GlobMatcher = Glob::new(pattern)
            .map_err(|e| Error::InvalidInput(format!("`{pattern}` is not a valid glob: {e}")))?
            .compile_matcher();

        let base = match args.get("path").and_then(Value::as_str) {
            Some(relative) => ctx.root.join(relative),
            None => ctx.root.clone(),
        };

        let root = ctx.root.clone();
        let found = tokio::task::spawn_blocking(move || {
            let mut found: Vec<String> = Vec::new();
            for entry in walker(&base).build().flatten() {
                if !entry.file_type().is_some_and(|t| t.is_file()) {
                    continue;
                }
                let path = relative(&root, entry.path());
                if matcher.is_match(path.as_str()) {
                    found.push(path);
                }
            }
            found.sort();
            found
        })
        .await
        .map_err(|e| Error::Other(e.to_string()))?;

        let total = found.len();
        let shown: Vec<String> = found.into_iter().take(MAX_RESULTS).collect();

        let mut output = if total == 0 {
            format!("No files match `{pattern}`.")
        } else {
            shown.join("\n")
        };
        if total > shown.len() {
            output.push_str(&format!(
                "\n\n[{total} files matched; showing the first {}. Narrow the pattern to see the rest.]",
                shown.len()
            ));
        }

        Ok(ToolOutcome {
            title: Some(format!("{total} files")),
            output,
            metadata: Some(json!({ "matches": total })),
            images: Vec::new(),
        })
    }
}

// ── grep ────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
    fn id(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Search file contents for a regular expression and return matching lines \
         with their file and line number. Files ignored by git are skipped. Use the \
         include parameter to restrict the search to particular files, for example \
         \"*.rs\"."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Regular expression to search for." },
                "path": { "type": "string", "description": "Directory to search under." },
                "include": { "type": "string", "description": "Only search files matching this glob." },
                "ignoreCase": { "type": "boolean", "default": false, "description": "Match case-insensitively." }
            },
            "required": ["pattern"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        let target = args
            .get("pattern")
            .and_then(Value::as_str)
            .unwrap_or(inertia_core::permission::ANY);
        PermissionRequest::new("read", target).with_always(inertia_core::permission::ANY)
    }

    fn render(&self, args: &Value) -> Option<String> {
        args.get("pattern")
            .and_then(Value::as_str)
            .map(|p| format!("Searching for {p}"))
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutcome> {
        let pattern = args
            .get("pattern")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidInput("pattern is required.".into()))?;
        let ignore_case = args
            .get("ignoreCase")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let regex = regex::RegexBuilder::new(pattern)
            .case_insensitive(ignore_case)
            .build()
            .map_err(|e| {
                Error::InvalidInput(format!("`{pattern}` is not a valid regular expression: {e}"))
            })?;

        let include = match args.get("include").and_then(Value::as_str) {
            Some(glob) => Some(
                Glob::new(glob)
                    .map_err(|e| {
                        Error::InvalidInput(format!("`{glob}` is not a valid glob: {e}"))
                    })?
                    .compile_matcher(),
            ),
            None => None,
        };

        let base = match args.get("path").and_then(Value::as_str) {
            Some(relative) => ctx.root.join(relative),
            None => ctx.root.clone(),
        };

        let root = ctx.root.clone();
        let (hits, files) = tokio::task::spawn_blocking(move || {
            let mut hits: Vec<String> = Vec::new();
            let mut files = 0usize;

            for entry in walker(&base).build().flatten() {
                if !entry.file_type().is_some_and(|t| t.is_file()) {
                    continue;
                }
                let path = relative(&root, entry.path());
                if let Some(include) = &include {
                    if !include.is_match(path.as_str()) {
                        continue;
                    }
                }
                // A huge file is almost always generated or vendored, and
                // reading it costs more than the hit is worth.
                if entry
                    .metadata()
                    .map(|m| m.len() > MAX_FILE_BYTES)
                    .unwrap_or(false)
                {
                    continue;
                }
                // Binary files decode to noise; lossy reading then matching
                // would produce hits nobody can act on.
                let Ok(contents) = std::fs::read_to_string(entry.path()) else {
                    continue;
                };

                let mut matched_here = false;
                for (number, line) in contents.lines().enumerate() {
                    if regex.is_match(line) {
                        matched_here = true;
                        let shown = if line.len() > 300 {
                            // Cut on a character boundary; slicing a
                            // multi-byte character in half panics.
                            let mut at = 300;
                            while at > 0 && !line.is_char_boundary(at) {
                                at -= 1;
                            }
                            format!("{}…", &line[..at])
                        } else {
                            line.to_string()
                        };
                        hits.push(format!("{path}:{}: {}", number + 1, shown.trim_end()));
                        if hits.len() >= MAX_RESULTS {
                            break;
                        }
                    }
                }
                if matched_here {
                    files += 1;
                }
                if hits.len() >= MAX_RESULTS {
                    break;
                }
            }

            (hits, files)
        })
        .await
        .map_err(|e| Error::Other(e.to_string()))?;

        let mut output = if hits.is_empty() {
            format!("No matches for `{pattern}`.")
        } else {
            hits.join("\n")
        };
        if hits.len() >= MAX_RESULTS {
            output.push_str(&format!(
                "\n\n[stopped at {MAX_RESULTS} matches. Narrow the pattern or use include to see the rest.]"
            ));
        }

        Ok(ToolOutcome {
            title: Some(format!("{} matches in {files} files", hits.len())),
            output,
            metadata: Some(json!({ "matches": hits.len(), "files": files })),
            images: Vec::new(),
        })
    }
}

pub fn search_tools() -> Vec<Arc<dyn Tool>> {
    vec![Arc::new(GlobTool), Arc::new(GrepTool)]
}
