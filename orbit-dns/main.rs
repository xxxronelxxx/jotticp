/// orbit-dns — PowerDNS 4.9 REST API proxy daemon
///
/// Listens on 127.0.0.1:5353 (internal only).
/// All write operations invalidate the in-memory zone list cache.
/// DNS propagation checks send raw DNS queries via tokio::net::UdpSocket.

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tracing::{error, info, instrument, warn};

// ─────────────────────────────────────────────────────────────────────────────
// Configuration
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Config {
    pdns_api_url: String,
    pdns_api_key: String,
    listen_addr: SocketAddr,
}

impl Config {
    fn from_env() -> Result<Self> {
        let pdns_api_url = std::env::var("PDNS_API_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8081".to_string());
        let pdns_api_key =
            std::env::var("PDNS_API_KEY").context("PDNS_API_KEY env var is required")?;
        let listen_addr = std::env::var("ORBIT_DNS_LISTEN")
            .unwrap_or_else(|_| "127.0.0.1:5353".to_string())
            .parse()
            .context("invalid ORBIT_DNS_LISTEN address")?;
        Ok(Self {
            pdns_api_url,
            pdns_api_key,
            listen_addr,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// In-memory zone list cache
// ─────────────────────────────────────────────────────────────────────────────

const ZONE_CACHE_TTL: Duration = Duration::from_secs(60);

#[derive(Debug)]
struct ZoneCache {
    zones: Vec<Value>,
    fetched_at: Instant,
}

impl ZoneCache {
    fn is_fresh(&self) -> bool {
        self.fetched_at.elapsed() < ZONE_CACHE_TTL
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared application state
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct AppState {
    config: Arc<Config>,
    client: reqwest::Client,
    zone_cache: Arc<RwLock<Option<ZoneCache>>>,
}

impl AppState {
    fn new(config: Config) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .expect("failed to build reqwest client");
        Self {
            config: Arc::new(config),
            client,
            zone_cache: Arc::new(RwLock::new(None)),
        }
    }

    fn pdns_url(&self, path: &str) -> String {
        format!(
            "{}/api/v1/servers/localhost{}",
            self.config.pdns_api_url, path
        )
    }

    fn pdns_headers(&self) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "X-API-Key",
            self.config
                .pdns_api_key
                .parse()
                .expect("invalid PDNS_API_KEY"),
        );
        headers
    }

    async fn invalidate_zone_cache(&self) {
        let mut cache = self.zone_cache.write().await;
        *cache = None;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Error type
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn internal(msg: impl Into<String>) -> Self {
        let m = msg.into();
        error!(error = %m, "internal error");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, m)
    }

    fn not_found(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, msg)
    }

    fn bad_request(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, msg)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = json!({ "error": self.message });
        (self.status, Json(body)).into_response()
    }
}

impl<E: std::fmt::Display> From<E> for ApiError {
    fn from(e: E) -> Self {
        ApiError::internal(e.to_string())
    }
}

type ApiResult<T> = std::result::Result<T, ApiError>;

// ─────────────────────────────────────────────────────────────────────────────
// Request / response DTOs
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CreateZoneRequest {
    name: String,
    kind: Option<String>,
    nameservers: Option<Vec<String>>,
    masters: Option<Vec<String>>,
    account: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateRecordRequest {
    name: String,
    #[serde(rename = "type")]
    record_type: String,
    ttl: Option<u32>,
    records: Vec<RecordContent>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct RecordContent {
    content: String,
    disabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct PropagationRequest {
    name: String,
    #[serde(rename = "type")]
    record_type: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers — ensure zone name ends with dot
// ─────────────────────────────────────────────────────────────────────────────

fn canonicalize_zone(zone: &str) -> String {
    if zone.ends_with('.') {
        zone.to_string()
    } else {
        format!("{}.", zone)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Handlers — Zones
// ─────────────────────────────────────────────────────────────────────────────

#[instrument(skip(state))]
async fn list_zones(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    // Try cache
    {
        let cache = state.zone_cache.read().await;
        if let Some(ref c) = *cache {
            if c.is_fresh() {
                info!(count = c.zones.len(), "zone list served from cache");
                return Ok(Json(json!({ "zones": c.zones })));
            }
        }
    }

    let resp = state
        .client
        .get(state.pdns_url("/zones"))
        .headers(state.pdns_headers())
        .send()
        .await
        .map_err(|e| ApiError::internal(format!("PDNS request failed: {e}")))?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(ApiError::internal(format!("PDNS error: {text}")));
    }

    let zones: Vec<Value> = resp.json().await.map_err(|e| {
        ApiError::internal(format!("failed to deserialize PDNS response: {e}"))
    })?;

    // Update cache
    {
        let mut cache = state.zone_cache.write().await;
        *cache = Some(ZoneCache {
            zones: zones.clone(),
            fetched_at: Instant::now(),
        });
    }

    info!(count = zones.len(), "zone list fetched from PowerDNS");
    Ok(Json(json!({ "zones": zones })))
}

#[instrument(skip(state, body))]
async fn create_zone(
    State(state): State<AppState>,
    Json(body): Json<CreateZoneRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let canonical_name = canonicalize_zone(&body.name);
    let nameservers = body
        .nameservers
        .unwrap_or_else(|| vec!["ns1.example.com.".into(), "ns2.example.com.".into()]);

    let pdns_body = json!({
        "name": canonical_name,
        "kind": body.kind.unwrap_or_else(|| "Native".into()),
        "nameservers": nameservers,
        "masters": body.masters.unwrap_or_default(),
        "account": body.account.unwrap_or_default(),
    });

    let resp = state
        .client
        .post(state.pdns_url("/zones"))
        .headers(state.pdns_headers())
        .json(&pdns_body)
        .send()
        .await
        .map_err(|e| ApiError::internal(format!("PDNS request failed: {e}")))?;

    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));

    if !status.is_success() {
        let msg = body["error"].as_str().unwrap_or("PDNS error").to_string();
        return Err(ApiError::new(StatusCode::BAD_GATEWAY, msg));
    }

    state.invalidate_zone_cache().await;
    info!(zone = %canonical_name, "zone created");
    Ok((StatusCode::CREATED, Json(body)))
}

#[instrument(skip(state))]
async fn delete_zone(
    State(state): State<AppState>,
    Path(zone): Path<String>,
) -> ApiResult<StatusCode> {
    let canonical = canonicalize_zone(&zone);

    let resp = state
        .client
        .delete(state.pdns_url(&format!("/zones/{}", canonical)))
        .headers(state.pdns_headers())
        .send()
        .await
        .map_err(|e| ApiError::internal(format!("PDNS request failed: {e}")))?;

    if resp.status() == StatusCode::NOT_FOUND {
        return Err(ApiError::not_found(format!("zone '{canonical}' not found")));
    }

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(ApiError::internal(format!("PDNS error: {text}")));
    }

    state.invalidate_zone_cache().await;
    info!(zone = %canonical, "zone deleted");
    Ok(StatusCode::NO_CONTENT)
}

// ─────────────────────────────────────────────────────────────────────────────
// Handlers — Records
// ─────────────────────────────────────────────────────────────────────────────

#[instrument(skip(state))]
async fn list_records(
    State(state): State<AppState>,
    Path(zone): Path<String>,
) -> ApiResult<Json<Value>> {
    let canonical = canonicalize_zone(&zone);

    let resp = state
        .client
        .get(state.pdns_url(&format!("/zones/{}", canonical)))
        .headers(state.pdns_headers())
        .send()
        .await
        .map_err(|e| ApiError::internal(format!("PDNS request failed: {e}")))?;

    if resp.status() == StatusCode::NOT_FOUND {
        return Err(ApiError::not_found(format!("zone '{canonical}' not found")));
    }

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(ApiError::internal(format!("PDNS error: {text}")));
    }

    let zone_data: Value = resp.json().await.map_err(|e| {
        ApiError::internal(format!("failed to deserialize zone: {e}"))
    })?;

    let rrsets = zone_data["rrsets"].clone();
    info!(zone = %canonical, "records listed");
    Ok(Json(json!({ "rrsets": rrsets })))
}

#[instrument(skip(state, body))]
async fn create_record(
    State(state): State<AppState>,
    Path(zone): Path<String>,
    Json(body): Json<CreateRecordRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let canonical_zone = canonicalize_zone(&zone);
    let canonical_name = canonicalize_zone(&body.name);

    if body.record_type.is_empty() {
        return Err(ApiError::bad_request("record type is required"));
    }
    if body.records.is_empty() {
        return Err(ApiError::bad_request("at least one record content is required"));
    }

    let ttl = body.ttl.unwrap_or(300);

    let rrset = json!({
        "name": canonical_name,
        "type": body.record_type.to_uppercase(),
        "ttl": ttl,
        "changetype": "REPLACE",
        "records": body.records.iter().map(|r| json!({
            "content": r.content,
            "disabled": r.disabled.unwrap_or(false),
        })).collect::<Vec<_>>(),
    });

    let patch_body = json!({ "rrsets": [rrset] });

    let resp = state
        .client
        .patch(state.pdns_url(&format!("/zones/{}", canonical_zone)))
        .headers(state.pdns_headers())
        .json(&patch_body)
        .send()
        .await
        .map_err(|e| ApiError::internal(format!("PDNS request failed: {e}")))?;

    if !resp.status().is_success() {
        let body: Value = resp.json().await.unwrap_or(json!({}));
        let msg = body["error"].as_str().unwrap_or("PDNS error").to_string();
        return Err(ApiError::new(StatusCode::BAD_GATEWAY, msg));
    }

    info!(
        zone = %canonical_zone,
        name = %canonical_name,
        record_type = %body.record_type,
        "record created/updated"
    );
    Ok((StatusCode::OK, Json(json!({ "status": "ok" }))))
}

#[instrument(skip(state))]
async fn delete_record(
    State(state): State<AppState>,
    Path((zone, name, record_type)): Path<(String, String, String)>,
) -> ApiResult<StatusCode> {
    let canonical_zone = canonicalize_zone(&zone);
    let canonical_name = canonicalize_zone(&name);

    let rrset = json!({
        "name": canonical_name,
        "type": record_type.to_uppercase(),
        "changetype": "DELETE",
    });

    let patch_body = json!({ "rrsets": [rrset] });

    let resp = state
        .client
        .patch(state.pdns_url(&format!("/zones/{}", canonical_zone)))
        .headers(state.pdns_headers())
        .json(&patch_body)
        .send()
        .await
        .map_err(|e| ApiError::internal(format!("PDNS request failed: {e}")))?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(ApiError::internal(format!("PDNS error: {text}")));
    }

    info!(
        zone = %canonical_zone,
        name = %canonical_name,
        record_type = %record_type,
        "record deleted"
    );
    Ok(StatusCode::NO_CONTENT)
}

// ─────────────────────────────────────────────────────────────────────────────
// Handlers — DNS propagation check
// ─────────────────────────────────────────────────────────────────────────────

/// Builds a minimal DNS query wire format for a given name + qtype.
/// Returns the query buffer and the random txid used, so callers can validate
/// that the response txid matches.
fn build_dns_query(name: &str, qtype: u16) -> (Vec<u8>, u16) {
    let txid: u16 = rand::random::<u16>();
    let mut buf: Vec<u8> = Vec::new();

    // Transaction ID — random per query to avoid cross-request response acceptance
    buf.extend_from_slice(&txid.to_be_bytes());
    // Flags: standard query, RD=1
    buf.extend_from_slice(&[0x01, 0x00]);
    // QDCOUNT=1, ANCOUNT=0, NSCOUNT=0, ARCOUNT=0
    buf.extend_from_slice(&[0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);

    // QNAME
    for label in name.trim_end_matches('.').split('.') {
        let bytes = label.as_bytes();
        buf.push(bytes.len() as u8);
        buf.extend_from_slice(bytes);
    }
    buf.push(0x00); // root label

    // QTYPE
    buf.extend_from_slice(&qtype.to_be_bytes());
    // QCLASS IN
    buf.extend_from_slice(&[0x00, 0x01]);

    (buf, txid)
}

/// Parses ANCOUNT from a raw DNS response (bytes 6-7).
fn parse_answer_count(resp: &[u8]) -> u16 {
    if resp.len() < 8 {
        return 0;
    }
    u16::from_be_bytes([resp[6], resp[7]])
}

/// Check whether a DNS name resolves at a given resolver (ip:port).
async fn check_resolver(resolver: &str, qname: &str, qtype: u16) -> Result<bool> {
    use tokio::net::UdpSocket;

    let sock = UdpSocket::bind("0.0.0.0:0")
        .await
        .context("bind UDP socket")?;

    sock.connect(resolver).await.context("connect to resolver")?;

    let (query, txid) = build_dns_query(qname, qtype);
    sock.send(&query).await.context("send DNS query")?;

    let mut buf = [0u8; 512];
    let n = tokio::time::timeout(Duration::from_secs(5), sock.recv(&mut buf))
        .await
        .context("DNS query timed out")?
        .context("recv DNS response")?;

    let resp = &buf[..n];

    // Validate that the response txid matches the query txid
    if resp.len() < 2 || resp[0..2] != query[0..2] {
        anyhow::bail!("DNS response txid mismatch (expected 0x{:04X})", txid);
    }

    let answer_count = parse_answer_count(resp);
    Ok(answer_count > 0)
}

#[instrument(skip(state, body))]
async fn check_propagation(
    State(state): State<AppState>,
    Path(zone): Path<String>,
    Json(body): Json<PropagationRequest>,
) -> ApiResult<Json<Value>> {
    let qname = canonicalize_zone(&body.name);
    // Map common record types to qtype numbers
    let qtype: u16 = match body.record_type.to_uppercase().as_str() {
        "A" => 1,
        "NS" => 2,
        "CNAME" => 5,
        "SOA" => 6,
        "MX" => 15,
        "TXT" => 16,
        "AAAA" => 28,
        "SRV" => 33,
        "CAA" => 257,
        other => {
            warn!(record_type = other, "unknown record type for propagation check, using A");
            1
        }
    };

    let resolvers = [("8.8.8.8:53", "google"), ("1.1.1.1:53", "cloudflare")];
    let mut results: HashMap<&str, bool> = HashMap::new();

    for (resolver_addr, resolver_name) in &resolvers {
        match check_resolver(resolver_addr, &qname, qtype).await {
            Ok(found) => {
                results.insert(resolver_name, found);
                info!(
                    resolver = resolver_name,
                    qname = %qname,
                    found,
                    "propagation check complete"
                );
            }
            Err(e) => {
                warn!(resolver = resolver_name, error = %e, "propagation check failed");
                results.insert(resolver_name, false);
            }
        }
    }

    let propagated = results.values().all(|&v| v);

    Ok(Json(json!({
        "zone": zone,
        "name": body.name,
        "type": body.record_type,
        "propagated": propagated,
        "resolvers": {
            "google":     results.get("google").copied().unwrap_or(false),
            "cloudflare": results.get("cloudflare").copied().unwrap_or(false),
        }
    })))
}

// ─────────────────────────────────────────────────────────────────────────────
// Router
// ─────────────────────────────────────────────────────────────────────────────

fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/zones", get(list_zones).post(create_zone))
        .route("/zones/{zone}", delete(delete_zone))
        .route("/zones/{zone}/records", get(list_records).post(create_record))
        .route("/zones/{zone}/records/{name}/{type}", delete(delete_record))
        .route("/zones/{zone}/check-propagation", post(check_propagation))
        .with_state(state)
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "orbit-dns" }))
}

// ─────────────────────────────────────────────────────────────────────────────
// Main
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("orbit_dns=info".parse().unwrap()),
        )
        .init();

    let config = Config::from_env().context("failed to load configuration")?;
    let listen_addr = config.listen_addr;

    info!(
        listen = %listen_addr,
        pdns_url = %config.pdns_api_url,
        "orbit-dns starting"
    );

    let state = AppState::new(config);
    let router = build_router(state);

    let listener = tokio::net::TcpListener::bind(listen_addr)
        .await
        .context("failed to bind listener")?;

    info!(addr = %listen_addr, "orbit-dns listening");

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")?;

    info!("orbit-dns stopped");
    Ok(())
}

async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut sigterm = signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
    let mut sigint = signal(SignalKind::interrupt()).expect("failed to install SIGINT handler");
    tokio::select! {
        _ = sigterm.recv() => { info!("received SIGTERM, shutting down"); }
        _ = sigint.recv()  => { info!("received SIGINT, shutting down"); }
    }
}
