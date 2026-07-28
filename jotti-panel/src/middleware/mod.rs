pub mod admin_ip;
pub mod audit;
pub mod feature_flags;
pub mod metrics;
pub mod security_headers;

// Re-export Claims and helpers from api::auth so that modules that import
// `crate::middleware::auth::Claims` (and `require_admin_role`) continue to compile.
pub mod auth {
    pub use crate::api::auth::{Claims, decode_jwt};

    use axum::{http::StatusCode, middleware::Next, response::Response};

    /// Route-layer middleware: rejects requests that are not carrying a valid
    /// `admin`-role JWT.
    ///
    /// Usage (requires state to be available in extensions via `with_state`):
    ///   ```
    ///   use axum::middleware;
    ///   router.route_layer(middleware::from_fn_with_state(state.clone(), require_admin_role))
    ///   ```
    pub async fn require_admin_role(
        axum::extract::State(state): axum::extract::State<std::sync::Arc<crate::AppState>>,
        req: axum::extract::Request,
        next: Next,
    ) -> Result<Response, StatusCode> {
        let auth_header = req
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or(StatusCode::UNAUTHORIZED)?;

        let token = auth_header
            .strip_prefix("Bearer ")
            .ok_or(StatusCode::UNAUTHORIZED)?;

        let claims = decode_jwt(token, &state.jwt_decoding_key)
            .map_err(|_| StatusCode::UNAUTHORIZED)?;

        if !claims.totp_verified || claims.role != "admin" {
            return Err(StatusCode::FORBIDDEN);
        }

        Ok(next.run(req).await)
    }
}
