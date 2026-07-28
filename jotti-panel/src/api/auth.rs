use axum::{
    Router,
    routing::post,
    extract::State,
    Json,
    http::StatusCode,
};
use chrono::{Duration, Utc};
use jsonwebtoken::{encode, decode, Algorithm, Header, EncodingKey, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{AppState, ApiError, ApiResult};

// ── JWT Claims ────────────────────────────────────────────────────────────────

/// Claims embedded in the JWT access token.
///
/// Algorithm: ES256 (ECDSA P-256). Private key signs; public key verifies.
/// The public key is served at /api/v1/.well-known/jwks.json.
///
/// `sub` is a String (JWT standard — contains the user UUID as text).
/// `jti` is a per-token unique ID. Blocklist key is `jottiecp:blocklist:{jti}` —
/// logging out one session does NOT invalidate other sessions for the same user.
///
/// AppState MUST hold:
///   `jwt_encoding_key: EncodingKey`  — from `EncodingKey::from_ec_pem(private_pem)`
///   `jwt_decoding_key: DecodingKey`  — from `DecodingKey::from_ec_pem(public_pem)`
/// Generated at first run, stored in /etc/jottiecp/jwt_ec_key.pem (private, 0600).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub:              String,           // user UUID as string (JWT standard)
    pub jti:              Uuid,             // unique token ID — per-token blocklist key
    pub email:            String,
    pub role:             String,           // "admin" | "reseller" | "user"
    pub exp:              i64,              // expiry (unix timestamp)
    pub iat:              i64,              // issued-at (unix timestamp)
    pub wizard_complete:  bool,             // false → redirect to onboarding wizard
    pub totp_verified:    bool,             // false → JWT is a pre-TOTP interim token
    pub impersonated_by:  Option<String>,   // Some(admin_uuid) when this is an impersonation token
}

// ── Request / Response types ─────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct LoginRequest {
    pub email:    String,
    pub password: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    /// Interim token — valid for TOTP step only (2 minutes).
    pub interim_token: String,
    pub totp_required: bool,
    /// Full session token — set only when totp_required = false (no TOTP configured).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<i64>,
    /// User identity fields — set only when totp_required = false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wizard_complete: Option<bool>,
}

#[derive(Deserialize)]
pub struct TotpRequest {
    pub token: String,
    pub code:  String,
}

#[derive(Deserialize)]
pub struct BackupCodeRequest {
    pub token: String,
    pub code:  String,  // 12-char alphanumeric backup code
}

#[derive(Serialize)]
pub struct AuthResponse {
    pub access_token:   String,
    pub refresh_token:  String,
    pub expires_in:     i64,
    pub user_id:        Uuid,
    pub email:          String,
    pub role:           String,
    pub wizard_complete: bool,
}

#[derive(Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(Deserialize)]
pub struct SetupTotpRequest {
    // Phase 1: no fields needed — just initiate TOTP setup to get the secret/QR
}

#[derive(Deserialize)]
pub struct ConfirmTotpRequest {
    pub totp_code: String,
}

#[derive(Serialize)]
pub struct SetupTotpResponse {
    pub secret:           String,
    pub provisioning_uri: String,
    pub backup_codes:     Vec<String>,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/auth/login",             post(login))
        .route("/api/v1/auth/totp",              post(verify_totp))
        .route("/api/v1/auth/totp/backup",       post(verify_backup_code_handler))
        .route("/api/v1/auth/refresh",           post(refresh_token))
        .route("/api/v1/auth/logout",            post(logout))
        .route("/api/v1/auth/totp/setup",          post(setup_totp))
        .route("/api/v1/auth/totp/setup/confirm", post(confirm_totp))
        .route("/api/v1/auth/totp/disable",       post(disable_totp))
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// Step 1: verify password, issue an interim token (totp_verified = false, 2-min TTL).
async fn login(
    State(state): State<Arc<AppState>>,
    axum::extract::ConnectInfo(addr): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Json(req): Json<LoginRequest>,
) -> ApiResult<Json<LoginResponse>> {
    let ip = addr.ip().to_string();

    let user = sqlx::query!(
        "SELECT id, email, password_hash, totp_secret, role::text AS role, failed_login_count,
                locked_until, wizard_complete
         FROM users WHERE email = $1 AND deleted_at IS NULL",
        req.email.to_lowercase()
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::InvalidCredentials)?;

    if let Some(locked_until) = user.locked_until {
        if locked_until > Utc::now() {
            let retry_after = (locked_until - Utc::now()).num_seconds() as u64;
            log_auth_failure(&ip, "account_locked", &user.email);
            return Err(ApiError::AccountLocked { retry_after });
        }
    }

    let password_ok = verify_password(&req.password, &user.password_hash)?;
    if !password_ok {
        let new_count = user.failed_login_count + 1;
        let lock_until = if new_count >= 10 {
            Some(Utc::now() + Duration::minutes(15))
        } else {
            None
        };
        sqlx::query!(
            "UPDATE users SET failed_login_count = $1, locked_until = $2 WHERE id = $3",
            new_count, lock_until, user.id
        )
        .execute(&state.db)
        .await?;

        log_auth_failure(&ip, "bad_password", &user.email);
        return Err(ApiError::InvalidCredentials);
    }

    sqlx::query!(
        "UPDATE users SET failed_login_count = 0, locked_until = NULL WHERE id = $1",
        user.id
    )
    .execute(&state.db)
    .await?;

    let totp_required = user.totp_secret.is_some();
    let user_id = user.id;
    let user_email = user.email.clone();
    let user_role = user.role.unwrap_or_default();
    let wizard_complete = user.wizard_complete;

    let interim_claims = Claims {
        sub:             user_id.to_string(),
        jti:             Uuid::new_v4(),
        email:           user_email.clone(),
        role:            user_role.clone(),
        exp:             (Utc::now() + Duration::minutes(2)).timestamp(),
        iat:             Utc::now().timestamp(),
        wizard_complete,
        totp_verified:   false,
        impersonated_by: None,
    };
    let interim_token = encode_jwt(&interim_claims, &state.jwt_encoding_key)?;

    // If no TOTP is configured, also issue a full session JWT so clients can
    // authenticate immediately without a second round-trip.
    let (access_token, expires_in) = if !totp_required {
        let session_claims = Claims {
            sub:             user_id.to_string(),
            jti:             Uuid::new_v4(),
            email:           user_email.clone(),
            role:            user_role.clone(),
            exp:             (Utc::now() + Duration::hours(8)).timestamp(),
            iat:             Utc::now().timestamp(),
            wizard_complete,
            totp_verified:   true,
            impersonated_by: None,
        };
        let tok = encode_jwt(&session_claims, &state.jwt_encoding_key)?;
        sqlx::query!("UPDATE users SET last_login_at = NOW() WHERE id = $1", user_id)
            .execute(&state.db)
            .await?;
        (Some(tok), Some(8_i64 * 3600))
    } else {
        (None, None)
    };

    tracing::info!(user_id = %user_id, totp_required, "Login step 1 complete");
    let (resp_user_id, resp_email, resp_role, resp_wizard) = if !totp_required {
        (Some(user_id), Some(user_email), Some(user_role), Some(wizard_complete))
    } else {
        (None, None, None, None)
    };
    Ok(Json(LoginResponse {
        interim_token,
        totp_required,
        access_token,
        expires_in,
        user_id:         resp_user_id,
        email:           resp_email,
        role:            resp_role,
        wizard_complete: resp_wizard,
    }))
}

/// Step 2a: verify TOTP code, issue full session JWT.
async fn verify_totp(
    State(state): State<Arc<AppState>>,
    axum::extract::ConnectInfo(addr): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Json(req): Json<TotpRequest>,
) -> ApiResult<Json<AuthResponse>> {
    let ip = addr.ip().to_string();

    let interim_claims = decode_jwt(&req.token, &state.jwt_decoding_key)
        .map_err(|_| ApiError::InvalidCredentials)?;

    if interim_claims.totp_verified {
        return Err(ApiError::Unauthorized);
    }

    let user_id: Uuid = interim_claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;

    let user = sqlx::query!(
        "SELECT id, email, role::text AS role, totp_secret, wizard_complete FROM users WHERE id = $1",
        user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::Unauthorized)?;

    let secret = user.totp_secret.ok_or(ApiError::Unauthorized)?;

    let totp = totp_rs::TOTP::new(
        totp_rs::Algorithm::SHA1,
        6,
        1,
        30,
        base32::decode(base32::Alphabet::RFC4648 { padding: false }, &secret)
            .ok_or(ApiError::Internal(anyhow::anyhow!("Invalid TOTP secret in DB")))?,
    )
    .map_err(|e| ApiError::Internal(anyhow::anyhow!("TOTP error: {}", e)))?;

    if !totp.check_current(&req.code).map_err(|e| ApiError::Internal(anyhow::anyhow!("{}", e)))? {
        log_auth_failure(&ip, "bad_totp", &user.email);
        return Err(ApiError::InvalidTotp);
    }

    issue_session_jwt(state, user.id, user.email, user.role.unwrap_or_default(), user.wizard_complete).await
}

/// Step 2b: verify a TOTP backup code (consumes and removes the used code to prevent replay).
async fn verify_backup_code_handler(
    State(state): State<Arc<AppState>>,
    axum::extract::ConnectInfo(addr): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Json(req): Json<BackupCodeRequest>,
) -> ApiResult<Json<AuthResponse>> {
    let ip = addr.ip().to_string();

    let interim_claims = decode_jwt(&req.token, &state.jwt_decoding_key)
        .map_err(|_| ApiError::InvalidCredentials)?;

    if interim_claims.totp_verified {
        return Err(ApiError::Unauthorized);
    }

    let user_id: Uuid = interim_claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;

    let user = sqlx::query!(
        "SELECT id, email, role::text AS role, wizard_complete, totp_backup_codes FROM users WHERE id = $1",
        user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::Unauthorized)?;

    let codes = user.totp_backup_codes.unwrap_or_default();
    let matched_index = codes.iter().position(|hash| verify_backup_code(&req.code, hash));

    let Some(idx) = matched_index else {
        log_auth_failure(&ip, "bad_backup_code", &user.email);
        return Err(ApiError::InvalidTotp);
    };

    let mut remaining = codes;
    remaining.remove(idx);
    sqlx::query!(
        "UPDATE users SET totp_backup_codes = $1 WHERE id = $2",
        &remaining, user_id
    )
    .execute(&state.db)
    .await?;

    issue_session_jwt(state, user.id, user.email, user.role.unwrap_or_default(), user.wizard_complete).await
}

async fn issue_session_jwt(
    state: Arc<AppState>,
    user_id: Uuid,
    email: String,
    role: String,
    wizard_complete: bool,
) -> ApiResult<Json<AuthResponse>> {
    let session_claims = Claims {
        sub:             user_id.to_string(),
        jti:             Uuid::new_v4(),
        email:           email.clone(),
        role:            role.clone(),
        exp:             (Utc::now() + Duration::hours(8)).timestamp(),
        iat:             Utc::now().timestamp(),
        wizard_complete,
        totp_verified:   true,
        impersonated_by: None,
    };
    let access_token = encode_jwt(&session_claims, &state.jwt_encoding_key)?;

    // Generate a single-use refresh token, store in Valkey with 30-day TTL.
    let refresh_token = Uuid::new_v4().to_string();
    {
        use redis::AsyncCommands;
        let mut conn = state.valkey.clone();
        let key = format!("refresh:{}", refresh_token);
        let _: () = conn.set_ex(key, user_id.to_string(), 30 * 86400).await
            .unwrap_or(()); // non-fatal; client will need to re-login after expiry
    }

    sqlx::query!("UPDATE users SET last_login_at = NOW() WHERE id = $1", user_id)
        .execute(&state.db)
        .await?;

    tracing::info!(user_id = %user_id, "Login successful");

    Ok(Json(AuthResponse {
        access_token,
        refresh_token,
        expires_in:      8 * 3600,
        user_id,
        email,
        role,
        wizard_complete,
    }))
}

/// Refresh a session JWT (single-use refresh token stored in Valkey).
async fn refresh_token(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RefreshRequest>,
) -> ApiResult<Json<AuthResponse>> {
    let key = format!("refresh:{}", req.refresh_token);

    // Atomically fetch-and-delete the refresh token so concurrent requests with the same
    // token cannot both succeed (prevents replay / TOCTOU on token rotation).
    let user_id_str: Option<String> = {
        use redis::AsyncCommands;
        let mut conn = state.valkey.clone();
        // GETDEL is Redis ≥6.2 / Valkey ≥7.2 — atomically returns the value and deletes the key
        conn.get_del(&key).await.unwrap_or(None)
    };

    let user_id_str = user_id_str.ok_or(ApiError::Unauthorized)?;
    let user_id: Uuid = user_id_str.parse().map_err(|_| ApiError::Unauthorized)?;

    let user = sqlx::query!(
        "SELECT id, email, role::text AS role, wizard_complete FROM users WHERE id = $1 AND deleted_at IS NULL",
        user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::Unauthorized)?;

    let user_role = user.role.unwrap_or_default();
    let claims = Claims {
        sub:             user.id.to_string(),
        jti:             Uuid::new_v4(),
        email:           user.email.clone(),
        role:            user_role.clone(),
        exp:             (Utc::now() + Duration::hours(8)).timestamp(),
        iat:             Utc::now().timestamp(),
        wizard_complete: user.wizard_complete,
        totp_verified:   true,
        impersonated_by: None,
    };
    let access_token = encode_jwt(&claims, &state.jwt_encoding_key)?;

    // Issue a new refresh token (rotation: old token was already deleted by GETDEL)
    let new_refresh = Uuid::new_v4().to_string();
    {
        use redis::AsyncCommands;
        let mut conn = state.valkey.clone();
        let _: () = conn.set_ex(
            format!("refresh:{}", new_refresh),
            user.id.to_string(),
            30 * 86400,
        ).await.unwrap_or(());
    }

    Ok(Json(AuthResponse {
        access_token,
        refresh_token:   new_refresh,
        expires_in:      8 * 3600,
        user_id:         user.id,
        email:           user.email,
        role:            user_role,
        wizard_complete: user.wizard_complete,
    }))
}

/// Logout: blocklist this token's JTI only — other active sessions for the same user are unaffected.
async fn logout(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<StatusCode> {
    let ttl = (claims.exp - Utc::now().timestamp()).max(0) as u64;
    if ttl > 0 {
        use redis::AsyncCommands;
        let mut conn = state.valkey.clone();
        let _: () = conn
            .set_ex(format!("jottiecp:blocklist:{}", claims.jti), "1", ttl)
            .await
            .unwrap_or(());
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Phase 1 of TOTP setup: generate a secret and return the provisioning URI.
/// The secret is stored temporarily in Valkey (10 min TTL).
/// Call POST /api/v1/auth/totp/setup/confirm with the first code to activate.
async fn setup_totp(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(_req): Json<SetupTotpRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let user_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;

    let secret_bytes: Vec<u8> = {
        use rand::RngCore;
        let mut buf = vec![0u8; 20];
        rand::thread_rng().fill_bytes(&mut buf);
        buf
    };
    let secret_b32 = base32::encode(base32::Alphabet::RFC4648 { padding: false }, &secret_bytes);

    // Store pending secret in Valkey for 10 minutes
    {
        use redis::AsyncCommands;
        let mut conn = state.valkey.clone();
        let key = format!("jottiecp:totp_setup:{}", user_id);
        let _: () = conn.set_ex(&key, &secret_b32, 600u64).await.unwrap_or(());
    }

    let provisioning_uri = format!(
        "otpauth://totp/JottiCP:{email}?secret={secret}&issuer=JottiCP",
        email  = urlencoding::encode(&claims.email),
        secret = secret_b32,
    );

    Ok(Json(serde_json::json!({
        "secret":           secret_b32,
        "provisioning_uri": provisioning_uri,
        "instructions":     "Scan the QR code in your authenticator app, then call POST /api/v1/auth/totp/setup/confirm with {\"totp_code\":\"123456\"}"
    })))
}

/// Phase 2 of TOTP setup: verify the first code from the authenticator app.
/// Activates TOTP and returns one-time backup codes.
async fn confirm_totp(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<ConfirmTotpRequest>,
) -> ApiResult<Json<SetupTotpResponse>> {
    let user_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;

    // Retrieve pending secret from Valkey
    let secret_b32: String = {
        use redis::AsyncCommands;
        let mut conn = state.valkey.clone();
        let key = format!("jottiecp:totp_setup:{}", user_id);
        conn.get::<_, Option<String>>(&key).await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("Valkey error: {}", e)))?
            .ok_or_else(|| ApiError::Validation(
                "TOTP setup not initiated or expired. Call POST /api/v1/auth/totp/setup first.".into()
            ))?
    };

    let secret_bytes = base32::decode(base32::Alphabet::RFC4648 { padding: false }, &secret_b32)
        .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("Invalid stored TOTP secret")))?;

    let totp = totp_rs::TOTP::new(totp_rs::Algorithm::SHA1, 6, 1, 30, secret_bytes)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("{}", e)))?;

    if !totp.check_current(&req.totp_code).map_err(|e| ApiError::Internal(anyhow::anyhow!("{}", e)))? {
        return Err(ApiError::InvalidTotp);
    }

    // Generate backup codes
    let backup_codes: Vec<String> = (0..8)
        .map(|_| {
            use rand::distributions::Alphanumeric;
            use rand::Rng;
            rand::thread_rng().sample_iter(&Alphanumeric).take(12).map(char::from).collect()
        })
        .collect();
    let backup_hashes: Vec<String> = backup_codes.iter()
        .filter_map(|c| hash_backup_code(c))
        .collect();
    if backup_hashes.len() != backup_codes.len() {
        return Err(ApiError::Internal(anyhow::anyhow!("Argon2 backup code hashing failed")));
    }

    sqlx::query!(
        "UPDATE users SET totp_secret = $1, totp_backup_codes = $2 WHERE id = $3",
        secret_b32, &backup_hashes, user_id
    )
    .execute(&state.db)
    .await?;

    // Delete pending secret from Valkey
    {
        use redis::AsyncCommands;
        let mut conn = state.valkey.clone();
        let _: () = conn.del(format!("jottiecp:totp_setup:{}", user_id)).await.unwrap_or(());
    }

    let provisioning_uri = format!(
        "otpauth://totp/JottiCP:{email}?secret={secret}&issuer=JottiCP",
        email  = urlencoding::encode(&claims.email),
        secret = secret_b32,
    );

    Ok(Json(SetupTotpResponse { secret: secret_b32, provisioning_uri, backup_codes }))
}

/// Disable TOTP (requires current TOTP code — prevents CSRF from disabling 2FA).
async fn disable_totp(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(req): Json<TotpRequest>,
) -> ApiResult<StatusCode> {
    let user_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    let user = sqlx::query!("SELECT totp_secret FROM users WHERE id = $1", user_id)
        .fetch_one(&state.db)
        .await?;

    let secret_b32 = user.totp_secret.ok_or(ApiError::Unauthorized)?;
    let secret_bytes = base32::decode(base32::Alphabet::RFC4648 { padding: false }, &secret_b32)
        .ok_or(ApiError::Internal(anyhow::anyhow!("Invalid TOTP secret")))?;

    let totp = totp_rs::TOTP::new(totp_rs::Algorithm::SHA1, 6, 1, 30, secret_bytes)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("{}", e)))?;

    if !totp.check_current(&req.code).map_err(|e| ApiError::Internal(anyhow::anyhow!("{}", e)))? {
        return Err(ApiError::InvalidTotp);
    }

    sqlx::query!(
        "UPDATE users SET totp_secret = NULL, totp_backup_codes = NULL WHERE id = $1",
        user_id
    )
    .execute(&state.db)
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

// ── JWT helpers (ES256 / ECDSA P-256) ────────────────────────────────────────

/// Sign a JWT with ES256. `key` must be built via `EncodingKey::from_ec_pem(private_pem)`.
pub fn encode_jwt(claims: &Claims, key: &EncodingKey) -> anyhow::Result<String> {
    encode(
        &Header::new(Algorithm::ES256),
        claims,
        key,
    )
    .map_err(|e| anyhow::anyhow!("JWT encoding failed: {}", e))
}

/// Verify and decode a JWT signed with ES256. `key` must be built via `DecodingKey::from_ec_pem(public_pem)`.
pub fn decode_jwt(token: &str, key: &DecodingKey) -> anyhow::Result<Claims> {
    let mut validation = Validation::new(Algorithm::ES256);
    validation.validate_exp = true;
    let data = decode::<Claims>(token, key, &validation)
        .map_err(|e| anyhow::anyhow!("JWT decode failed: {}", e))?;
    Ok(data.claims)
}

// ── Password helpers ──────────────────────────────────────────────────────────

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    use argon2::{Argon2, PasswordHasher};
    use argon2::password_hash::{rand_core::OsRng, SaltString};
    let salt   = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    Ok(argon2.hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("Argon2 error: {}", e))?
        .to_string())
}

pub fn verify_password(password: &str, hash: &str) -> anyhow::Result<bool> {
    use argon2::{Argon2, PasswordVerifier};
    use argon2::password_hash::PasswordHash;
    let parsed = PasswordHash::new(hash)
        .map_err(|e| anyhow::anyhow!("Invalid password hash in DB: {}", e))?;
    Ok(Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok())
}

/// Hash a backup code with argon2 (not SHA-256 — low-entropy codes need a slow hash).
fn hash_backup_code(code: &str) -> Option<String> {
    use argon2::{Argon2, PasswordHasher};
    use argon2::password_hash::{rand_core::OsRng, SaltString};
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(code.as_bytes(), &salt)
        .ok()
        .map(|h| h.to_string())
}

/// Verify a backup code against its stored argon2 hash.
pub fn verify_backup_code(code: &str, hash: &str) -> bool {
    use argon2::{Argon2, PasswordVerifier};
    use argon2::password_hash::PasswordHash;
    let Ok(parsed) = PasswordHash::new(hash) else { return false; };
    Argon2::default().verify_password(code.as_bytes(), &parsed).is_ok()
}

// ── Audit logging ─────────────────────────────────────────────────────────────

pub fn log_auth_failure(ip: &str, reason: &str, email: &str) {
    tracing::warn!(
        target: "jottiecp_auth",
        "FAILED_LOGIN ip={} reason={} email={} ts={}",
        ip, reason, email, Utc::now().to_rfc3339()
    );
}

// ── Axum extractor for Claims ─────────────────────────────────────────────────

impl<S> axum::extract::FromRequestParts<S> for Claims
where
    S: Send + Sync,
    Arc<AppState>: axum::extract::FromRef<S>,
{
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        let app_state = <Arc<AppState> as axum::extract::FromRef<S>>::from_ref(state);

        let auth_header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or(ApiError::Unauthorized)?;

        let token = auth_header
            .strip_prefix("Bearer ")
            .ok_or(ApiError::Unauthorized)?;

        let claims = decode_jwt(token, &app_state.jwt_decoding_key)
            .map_err(|_| ApiError::Unauthorized)?;

        if !claims.totp_verified {
            return Err(ApiError::Unauthorized);
        }

        // Blocklist check by JTI (per-token), not by user ID (which would block all sessions)
        {
            use redis::AsyncCommands;
            let mut conn = app_state.valkey.clone();
            let blocked: Option<String> = conn
                .get(format!("jottiecp:blocklist:{}", claims.jti))
                .await
                .unwrap_or(None);
            if blocked.is_some() {
                return Err(ApiError::Unauthorized);
            }
        }

        Ok(claims)
    }
}
