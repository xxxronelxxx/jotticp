//! DNS management via PowerDNS REST API: zones (Create/Delete) + records (Upsert/Delete).

use crate::journal::Journal;
use crate::proto::{
    CreateDnsZoneRequest, DeleteDnsRecordRequest, DeleteDnsZoneRequest, DnsRecordRequest,
    StatusResponse,
};
use crate::validation::validate_dns_zone;
use std::process::Command;
use tonic::Status;
use tracing::{info, warn};

// ── CreateDnsZone ─────────────────────────────────────────────────────────────

pub async fn create_dns_zone(
    req: CreateDnsZoneRequest,
    journal: &Journal,
) -> Result<tonic::Response<StatusResponse>, Status> {
    let zone = validate_dns_zone(&req.zone)?;

    info!(zone = %zone, ns1 = %req.ns1, ns2 = %req.ns2, "create_dns_zone");

    let op_id = journal
        .begin_op(
            "create_dns_zone",
            None,
            &serde_json::json!({ "zone": zone }),
        )
        .await
        .map_err(|e| Status::internal(format!("journal error: {}", e)))?;

    let result = do_create_dns_zone(&zone, &req.ns1, &req.ns2, &req.admin).await;

    match &result {
        Ok(_)  => { let _ = journal.finish_op(&op_id).await; }
        Err(e) => { let _ = journal.fail_op(&op_id, &e.message()).await; }
    }

    result
}

async fn do_create_dns_zone(
    zone: &str,
    ns1: &str,
    ns2: &str,
    admin: &str,
) -> Result<tonic::Response<StatusResponse>, Status> {
    // Ensure zone ends with trailing dot (RFC 1035)
    let zone_fqdn = if zone.ends_with('.') {
        zone.to_string()
    } else {
        format!("{}.", zone)
    };

    let admin_email = if admin.is_empty() {
        format!("hostmaster.{}.", zone)
    } else {
        admin.replace('@', ".") + "."
    };

    // Default nameservers if not provided
    let ns1 = if ns1.is_empty() { "ns1.jotticp.local." } else { ns1 };
    let ns2 = if ns2.is_empty() { "ns2.jotticp.local." } else { ns2 };

    // PowerDNS API endpoint (local socket or localhost:8081)
    let pdns_api_url = std::env::var("PDNS_API_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8081".to_string());
    let pdns_api_key = std::env::var("PDNS_API_KEY")
        .unwrap_or_else(|_| "jotticp-pdns-key".to_string());

    // Build the zone creation payload
    let payload = serde_json::json!({
        "name": zone_fqdn,
        "kind": "Native",
        // PowerDNS auto-creates NS records from `nameservers`; supplying BOTH this and an
        // explicit NS rrset is rejected ("Nameservers list MUST NOT be mixed with zone-level
        // NS in rrsets"), so we only send the SOA rrset here.
        "nameservers": [ns1, ns2],
        "rrsets": [
            {
                "name": zone_fqdn,
                "type": "SOA",
                "ttl": 3600,
                "records": [{
                    "content": format!("{} {} 1 10800 3600 604800 300", ns1, admin_email),
                    "disabled": false
                }]
            }
        ]
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/v1/servers/localhost/zones", pdns_api_url))
        .header("X-API-Key", &pdns_api_key)
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| Status::internal(format!("PowerDNS API request failed: {}", e)))?;

    if !resp.status().is_success() {
        let status_code = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(Status::internal(format!(
            "PowerDNS API returned {}: {}",
            status_code, body
        )));
    }

    info!(zone = %zone_fqdn, "DNS zone created");

    Ok(tonic::Response::new(StatusResponse {
        success: true,
        message: format!("DNS zone {} created", zone_fqdn),
    }))
}

// ── DeleteDnsZone ─────────────────────────────────────────────────────────────

pub async fn delete_dns_zone(
    req: DeleteDnsZoneRequest,
    journal: &Journal,
) -> Result<tonic::Response<StatusResponse>, Status> {
    let zone = validate_dns_zone(&req.zone)?;

    info!(zone = %zone, "delete_dns_zone");

    let op_id = journal
        .begin_op("delete_dns_zone", None, &serde_json::json!({ "zone": zone }))
        .await
        .map_err(|e| Status::internal(format!("journal error: {}", e)))?;

    let result = do_delete_dns_zone(&zone).await;

    match &result {
        Ok(_)  => { let _ = journal.finish_op(&op_id).await; }
        Err(e) => { let _ = journal.fail_op(&op_id, &e.message()).await; }
    }

    result
}

async fn do_delete_dns_zone(zone: &str) -> Result<tonic::Response<StatusResponse>, Status> {
    let zone_fqdn = if zone.ends_with('.') {
        zone.to_string()
    } else {
        format!("{}.", zone)
    };

    let pdns_api_url = std::env::var("PDNS_API_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8081".to_string());
    let pdns_api_key = std::env::var("PDNS_API_KEY")
        .unwrap_or_else(|_| "jotticp-pdns-key".to_string());

    let client = reqwest::Client::new();
    let resp = client
        .delete(format!(
            "{}/api/v1/servers/localhost/zones/{}",
            pdns_api_url, zone_fqdn
        ))
        .header("X-API-Key", &pdns_api_key)
        .send()
        .await
        .map_err(|e| Status::internal(format!("PowerDNS API request failed: {}", e)))?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        warn!(zone = %zone_fqdn, "DNS zone not found in PowerDNS — nothing to delete");
    } else if !resp.status().is_success() {
        let status_code = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(Status::internal(format!(
            "PowerDNS API returned {}: {}",
            status_code, body
        )));
    }

    info!(zone = %zone_fqdn, "DNS zone deleted");

    Ok(tonic::Response::new(StatusResponse {
        success: true,
        message: format!("DNS zone {} deleted", zone_fqdn),
    }))
}

// ── Records (Upsert / Delete) ─────────────────────────────────────────────────

fn pdns_api() -> (String, String) {
    let url = std::env::var("PDNS_API_URL").unwrap_or_else(|_| "http://127.0.0.1:8081".to_string());
    let key = std::env::var("PDNS_API_KEY").unwrap_or_else(|_| "jotticp-pdns-key".to_string());
    (url, key)
}

fn ensure_dot(s: &str) -> String {
    if s.ends_with('.') { s.to_string() } else { format!("{}.", s) }
}

/// Build the absolute record name PowerDNS expects (FQDN with trailing dot).
fn record_fqdn(name: &str, zone_fqdn: &str) -> String {
    let n = name.trim();
    if n.is_empty() || n == "@" {
        return zone_fqdn.to_string();
    }
    let z_nodot = zone_fqdn.trim_end_matches('.');
    let n_nodot = n.trim_end_matches('.');
    if n_nodot == z_nodot || n_nodot.ends_with(&format!(".{}", z_nodot)) {
        ensure_dot(n_nodot)
    } else {
        format!("{}.{}", n_nodot, zone_fqdn)
    }
}

/// MX/SRV carry priority as a leading integer in the PowerDNS record content.
fn record_content(rtype: &str, content: &str, priority: i32) -> String {
    let first_is_num = content
        .split_whitespace()
        .next()
        .map(|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_digit()))
        .unwrap_or(false);
    if (rtype == "MX" || rtype == "SRV") && priority > 0 && !first_is_num {
        format!("{} {}", priority, content)
    } else {
        content.to_string()
    }
}

async fn patch_rrset(zone_fqdn: &str, rrset: serde_json::Value) -> Result<(), Status> {
    let (url, key) = pdns_api();
    let resp = reqwest::Client::new()
        .patch(format!("{}/api/v1/servers/localhost/zones/{}", url, zone_fqdn))
        .header("X-API-Key", &key)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({ "rrsets": [rrset] }))
        .send()
        .await
        .map_err(|e| Status::internal(format!("PowerDNS API request failed: {}", e)))?;
    if !resp.status().is_success() {
        let sc = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(Status::internal(format!("PowerDNS API returned {}: {}", sc, body)));
    }
    // Best-effort rectify so ordering/DNSSEC stays consistent.
    let _ = reqwest::Client::new()
        .put(format!("{}/api/v1/servers/localhost/zones/{}/rectify", url, zone_fqdn))
        .header("X-API-Key", &key)
        .send()
        .await;
    Ok(())
}

pub async fn upsert_dns_record(
    req: DnsRecordRequest,
    journal: &Journal,
) -> Result<tonic::Response<StatusResponse>, Status> {
    let zone = validate_dns_zone(&req.zone)?;
    let zone_fqdn = ensure_dot(&zone);
    let rtype = req.rtype.trim().to_uppercase();
    if rtype.is_empty() {
        return Err(Status::invalid_argument("rtype required"));
    }
    let rname = record_fqdn(&req.name, &zone_fqdn);
    let ttl = if req.ttl > 0 { req.ttl } else { 3600 };
    let content = record_content(&rtype, req.content.trim(), req.priority);
    info!(zone = %zone_fqdn, name = %rname, rtype = %rtype, "upsert_dns_record");

    let op_id = journal
        .begin_op(
            "upsert_dns_record",
            None,
            &serde_json::json!({ "zone": zone_fqdn, "name": rname, "type": rtype }),
        )
        .await
        .map_err(|e| Status::internal(format!("journal error: {}", e)))?;

    let rrset = serde_json::json!({
        "name": rname, "type": rtype, "ttl": ttl, "changetype": "REPLACE",
        "records": [{ "content": content, "disabled": false }]
    });
    let result = patch_rrset(&zone_fqdn, rrset).await;
    match &result {
        Ok(_) => { let _ = journal.finish_op(&op_id).await; }
        Err(e) => { let _ = journal.fail_op(&op_id, &e.message()).await; }
    }
    result?;
    Ok(tonic::Response::new(StatusResponse {
        success: true,
        message: format!("record {} {} upserted", rname, rtype),
    }))
}

pub async fn delete_dns_record(
    req: DeleteDnsRecordRequest,
    journal: &Journal,
) -> Result<tonic::Response<StatusResponse>, Status> {
    let zone = validate_dns_zone(&req.zone)?;
    let zone_fqdn = ensure_dot(&zone);
    let rtype = req.rtype.trim().to_uppercase();
    if rtype.is_empty() {
        return Err(Status::invalid_argument("rtype required"));
    }
    let rname = record_fqdn(&req.name, &zone_fqdn);
    info!(zone = %zone_fqdn, name = %rname, rtype = %rtype, "delete_dns_record");

    let op_id = journal
        .begin_op(
            "delete_dns_record",
            None,
            &serde_json::json!({ "zone": zone_fqdn, "name": rname, "type": rtype }),
        )
        .await
        .map_err(|e| Status::internal(format!("journal error: {}", e)))?;

    let rrset = serde_json::json!({ "name": rname, "type": rtype, "changetype": "DELETE" });
    let result = patch_rrset(&zone_fqdn, rrset).await;
    match &result {
        Ok(_) => { let _ = journal.finish_op(&op_id).await; }
        Err(e) => { let _ = journal.fail_op(&op_id, &e.message()).await; }
    }
    result?;
    Ok(tonic::Response::new(StatusResponse {
        success: true,
        message: format!("record {} {} deleted", rname, rtype),
    }))
}
