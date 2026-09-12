//! The headless harness.
//!
//! Runs the real agent loop - the same code the app runs - wired to mock
//! providers, mock tools and a scripted permission gate. No API key, no
//! network, no window.
//!
//! This is the loop to develop against. A change to the turn loop, the
//! registry, or the permission engine shows up here in under a second, with
//! the whole event stream printed in order, instead of requiring a full app
//! build and a conversation with a real model to reproduce.
//!
//! ```text
//! cargo run -p inertia-devkit               # list the scenarios
//! cargo run -p inertia-devkit -- tools      # run one
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use futures::StreamExt;
use inertia_agent::{Agent, AgentConfig, AgentEvent, Turn};
use inertia_core::message::Entry;
use inertia_core::permission::{Action, Rule};
use inertia_core::tool::ToolRegistry;
use inertia_core::SessionId;
use inertia_mock::{Behaviour, MockGate, MockProvider, MockTool, Policy};
use inertia_tools::Registry;
use serde_json::json;

struct Scenario {
    name: &'static str,
    blurb: &'static str,
}

const SCENARIOS: &[Scenario] = &[
    Scenario {
        name: "chat",
        blurb: "a plain reply with no tools",
    },
    Scenario {
        name: "tools",
        blurb: "a tool round-trip, then a reply",
    },
    Scenario {
        name: "denied",
        blurb: "a tool call the user refuses",
    },
    Scenario {
        name: "invalid",
        blurb: "a call the schema rejects before it runs",
    },
    Scenario {
        name: "loop",
        blurb: "a model repeating itself until the guard fires",
    },
    Scenario {
        name: "failure",
        blurb: "the provider falling over mid-stream",
    },
];

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "inertia=debug".into()),
        )
        .with_target(false)
        .init();

    let Some(name) = std::env::args().nth(1) else {
        println!("scenarios:\n");
        for scenario in SCENARIOS {
            println!("  {:<10} {}", scenario.name, scenario.blurb);
        }
        println!("\nrun one with: cargo run -p inertia-devkit -- <name>");
        return Ok(());
    };

    match name.as_str() {
        "chat" => chat().await,
        "tools" => tools().await,
        "denied" => denied().await,
        "invalid" => invalid().await,
        "loop" => doom_loop().await,
        "failure" => failure().await,
        other => {
            anyhow::bail!("no scenario called {other:?}. Run with no arguments to list them.")
        }
    }

    Ok(())
}

// ── the scenarios ───────────────────────────────────────────────────────

async fn chat() {
    let gate = Arc::new(MockGate::allow_all());
    drive(
        "chat",
        MockProvider::new().replying("Nothing to do here - the answer is 4."),
        Registry::new(gate.clone()),
        gate,
        AgentConfig::default(),
    )
    .await;
}

async fn tools() {
    let gate = Arc::new(MockGate::allow_all());
    let registry = Registry::new(gate.clone()).with_tool(Arc::new(
        MockTool::new("read")
            .with_description("Reads a file from the workspace.")
            .with_behaviour(Behaviour::Output(
                "fn main() {\n    println!(\"hello\");\n}".into(),
            )),
    ));

    drive(
        "tools",
        MockProvider::new()
            .calling("read", json!({ "input": "src/main.rs" }))
            .replying("It prints \"hello\" and does nothing else."),
        registry,
        gate,
        AgentConfig::default(),
    )
    .await;
}

async fn denied() {
    let gate = Arc::new(MockGate::deny_all());
    let registry = Registry::new(gate.clone())
        .with_tool(Arc::new(MockTool::new("shell").with_permission_key("shell")));

    drive(
        "denied",
        MockProvider::new().calling("shell", json!({ "input": "rm -rf /" })),
        registry,
        gate,
        AgentConfig::default(),
    )
    .await;
}

async fn invalid() {
    let gate = Arc::new(MockGate::allow_all());
    // A schema with a required field, so the model's empty call is rejected
    // before the tool runs and before anyone is asked to approve it.
    let registry = Registry::new(gate.clone()).with_tool(Arc::new(
        MockTool::new("read").with_parameters(json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"],
            "additionalProperties": false,
        })),
    ));

    drive(
        "invalid",
        MockProvider::new()
            .calling("read", json!({ "file": "src/main.rs" }))
            .replying("Sorry - the argument is called `path`. Trying again."),
        registry,
        gate,
        AgentConfig::default(),
    )
    .await;
}

async fn doom_loop() {
    let gate = Arc::new(MockGate::new(Policy::Rules(vec![
        Rule::for_any("mock", Action::Allow),
        // Nobody is here to approve a repeat, so the guard refuses it.
        Rule::for_any("doom_loop", Action::Deny),
    ])));
    let registry = Registry::new(gate.clone()).with_tool(Arc::new(MockTool::new("search")));

    let mut provider = MockProvider::new();
    for _ in 0..5 {
        provider = provider.calling("search", json!({ "input": "the same query" }));
    }

    drive(
        "loop",
        provider.replying("I keep getting the same result. I need a different approach."),
        registry,
        gate,
        AgentConfig::default(),
    )
    .await;
}

async fn failure() {
    let gate = Arc::new(MockGate::allow_all());
    drive(
        "failure",
        MockProvider::new().failing("503 upstream is unavailable", Some(503)),
        Registry::new(gate.clone()),
        gate,
        AgentConfig::default(),
    )
    .await;
}

// ── the runner ──────────────────────────────────────────────────────────

async fn drive(
    name: &str,
    provider: MockProvider,
    registry: Registry,
    gate: Arc<MockGate>,
    config: AgentConfig,
) {
    println!("\n── {name} {}\n", "─".repeat(60usize.saturating_sub(name.len())));

    let agent = Agent::new(
        Arc::new(provider),
        Arc::new(registry) as Arc<dyn ToolRegistry>,
        gate.clone(),
        AgentConfig {
            model: "mock-model".into(),
            ..config
        },
    );

    let turn = Turn {
        session: SessionId::new(),
        system: "You are Inertia, running in the development harness.".into(),
        history: vec![Entry::user("What does src/main.rs do?")],
        root: PathBuf::from("."),
    };

    println!("  user  │ What does src/main.rs do?");

    let mut events = agent.run(turn);
    let mut streaming = false;

    while let Some(event) = events.next().await {
        match event {
            AgentEvent::Start { model } => println!("  start │ {model}"),
            AgentEvent::Step { step } => println!("  step  │ {step}"),
            AgentEvent::Delta { text } => {
                // Deltas arrive a fragment at a time; print them as one line
                // rather than one line each, the way a UI would.
                if !streaming {
                    print!("  model │ ");
                    streaming = true;
                }
                print!("{text}");
            }
            AgentEvent::Reasoning { text } => println!("  think │ {text}"),
            // Something typed while the turn was already running, shown where
            // it reached the model rather than where it was typed.
            AgentEvent::Steered { text } => println!("  steer │ {text}"),
            AgentEvent::ToolStarted { call, .. } => {
                end_stream(&mut streaming);
                println!("  tool  │ → {} {}", call.name, call.arguments);
            }
            AgentEvent::ToolFinished { result } => {
                let mark = if result.ok { "ok" } else { "failed" };
                println!(
                    "  tool  │ ← {} [{mark}] {}",
                    result.tool,
                    first_line(&result.output)
                );
            }
            AgentEvent::Notice { message } => {
                end_stream(&mut streaming);
                println!("  note  │ {message}");
            }
            AgentEvent::Done {
                stopped,
                history,
                usage,
            } => {
                end_stream(&mut streaming);
                println!("  done  │ {stopped:?}");
                println!("        │ {} entries in the transcript", history.len());
                if let Some(usage) = usage {
                    println!(
                        "        │ {} in, {} out",
                        usage.total_input(),
                        usage.output_tokens
                    );
                }
            }
        }
    }

    let asked = gate.asked();
    if !asked.is_empty() {
        println!("\n  permission requests:");
        for request in asked {
            println!("        │ {} → {}", request.key, request.target);
        }
    }
    println!();
}

fn end_stream(streaming: &mut bool) {
    if *streaming {
        println!();
        *streaming = false;
    }
}

fn first_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default();
    if line.len() > 70 {
        format!("{}…", &line[..70])
    } else {
        line.to_string()
    }
}
