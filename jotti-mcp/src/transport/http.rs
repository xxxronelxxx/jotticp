use anyhow::Result;
use axum::{
    Router,
    routing::{get, post},
    extract::State,
    Json,
    response::IntoResponse,
    http::{HeaderMap, StatusCode},
};
use serde_json::Value;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::client::OrbitClient;
use crate::tools::tool_definitions;
use crate::transport::stdio::{
    initialize_response, json_rpc_error, json_rpc_result, process_message,
};

// ── Shared state for HTTP transport ──────────────────────────────────────────

#[derive(Clone)]
struct HttpState {
    client:  Arc<Mutex<OrbitClient>>,
    /// Expected value of the `Authorization: Bearer <token>` header.
    /// Set from `ORBIT_API_KEY` at startup.
    api_key: String,
}

// ── Authentication helper ─────────────────────────────────────────────────────

/// Returns Ok(()) if the request carries a valid Bearer token, or an error
/// response tuple ready to be returned from a handler.
fn require_auth(headers: &HeaderMap, api_key: &str) -> Result<(), (StatusCode, Json<Value>)> {
    let provided = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");

    if provided.is_empty() {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "Missing Authorization header" })),
        ));
    }

    // Constant-time comparison to prevent timing attacks.
    if !constant_time_eq(provided.as_bytes(), api_key.as_bytes()) {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "Invalid API key" })),
        ));
    }

    Ok(())
}

/// Constant-time byte slice comparison (no short-circuit).
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// ── Run HTTP server ───────────────────────────────────────────────────────────

/// Listen on 127.0.0.1:3002 and serve MCP over HTTP.
pub async fn run_http(client: Arc<Mutex<OrbitClient>>) -> Result<()> {
    // The API key is already validated present by OrbitClient::from_env(),
    // but we re-read it here to use as the inbound auth token.
    let api_key = std::env::var("ORBIT_API_KEY")
        .map_err(|_| anyhow::anyhow!("ORBIT_API_KEY must be set for HTTP transport"))?;

    let state = HttpState { client, api_key };

    let app = Router::new()
        .route("/mcp",               post(mcp_endpoint))
        .route("/mcp/tools",         get(tools_endpoint))
        .route("/.well-known/mcp",   get(well_known_endpoint))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3002));
    tracing::info!("jotti-mcp HTTP transport listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// POST /mcp — MCP JSON-RPC 2.0 endpoint (requires Bearer auth).
async fn mcp_endpoint(
    State(state): State<HttpState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    if let Err(auth_err) = require_auth(&headers, &state.api_key) {
        return auth_err.into_response();
    }

    let raw = match std::str::from_utf8(&body) {
        Ok(s)  => s,
        Err(_) => {
            let err = json_rpc_error(Value::Null, -32700, "Invalid UTF-8 in request body");
            return (StatusCode::BAD_REQUEST, Json(err)).into_response();
        }
    };

    match process_message(raw, state.client).await {
        Some(resp) => (StatusCode::OK, Json(resp)).into_response(),
        None => {
            // Notification — 204 No Content
            StatusCode::NO_CONTENT.into_response()
        }
    }
}

/// GET /mcp/tools — human-readable tool listing for documentation/introspection.
async fn tools_endpoint() -> Json<Value> {
    Json(serde_json::json!({
        "server": "jotti-mcp",
        "version": env!("CARGO_PKG_VERSION"),
        "tools": tool_definitions()
    }))
}

/// GET /.well-known/mcp — MCP server discovery document.
async fn well_known_endpoint() -> Json<Value> {
    Json(serde_json::json!({
        "schema_version": "2024-11-05",
        "name": "jotti-mcp",
        "description": "JottiCP MCP server — exposes all admin operations as MCP tools for AI agents",
        "version": env!("CARGO_PKG_VERSION"),
        "endpoint": "http://127.0.0.1:3002/mcp",
        "transport": "http",
        "capabilities": initialize_response()["capabilities"],
        "tools_endpoint": "http://127.0.0.1:3002/mcp/tools"
    }))
}
