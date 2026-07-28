use anyhow::Result;
use serde_json::Value;
use std::io::{BufRead, Write};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::client::OrbitClient;
use crate::dispatch_tool;

/// Run the MCP server using stdio transport (newline-delimited JSON-RPC 2.0).
/// Reads from stdin, writes responses to stdout. This is the default MCP transport.
pub async fn run_stdio(client: Arc<Mutex<OrbitClient>>) -> Result<()> {
    let stdin  = std::io::stdin();
    let stdout = std::io::stdout();
    let mut stdout_lock = stdout.lock();

    // NOTE: The server does NOT send notifications/initialized proactively.
    // The client sends notifications/initialized to the server after receiving
    // the server's initialize response.  The server just listens.

    for line in stdin.lock().lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let response = process_message(line, client.clone()).await;
        if let Some(resp) = response {
            writeln!(stdout_lock, "{}", serde_json::to_string(&resp)?)?;
            stdout_lock.flush()?;
        }
    }

    Ok(())
}

/// Process a single JSON-RPC 2.0 message and return an optional response.
/// Notifications (no id) return None.
pub async fn process_message(raw: &str, client: Arc<Mutex<OrbitClient>>) -> Option<Value> {
    let msg: Value = match serde_json::from_str(raw) {
        Ok(v)  => v,
        Err(e) => {
            return Some(json_rpc_error(
                Value::Null,
                -32700,
                &format!("Parse error: {}", e),
            ));
        }
    };

    let id     = msg.get("id").cloned().unwrap_or(Value::Null);
    let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");

    // Notifications have no id — don't reply
    let has_id = msg.get("id").is_some();

    match method {
        "initialize" => {
            if !has_id { return None; }
            Some(json_rpc_result(id, initialize_response()))
        }
        "ping" => {
            if !has_id { return None; }
            Some(json_rpc_result(id, serde_json::json!({})))
        }
        "tools/list" => {
            if !has_id { return None; }
            let tools = crate::tools::tool_definitions();
            Some(json_rpc_result(id, serde_json::json!({ "tools": tools })))
        }
        "tools/call" => {
            if !has_id { return None; }
            let params = msg.get("params").cloned().unwrap_or(serde_json::json!({}));
            let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let tool_args = params.get("arguments").cloned().unwrap_or(serde_json::json!({}));

            match dispatch_tool(tool_name, &tool_args, client).await {
                Ok(result) => Some(json_rpc_result(id, serde_json::json!({
                    "content": [{
                        "type": "text",
                        "text": serde_json::to_string_pretty(&result)
                            .unwrap_or_else(|_| result.to_string())
                    }],
                    "isError": false
                }))),
                // MCP spec §3.3.2: tool errors must NOT be JSON-RPC errors.
                // They must be successful JSON-RPC responses with isError=true.
                Err(e) => Some(json_rpc_result(id, serde_json::json!({
                    "content": [{
                        "type": "text",
                        "text": format!("Error: {}", e)
                    }],
                    "isError": true
                }))),
            }
        }
        "notifications/initialized" => None,
        _ => {
            if !has_id { return None; }
            Some(json_rpc_error(id, -32601, &format!("Method not found: {}", method)))
        }
    }
}

pub fn initialize_response() -> Value {
    serde_json::json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "tools": {}
        },
        "serverInfo": {
            "name": "jotti-mcp",
            "version": env!("CARGO_PKG_VERSION")
        }
    })
}

pub fn json_rpc_result(id: Value, result: Value) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    })
}

pub fn json_rpc_error(id: Value, code: i64, message: &str) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message
        }
    })
}
