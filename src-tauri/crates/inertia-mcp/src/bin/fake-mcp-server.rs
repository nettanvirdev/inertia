//! A minimal MCP server, for testing the client against a real process.
//!
//! Deliberately not a mock object: it is spawned, it speaks newline-delimited
//! JSON over real pipes, and it misbehaves in the specific ways real servers
//! do - printing a banner to stdout before the protocol starts, and logging to
//! stderr. Those are the things a mock cannot test.

use std::io::{BufRead, Write};

fn main() {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    // Real servers print banners, and a client that treats stdout as pure
    // protocol chokes on them.
    println!("fake-mcp-server starting up...");
    let _ = stdout.flush();
    eprintln!("[fake] ready");

    for line in stdin.lock().lines().map_while(Result::ok) {
        let Ok(message) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };

        let method = message
            .get("method")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();
        let id = message.get("id").cloned();

        // A notification expects no reply.
        let Some(id) = id else {
            if method == "notifications/initialized" {
                eprintln!("[fake] initialized");
            }
            continue;
        };

        let result = match method.as_str() {
            "initialize" => serde_json::json!({
                "protocolVersion": "2025-06-18",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "fake", "version": "1.0.0" }
            }),

            "tools/list" => serde_json::json!({
                "tools": [
                    {
                        "name": "echo",
                        "description": "Echoes its argument back.",
                        "inputSchema": {
                            "type": "object",
                            // A `$ref` and a `$schema`, so the client's
                            // sanitising is exercised against something real.
                            "$schema": "https://json-schema.org/draft/2020-12/schema",
                            "properties": {
                                "text": { "type": "string" },
                                "projectPath": { "type": "string" }
                            },
                            "required": ["text"]
                        }
                    },
                    {
                        "name": "fails",
                        "description": "Always reports an error.",
                        "inputSchema": { "type": "object", "properties": {} }
                    }
                ]
            }),

            "tools/call" => {
                let name = message
                    .pointer("/params/name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("");
                let arguments = message
                    .pointer("/params/arguments")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);

                match name {
                    "fails" => serde_json::json!({
                        "content": [{ "type": "text", "text": "it went wrong" }],
                        "isError": true
                    }),
                    _ => {
                        let text = arguments
                            .get("text")
                            .and_then(|t| t.as_str())
                            .unwrap_or("");
                        let folder = arguments
                            .get("projectPath")
                            .and_then(|t| t.as_str())
                            .unwrap_or("");
                        serde_json::json!({
                            "content": [
                                { "type": "text", "text": format!("echoed: {text}") },
                                { "type": "text", "text": format!("folder: {folder}") }
                            ]
                        })
                    }
                }
            }

            other => {
                let error = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": format!("no method {other}") }
                });
                println!("{error}");
                let _ = stdout.flush();
                continue;
            }
        };

        let reply = serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result });
        println!("{reply}");
        let _ = stdout.flush();
    }
}
