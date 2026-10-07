//! The tool registry: one call path for every tool, from every source.
//!
//! The agent loop holds this and nothing else tool-shaped. It cannot reach a
//! [`Tool`] directly, which means it cannot skip the permission check even by
//! accident - the only way to run anything is through [`Registry::run`], and
//! that always asks.
//!
//! The pipeline, in order, and each step's reason for existing:
//!
//!   1. **parse** the raw argument string, repairing what is repairable.
//!   2. **normalize** - the tool's own chance to fix near-miss arguments.
//!   3. **validate** against the schema. Failing here never runs the tool and
//!      never asks permission, so a malformed call cannot provoke a prompt.
//!   4. **ask** permission.
//!   5. **execute**.
//!   6. **truncate** the output, so one runaway call cannot evict the
//!      conversation from the context window.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use inertia_core::message::ToolCall;
use inertia_core::permission::Action;
use inertia_core::provider::ToolSpec;
use inertia_core::tool::{
    PermissionGate, ProviderProblem, Tool, ToolContext, ToolProvider, ToolRegistry, ToolResult,
};
use inertia_core::Result;
use parking_lot::Mutex;

use crate::schema;
use crate::truncate::{self, DEFAULT_LIMIT};

/// Tunables, so tests can shrink limits instead of generating megabytes.
#[derive(Debug, Clone)]
pub struct Limits {
    /// Byte ceiling on a single tool result.
    pub output_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            output_bytes: DEFAULT_LIMIT,
        }
    }
}

pub struct Registry {
    builtins: Vec<Arc<dyn Tool>>,
    providers: Vec<Arc<dyn ToolProvider>>,
    permissions: Arc<dyn PermissionGate>,
    limits: Limits,
    problems: Mutex<Vec<ProviderProblem>>,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("builtins", &self.builtins.len())
            .field("providers", &self.providers.len())
            .finish_non_exhaustive()
    }
}

impl Registry {
    pub fn new(permissions: Arc<dyn PermissionGate>) -> Self {
        Self {
            builtins: Vec::new(),
            providers: Vec::new(),
            permissions,
            limits: Limits::default(),
            problems: Mutex::new(Vec::new()),
        }
    }

    pub fn with_tool(mut self, tool: Arc<dyn Tool>) -> Self {
        self.builtins.push(tool);
        self
    }

    pub fn with_tools(mut self, tools: impl IntoIterator<Item = Arc<dyn Tool>>) -> Self {
        self.builtins.extend(tools);
        self
    }

    /// Adds a dynamic source - an MCP server, an imported OpenAPI document.
    pub fn with_provider(mut self, provider: Arc<dyn ToolProvider>) -> Self {
        self.providers.push(provider);
        self
    }

    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Every tool currently available, sorted by id.
    ///
    /// A source that fails is **dropped, not propagated**: a misconfigured MCP
    /// server must not take down a turn that was not going to use it. The
    /// failure is recorded for the UI instead.
    async fn collect(&self, root: &std::path::Path) -> Vec<Arc<dyn Tool>> {
        let mut tools = self.builtins.clone();
        let mut problems = Vec::new();

        for provider in &self.providers {
            match provider.tools(root).await {
                Ok(found) => tools.extend(found),
                Err(e) => problems.push(ProviderProblem {
                    source: provider.name().to_string(),
                    message: e.to_string(),
                }),
            }
        }

        *self.problems.lock() = problems;

        // Sorting is not cosmetic. Two turns whose tool lists differ only in
        // order share no cached prompt prefix, which silently doubles the cost
        // of every conversation that adds or loses a tool mid-session.
        tools.sort_by(|a, b| a.id().cmp(b.id()));

        // A duplicate id would make dispatch ambiguous. First registration
        // wins, so a builtin cannot be shadowed by a remote server claiming
        // its name.
        let mut seen = HashSet::new();
        tools.retain(|t| seen.insert(t.id().to_string()));

        tools
    }

    /// Every tool currently available, as the tools themselves.
    ///
    /// `specs()` answers the model's question - name, description, schema -
    /// and deliberately drops everything else. Grouping tools for on-demand
    /// loading needs what a spec does not carry: which source a tool came from
    /// and which server or app within it. Rather than widen `ToolSpec` for one
    /// caller, that caller asks here.
    pub async fn live(&self, root: &std::path::Path) -> Vec<Arc<dyn Tool>> {
        self.collect(root).await
    }

    async fn find(&self, root: &std::path::Path, name: &str) -> Option<Arc<dyn Tool>> {
        self.collect(root)
            .await
            .into_iter()
            .find(|t| t.id() == name)
    }
}

#[async_trait]
impl ToolRegistry for Registry {
    async fn specs(&self) -> Vec<ToolSpec> {
        // The registry is asked for specs before a call exists, so there is no
        // context to take a root from; providers that need one cache against
        // the workspace they were built for.
        let root = std::path::Path::new(".");
        let tools = self.collect(root).await;

        let mut specs = Vec::with_capacity(tools.len());
        for tool in tools {
            // A tool denied unconditionally is not offered at all. One denied
            // only for certain arguments stays visible, because the model can
            // still call it correctly for everything else.
            let key = tool.permission(&serde_json::Value::Null).key;
            if self
                .permissions
                .verdict(&key, inertia_core::permission::ANY)
                .await
                == Action::Deny
            {
                continue;
            }
            specs.push(ToolSpec::new(
                tool.id(),
                tool.description(),
                tool.parameters(),
            ));
        }
        specs
    }

    async fn run(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult> {
        let started = Instant::now();
        let elapsed = |started: Instant| started.elapsed().as_millis() as u64;

        let Some(tool) = self.find(&ctx.root, &call.name).await else {
            // Naming what does exist would be better, but the list can be
            // hundreds long; the model is better served by re-reading the
            // tools it was given.
            return Ok(ToolResult::failed(
                call.id.clone(),
                &call.name,
                format!("No tool named `{}` exists.", call.name),
            ));
        };

        // 1. Parse.
        let raw = match schema::parse_arguments(&call.arguments) {
            Ok(value) => value,
            Err(message) => {
                return Ok(invalid(call, tool.id(), &message, elapsed(started)));
            }
        };

        // 2. Normalize - the tool's own repair pass, which must not fail.
        let normalized = tool.normalize(raw);

        // 3. Validate. Note this precedes the permission ask: a malformed call
        //    must not be able to raise a prompt, or a model could interrupt the
        //    user with nonsense.
        let args = match schema::validate(&tool.parameters(), normalized) {
            Ok(value) => value,
            Err(message) => {
                let message = format!("Invalid arguments for {}: {}", tool.id(), message);
                return Ok(invalid(call, tool.id(), &message, elapsed(started)));
            }
        };

        // 4. Ask.
        let request = tool.permission(&args);
        let decision = self.permissions.ask(&request).await?;
        if !decision.is_allowed() {
            return Ok(ToolResult {
                title: tool.render(&args),
                metadata: Some(serde_json::json!({ "error": "denied" })),
                duration_ms: elapsed(started),
                ..ToolResult::failed(
                    call.id.clone(),
                    tool.id(),
                    format!("Permission to run `{}` was refused.", tool.id()),
                )
            });
        }

        // 5. Execute.
        let mut call_ctx = ctx.clone();
        call_ctx.call_id = call.id.clone();

        match tool.execute(args.clone(), &call_ctx).await {
            Ok(outcome) => {
                // 6. Truncate.
                let capped = truncate::truncate(outcome.output, self.limits.output_bytes);

                let mut metadata = outcome.metadata;
                if let Some(t) = capped.truncation {
                    let extra = serde_json::json!({
                        "truncated": true,
                        "originalBytes": t.original_bytes,
                        "droppedBytes": t.dropped_bytes,
                    });
                    metadata = Some(match metadata {
                        Some(serde_json::Value::Object(mut map)) => {
                            if let serde_json::Value::Object(add) = extra {
                                map.extend(add);
                            }
                            serde_json::Value::Object(map)
                        }
                        _ => extra,
                    });
                }

                Ok(ToolResult {
                    call_id: call.id.clone(),
                    tool: tool.id().to_string(),
                    ok: true,
                    title: outcome.title.or_else(|| tool.render(&args)),
                    output: capped.output,
                    metadata,
                    images: outcome
                        .images
                        .into_iter()
                        .filter(|s| !s.is_empty())
                        .collect(),
                    duration_ms: elapsed(started),
                })
            }
            // Cancellation is the one failure that unwinds. Everything else is
            // a result the model reads and reacts to; there is nobody left to
            // read this one.
            Err(e) if e.is_cancellation() => Err(e),
            Err(e) => Ok(ToolResult {
                title: tool.render(&args),
                duration_ms: elapsed(started),
                ..ToolResult::failed(call.id.clone(), tool.id(), e.to_string())
            }),
        }
    }

    async fn problems(&self) -> Vec<ProviderProblem> {
        self.problems.lock().clone()
    }
}

fn invalid(call: &ToolCall, tool: &str, message: &str, duration_ms: u64) -> ToolResult {
    ToolResult {
        title: Some(tool.to_string()),
        metadata: Some(serde_json::json!({ "error": "invalid-arguments" })),
        duration_ms,
        ..ToolResult::failed(call.id.clone(), tool, message)
    }
}

/// Convenience for callers that hold a concrete `Registry` but need the trait
/// object the agent loop takes.
pub fn shared(registry: Registry) -> Arc<dyn ToolRegistry> {
    Arc::new(registry)
}
