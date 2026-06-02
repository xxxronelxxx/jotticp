use axum::{
    Router,
    routing::{get, post, put},
    extract::{Path, Query, State},
    Json,
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{AppState, ApiError, ApiResult};
use super::auth::Claims;

#[derive(Debug, Deserialize)]
pub struct ListNotificationsQuery {
    pub unread_only: Option<bool>,
    pub limit:       Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct NotificationResponse {
    pub id:         Uuid,
    #[serde(rename = "type")]
    pub kind:       String,
    pub title:      String,
    pub body:       String,
    pub action_url: Option<String>,
    pub read_at:    Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub site_id:    Option<Uuid>,
    pub server_id:  Option<Uuid>,
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/notifications",               get(list_notifications))
        .route("/api/v1/notifications/mark-all-read", post(mark_all_read))
        .route("/api/v1/notifications/{id}/read",      put(mark_read))
}

async fn list_notifications(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Query(q): Query<ListNotificationsQuery>,
) -> ApiResult<Json<Vec<NotificationResponse>>> {
    let user_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    let limit = q.limit.unwrap_or(50).min(200);
    let unread_only = q.unread_only.unwrap_or(false);

    let rows = sqlx::query!(
        r#"SELECT id, type::text AS kind, title, body, action_url,
                  read_at, created_at, site_id, server_id
           FROM notifications
           WHERE (user_id = $1 OR user_id IS NULL)
             AND ($2::boolean = false OR read_at IS NULL)
           ORDER BY created_at DESC
           LIMIT $3"#,
        user_id, unread_only, limit
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows.into_iter().map(|r| NotificationResponse {
        id:         r.id,
        kind:       r.kind.unwrap_or_default(),
        title:      r.title,
        body:       r.body,
        action_url: r.action_url,
        read_at:    r.read_at,
        created_at: r.created_at,
        site_id:    r.site_id,
        server_id:  r.server_id,
    }).collect()))
}

async fn mark_read(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let user_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    sqlx::query!(
        "UPDATE notifications SET read_at = NOW()
         WHERE id = $1 AND (user_id = $2 OR user_id IS NULL) AND read_at IS NULL",
        id, user_id
    )
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn mark_all_read(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> ApiResult<StatusCode> {
    let user_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    sqlx::query!(
        "UPDATE notifications SET read_at = NOW()
         WHERE (user_id = $1 OR user_id IS NULL) AND read_at IS NULL",
        user_id
    )
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
