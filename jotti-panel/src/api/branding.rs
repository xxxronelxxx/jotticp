use axum::{
    Router,
    routing::{get, post},
    extract::State,
    Json,
    http::StatusCode,
};
use axum_extra::extract::Multipart;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{AppState, ApiError, ApiResult};
use super::auth::Claims;

// ── Types ─────────────────────────────────────────────────────────────────────

const MAX_LOGO_BYTES: usize = 1024 * 1024; // 1 MB
const LOGO_TARGET_W: u32 = 200;
const LOGO_TARGET_H: u32 = 60;

#[derive(Debug, Serialize)]
pub struct BrandingResponse {
    pub id:            Option<Uuid>,
    pub reseller_id:   Uuid,
    pub panel_name:    Option<String>,
    pub logo_url:      Option<String>,
    pub primary_color: Option<String>,
    pub support_url:   Option<String>,
    pub custom_domain: Option<String>,
    pub updated_at:    Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateBrandingRequest {
    pub logo_url:      Option<String>,
    pub primary_color: Option<String>,
    pub panel_name:    Option<String>,
    pub support_url:   Option<String>,
    pub custom_domain: Option<String>,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/branding",        get(get_branding).put(update_branding))
        .route("/api/v1/branding/logo",   post(upload_logo))
}

// ── Access helper ─────────────────────────────────────────────────────────────

fn caller(claims: &Claims) -> ApiResult<(Uuid, bool)> {
    let user_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    let is_admin = claims.role == "admin";
    Ok((user_id, is_admin))
}

fn require_reseller_or_admin(claims: &Claims) -> ApiResult<(Uuid, bool)> {
    let (user_id, is_admin) = caller(claims)?;
    let is_reseller = claims.role == "reseller";
    if !is_admin && !is_reseller {
        return Err(ApiError::Forbidden("Branding requires reseller or admin role".into()));
    }
    Ok((user_id, is_admin))
}

// ── Handlers ──────────────────────────────────────────────────────────────────

async fn get_branding(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<Json<BrandingResponse>> {
    let (reseller_id, is_admin) = require_reseller_or_admin(&claims)?;

    // Admins get system-wide branding; resellers get their own
    let lookup_id = if is_admin { Uuid::nil() } else { reseller_id };

    let row = sqlx::query!(
        "SELECT id, reseller_id, brand_name AS panel_name, logo_path, primary_color,
                support_url, updated_at
         FROM reseller_brands WHERE reseller_id = $1",
        lookup_id
    )
    .fetch_optional(&state.db)
    .await?;

    match row {
        Some(r) => Ok(Json(BrandingResponse {
            id:            Some(r.id),
            reseller_id:   r.reseller_id,
            panel_name:    Some(r.panel_name),
            logo_url:      r.logo_path.map(|p| logo_path_to_url(&p)),
            primary_color: r.primary_color,
            support_url:   r.support_url,
            custom_domain: None,
            updated_at:    Some(r.updated_at),
        })),
        None => Ok(Json(BrandingResponse {
            id:            None,
            reseller_id:   lookup_id,
            panel_name:    None,
            logo_url:      None,
            primary_color: None,
            support_url:   None,
            custom_domain: None,
            updated_at:    None,
        })),
    }
}

async fn update_branding(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<UpdateBrandingRequest>,
) -> ApiResult<Json<BrandingResponse>> {
    let (reseller_id, is_admin) = require_reseller_or_admin(&claims)?;
    let lookup_id = if is_admin { Uuid::nil() } else { reseller_id };

    // Validate color format (#RRGGBB)
    if let Some(ref color) = req.primary_color {
        if !is_valid_hex_color(color) {
            return Err(ApiError::Validation("primary_color must be a valid hex color (e.g. #1a2b3c)".into()));
        }
    }

    // Validate custom domain format
    if let Some(ref domain) = req.custom_domain {
        if !domain.is_empty() && !is_valid_domain(domain) {
            return Err(ApiError::Validation(format!("'{}' is not a valid domain name", domain)));
        }
    }

    // Validate URL format for logo_url and support_url
    for (field, val) in [("logo_url", &req.logo_url), ("support_url", &req.support_url)] {
        if let Some(url) = val {
            if !url.starts_with("https://") && !url.starts_with("http://") {
                return Err(ApiError::Validation(format!("{} must be a valid HTTP(S) URL", field)));
            }
        }
    }

    // brand_name is NOT NULL — use panel_name if provided, else "JottiCP"
    let brand_name = req.panel_name.as_deref().unwrap_or("JottiCP").to_string();

    // Check if a row already exists for this reseller
    let existing_id = sqlx::query_scalar!(
        "SELECT id FROM reseller_brands WHERE reseller_id = $1",
        lookup_id
    )
    .fetch_optional(&state.db)
    .await?;

    if let Some(existing_id) = existing_id {
        sqlx::query!(
            "UPDATE reseller_brands SET
               brand_name    = COALESCE($2, brand_name),
               logo_path     = COALESCE($3, logo_path),
               primary_color = COALESCE($4, primary_color),
               support_url   = COALESCE($5, support_url),
               updated_at    = NOW()
             WHERE id = $1",
            existing_id,
            req.panel_name,
            req.logo_url,
            req.primary_color,
            req.support_url,
        )
        .execute(&state.db)
        .await?;
    } else {
        let branding_id = Uuid::new_v4();
        sqlx::query!(
            "INSERT INTO reseller_brands (id, reseller_id, brand_name, logo_path, primary_color,
                                          support_url, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, NOW())",
            branding_id, lookup_id,
            brand_name,
            req.logo_url,
            req.primary_color,
            req.support_url,
        )
        .execute(&state.db)
        .await?;
    }

    get_branding(State(state), claims).await
}

async fn upload_logo(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    mut multipart: Multipart,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let (reseller_id, is_admin) = require_reseller_or_admin(&claims)?;
    let lookup_id = if is_admin { Uuid::nil() } else { reseller_id };

    let mut logo_data: Option<(Vec<u8>, String)> = None;

    while let Some(field) = multipart.next_field().await
        .map_err(|e| ApiError::Validation(format!("Multipart error: {}", e)))?
    {
        let content_type = field.content_type()
            .map(|ct| ct.to_string())
            .unwrap_or_default();
        let name = field.name().unwrap_or("").to_string();
        let data = field.bytes().await
            .map_err(|e| ApiError::Validation(format!("Field read error: {}", e)))?;

        if data.len() > MAX_LOGO_BYTES {
            return Err(ApiError::Validation("Logo must be 1MB or smaller".into()));
        }

        if name == "logo" || name == "file" {
            // Validate MIME type
            if !content_type.contains("png") && !content_type.contains("webp")
                && !content_type.contains("jpeg") && !content_type.contains("jpg")
            {
                // Try to detect by magic bytes
                if !is_image_bytes(&data) {
                    return Err(ApiError::Validation("Logo must be PNG, WebP, or JPEG".into()));
                }
            }
            logo_data = Some((data.to_vec(), content_type));
        }
    }

    let (data, content_type) = logo_data
        .ok_or_else(|| ApiError::Validation("Missing 'logo' field in multipart".into()))?;

    // Resize to 200x60 using the `image` crate
    let resized = resize_logo(&data, LOGO_TARGET_W, LOGO_TARGET_H)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Image resize failed: {}", e)))?;

    // Determine output format
    let ext = if content_type.contains("webp") { "webp" } else { "png" };
    let logo_dir = format!("/etc/jottiecp/branding/{}", lookup_id);
    let logo_path = format!("{}/logo.{}", logo_dir, ext);
    let logo_url  = format!("/api/v1/static/branding/{}/logo.{}", lookup_id, ext);

    tokio::fs::create_dir_all(&logo_dir).await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Cannot create branding dir: {}", e)))?;
    tokio::fs::write(&logo_path, &resized).await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Cannot write logo: {}", e)))?;

    let branding_id = Uuid::new_v4();

    sqlx::query!(
        "INSERT INTO branding (id, reseller_id, logo_path, updated_at)
         VALUES ($1, $2, $3, NOW())
         ON CONFLICT (reseller_id) DO UPDATE SET logo_path = $3, updated_at = NOW()",
        branding_id, lookup_id, logo_path
    )
    .execute(&state.db)
    .await?;

    Ok((StatusCode::CREATED, Json(serde_json::json!({
        "logo_url": logo_url,
        "width": LOGO_TARGET_W,
        "height": LOGO_TARGET_H,
        "size_bytes": resized.len(),
    }))))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn resize_logo(data: &[u8], width: u32, height: u32) -> anyhow::Result<Vec<u8>> {
    use image::{ImageReader, ImageFormat};
    use std::io::Cursor;

    let reader = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(|e| anyhow::anyhow!("Cannot read image: {}", e))?;

    let img = reader.decode()
        .map_err(|e| anyhow::anyhow!("Cannot decode image: {}", e))?;

    let resized = img.resize(width, height, image::imageops::FilterType::Lanczos3);

    let mut out = Vec::new();
    resized.write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
        .map_err(|e| anyhow::anyhow!("Cannot encode PNG: {}", e))?;

    Ok(out)
}

fn is_image_bytes(data: &[u8]) -> bool {
    // PNG: 89 50 4E 47
    if data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) { return true; }
    // JPEG: FF D8 FF
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) { return true; }
    // WebP: RIFF....WEBP
    if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" { return true; }
    false
}

fn is_valid_hex_color(color: &str) -> bool {
    let s = color.trim();
    if !s.starts_with('#') { return false; }
    let hex = &s[1..];
    (hex.len() == 6 || hex.len() == 3)
        && hex.chars().all(|c| c.is_ascii_hexdigit())
}

fn is_valid_domain(domain: &str) -> bool {
    let re = regex::Regex::new(
        r"^(?:[a-z0-9](?:[a-z0-9\-]{0,61}[a-z0-9])?\.)+[a-z]{2,}$"
    ).unwrap();
    re.is_match(&domain.to_lowercase()) && domain.len() <= 253
}

fn logo_path_to_url(path: &str) -> String {
    // Convert filesystem path to served URL
    // /etc/jottiecp/branding/{id}/logo.png -> /api/v1/static/branding/{id}/logo.png
    path.replace("/etc/jottiecp/branding/", "/api/v1/static/branding/")
}
