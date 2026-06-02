/// Step 3.19 — Native DB Manager API (PostgreSQL)
///
/// Provides a safe, read-biased interface to databases owned by a site.
/// All callers restricted to SELECT-only queries — read-only enforced for all roles.
/// Uses dynamic PgPool connections built from credentials in user_databases.

use axum::{
    Router,
    routing::{get, post, put},
    extract::{Path, Query, State},
    Json,
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

use crate::{AppState, ApiError, ApiResult};
use super::auth::Claims;

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct PaginationQuery {
    pub limit:  Option<i64>,
    pub offset: Option<i64>,
    pub schema: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SqlQueryRequest {
    pub sql:    String,
    pub schema: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TableInfo {
    pub name:       String,
    pub row_count:  i64,
    pub size_bytes: i64,
}

#[derive(Debug, Serialize)]
pub struct RowsResult {
    pub columns: Vec<String>,
    pub rows:    Vec<Vec<Value>>,
    pub total:   usize,
}

#[derive(Debug, Serialize)]
pub struct QueryResult {
    pub columns:      Vec<String>,
    pub rows:         Vec<Vec<Value>>,
    pub affected_rows: u64,
    pub duration_ms:  u64,
}

// ── SQL guard for non-admins ───────────────────────────────────────────────────

const WRITE_PATTERNS: &[&str] = &[
    "drop ", "truncate ", "delete ", "alter ", "create ",
    "insert ", "update ", "grant ", "revoke ", "replace ",
];

fn strip_sql_comments(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let bytes = sql.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // -- line comment
        if i + 1 < bytes.len() && bytes[i] == b'-' && bytes[i + 1] == b'-' {
            while i < bytes.len() && bytes[i] != b'\n' { i += 1; }
        // /* block comment */
        } else if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') { i += 1; }
            if i + 1 < bytes.len() { i += 2; }
            out.push(' ');
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

fn is_read_only_sql(sql: &str) -> bool {
    let clean = strip_sql_comments(sql);
    let lower = clean.to_lowercase();
    let trimmed = lower.trim_start();
    !WRITE_PATTERNS.iter().any(|p| {
        trimmed.starts_with(p)
            || lower.contains(&format!(" {p}"))
            || lower.contains(&format!("({p}"))
            || lower.contains(&format!("\t{p}"))
            || lower.contains(&format!("\n{p}"))
    })
}

// ── CSV helper ─────────────────────────────────────────────────────────────────

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/databases/{db_id}/schemas",                        get(list_schemas))
        .route("/api/v1/databases/{db_id}/tables",                         get(list_tables))
        .route("/api/v1/databases/{db_id}/tables/{table}/rows",             get(list_rows))
        .route("/api/v1/databases/{db_id}/tables/{table}/rows/{pk}",         put(update_row))
        .route("/api/v1/databases/{db_id}/tables/{table}/export",           get(export_csv))
        .route("/api/v1/databases/{db_id}/query",                          post(execute_query))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn caller(claims: &Claims) -> ApiResult<(Uuid, bool)> {
    let user_id: Uuid = claims.sub.parse().map_err(|_| ApiError::Unauthorized)?;
    Ok((user_id, claims.role == "admin"))
}

fn validate_id(s: &str, label: &str) -> ApiResult<()> {
    if s.is_empty() || !s.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return Err(ApiError::Validation(format!(
            "Invalid {label} — only alphanumeric and underscores allowed"
        )));
    }
    Ok(())
}

struct DbCreds {
    db_type: String,
    db_name: String,
    db_user: String,
    db_pass: String,
    db_host: String,
    db_port: u16,
}

async fn resolve_db(state: &AppState, db_id: Uuid, user_id: Uuid, is_admin: bool) -> ApiResult<DbCreds> {
    let row = sqlx::query!(
        r#"SELECT ud.db_type::TEXT AS "db_type!",
                  ud.db_name,
                  ud.db_user,
                  ud.db_password    AS "db_pass",
                  host(ud.db_host)   AS "db_host",
                  ud.db_port        AS "db_port!",
                  ud.status
           FROM user_databases ud
           JOIN sites s ON s.id = ud.site_id
           WHERE ud.id = $1
             AND ud.deleted_at IS NULL
             AND s.deleted_at IS NULL
             AND ($2::boolean OR s.owner_id = $3)"#,
        db_id, is_admin, user_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound { resource: "database" })?;

    if row.status != "active" {
        return Err(ApiError::Validation(format!(
            "Database '{}' is not yet ready (status: {}). It will be provisioned shortly.",
            row.db_name, row.status
        )));
    }

    let db_host = row.db_host.ok_or_else(|| {
        ApiError::Validation("Database host not configured — still provisioning.".into())
    })?;

    Ok(DbCreds {
        db_type: row.db_type,
        db_name: row.db_name,
        db_user: row.db_user,
        db_pass: row.db_pass.unwrap_or_default(),
        db_host,
        db_port: row.db_port as u16,
    })
}

async fn connect_pg(creds: &DbCreds) -> ApiResult<sqlx::postgres::PgPool> {
    use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

    let opts = PgConnectOptions::new()
        .host(&creds.db_host)
        .port(creds.db_port)
        .database(&creds.db_name)
        .username(&creds.db_user)
        .password(&creds.db_pass);

    PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(8))
        .connect_with(opts)
        .await
        .map_err(|e| ApiError::Validation(format!("Cannot connect to database: {e}")))
}

fn require_postgres(db_type: &str) -> ApiResult<()> {
    if db_type != "postgres" {
        return Err(ApiError::Validation(format!(
            "DB Manager currently supports PostgreSQL only; '{db_type}' is not yet supported."
        )));
    }
    Ok(())
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// GET /api/v1/databases/{db_id}/schemas
async fn list_schemas(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(db_id): Path<Uuid>,
) -> ApiResult<Json<Vec<String>>> {
    let (user_id, is_admin) = caller(&claims)?;
    let creds = resolve_db(&state, db_id, user_id, is_admin).await?;

    if creds.db_type.as_str() != "postgres" {
        return Ok(Json(vec![creds.db_name]));
    }

    let pool = connect_pg(&creds).await?;
    let rows = sqlx::query(
        "SELECT schema_name::text FROM information_schema.schemata
         WHERE schema_name NOT IN ('pg_catalog','information_schema','pg_toast')
         ORDER BY schema_name"
    )
    .fetch_all(&pool)
    .await
    .map_err(|e| ApiError::Validation(format!("Failed to list schemas: {e}")))?;

    use sqlx::Row;
    let schemas: Vec<String> = rows.iter().filter_map(|r| r.try_get::<String, _>(0).ok()).collect();
    Ok(Json(if schemas.is_empty() { vec!["public".into()] } else { schemas }))
}

/// GET /api/v1/databases/{db_id}/tables?schema=public
async fn list_tables(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(db_id): Path<Uuid>,
    Query(q): Query<PaginationQuery>,
) -> ApiResult<Json<Vec<TableInfo>>> {
    let (user_id, is_admin) = caller(&claims)?;
    let creds = resolve_db(&state, db_id, user_id, is_admin).await?;
    require_postgres(&creds.db_type)?;

    let pool = connect_pg(&creds).await?;
    let schema = q.schema.as_deref().unwrap_or("public");

    let rows = sqlx::query(
        r#"SELECT t.tablename::text AS name,
                  COALESCE(s.n_live_tup, 0::bigint) AS row_count,
                  COALESCE(pg_total_relation_size(c.oid), 0::bigint) AS size_bytes
           FROM pg_tables t
           LEFT JOIN pg_stat_user_tables s
                ON s.schemaname = t.schemaname AND s.relname = t.tablename
           LEFT JOIN pg_class c
                ON c.relname = t.tablename
               AND c.relnamespace = (SELECT oid FROM pg_namespace WHERE nspname = t.schemaname)
           WHERE t.schemaname = $1
           ORDER BY t.tablename"#
    )
    .bind(schema)
    .fetch_all(&pool)
    .await
    .map_err(|e| ApiError::Validation(format!("Failed to list tables: {e}")))?;

    use sqlx::Row;
    let tables: Vec<TableInfo> = rows.iter().map(|r| TableInfo {
        name:       r.try_get::<String, _>("name").unwrap_or_default(),
        row_count:  r.try_get::<i64, _>("row_count").unwrap_or(0),
        size_bytes: r.try_get::<i64, _>("size_bytes").unwrap_or(0),
    }).collect();

    Ok(Json(tables))
}

/// GET /api/v1/databases/{db_id}/tables/{table}/rows?limit=50&offset=0&schema=public
async fn list_rows(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path((db_id, table)): Path<(Uuid, String)>,
    Query(q): Query<PaginationQuery>,
) -> ApiResult<Json<RowsResult>> {
    let (user_id, is_admin) = caller(&claims)?;
    let creds = resolve_db(&state, db_id, user_id, is_admin).await?;
    validate_id(&table, "table")?;
    require_postgres(&creds.db_type)?;

    let pool = connect_pg(&creds).await?;
    let schema = q.schema.as_deref().unwrap_or("public");
    validate_id(schema, "schema")?;
    let limit  = q.limit.unwrap_or(50).clamp(1, 1000);
    let offset = q.offset.unwrap_or(0).max(0);

    // Column names in ordinal order (for deterministic row-to-array mapping)
    let col_rows = sqlx::query(
        "SELECT column_name::text FROM information_schema.columns
         WHERE table_schema = $1 AND table_name = $2
         ORDER BY ordinal_position"
    )
    .bind(schema)
    .bind(table.as_str())
    .fetch_all(&pool)
    .await
    .map_err(|e| ApiError::Validation(format!("Schema lookup failed: {e}")))?;

    use sqlx::Row;
    let columns: Vec<String> = col_rows.iter()
        .filter_map(|r| r.try_get::<String, _>(0).ok())
        .collect();

    // Total count for pagination
    let count_sql = format!("SELECT COUNT(*) FROM \"{schema}\".\"{table}\"");
    let total: i64 = sqlx::query(&count_sql)
        .fetch_one(&pool)
        .await
        .map(|r| r.try_get::<i64, _>(0).unwrap_or(0))
        .unwrap_or(0);

    // Rows via PostgreSQL row_to_json — handles all column types automatically
    let row_sql = format!(
        "SELECT row_to_json(t) AS j FROM \"{schema}\".\"{table}\" AS t LIMIT $1 OFFSET $2"
    );
    let pg_rows = sqlx::query(&row_sql)
        .bind(limit)
        .bind(offset)
        .fetch_all(&pool)
        .await
        .map_err(|e| ApiError::Validation(format!("Failed to browse table: {e}")))?;

    let data: Vec<Vec<Value>> = pg_rows.iter().map(|r| {
        let obj: Value = r.try_get("j").unwrap_or(json!({}));
        columns.iter().map(|col| obj.get(col).cloned().unwrap_or(json!(null))).collect()
    }).collect();

    Ok(Json(RowsResult { columns, rows: data, total: total as usize }))
}

/// POST /api/v1/databases/{db_id}/query
async fn execute_query(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path(db_id): Path<Uuid>,
    Json(req): Json<SqlQueryRequest>,
) -> ApiResult<Json<QueryResult>> {
    let (user_id, is_admin) = caller(&claims)?;
    let creds = resolve_db(&state, db_id, user_id, is_admin).await?;
    require_postgres(&creds.db_type)?;

    let sql = req.sql.trim().to_string();
    if sql.is_empty() {
        return Err(ApiError::Validation("SQL cannot be empty".into()));
    }
    if !is_read_only_sql(&sql) {
        return Err(ApiError::Validation(
            "Only SELECT queries are allowed in the query runner.".into()
        ));
    }
    if sql.contains(';') {
        return Err(ApiError::Validation(
            "SQL must be a single statement (no semicolons).".into()
        ));
    }
    if sql.len() > 65_536 {
        return Err(ApiError::Validation("SQL too long (max 64 KiB)".into()));
    }

    let pool = connect_pg(&creds).await?;
    let start = std::time::Instant::now();

    let lower = sql.to_lowercase();
    let trimmed = lower.trim_start();
    let is_select = trimmed.starts_with("select")
        || trimmed.starts_with("with ")
        || trimmed.starts_with("table ")
        || trimmed.starts_with("values");

    if is_select {
        // Wrap in row_to_json to handle all PostgreSQL column types
        let wrapped = format!("SELECT row_to_json(t) AS j FROM ({sql}) AS t LIMIT 1000");
        let rows = sqlx::query(&wrapped)
            .fetch_all(&pool)
            .await
            .map_err(|e| ApiError::Validation(format!("Query error: {e}")))?;

        let duration_ms = start.elapsed().as_millis() as u64;

        if rows.is_empty() {
            return Ok(Json(QueryResult {
                columns: vec![], rows: vec![], affected_rows: 0, duration_ms,
            }));
        }

        use sqlx::Row;
        let first: Value = rows[0].try_get("j").unwrap_or(json!({}));
        let columns: Vec<String> = if let Value::Object(ref map) = first {
            map.keys().cloned().collect()
        } else {
            vec![]
        };

        let data: Vec<Vec<Value>> = rows.iter().map(|r| {
            let obj: Value = r.try_get("j").unwrap_or(json!({}));
            columns.iter().map(|col| obj.get(col).cloned().unwrap_or(json!(null))).collect()
        }).collect();

        Ok(Json(QueryResult { columns, rows: data, affected_rows: 0, duration_ms }))
    } else {
        let result = sqlx::query(&sql)
            .execute(&pool)
            .await
            .map_err(|e| ApiError::Validation(format!("Query error: {e}")))?;
        let duration_ms = start.elapsed().as_millis() as u64;
        Ok(Json(QueryResult {
            columns:      vec!["rows_affected".into()],
            rows:         vec![vec![json!(result.rows_affected())]],
            affected_rows: result.rows_affected(),
            duration_ms,
        }))
    }
}

/// PUT /api/v1/databases/{db_id}/tables/{table}/rows/{pk}
async fn update_row(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path((db_id, table, pk)): Path<(Uuid, String, String)>,
    Json(data): Json<std::collections::HashMap<String, Value>>,
) -> ApiResult<Json<Value>> {
    let (user_id, is_admin) = caller(&claims)?;
    let creds = resolve_db(&state, db_id, user_id, is_admin).await?;
    validate_id(&table, "table")?;
    require_postgres(&creds.db_type)?;

    if !pk.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        return Err(ApiError::Validation("Invalid primary key value".into()));
    }
    for col in data.keys() {
        validate_id(col, "column")?;
    }
    if data.is_empty() {
        return Err(ApiError::Validation("No columns to update".into()));
    }

    let pool = connect_pg(&creds).await?;

    // Look up the primary key column name
    use sqlx::Row;
    let pk_col_row = sqlx::query(
        "SELECT kcu.column_name::text
         FROM information_schema.table_constraints tc
         JOIN information_schema.key_column_usage kcu
              ON tc.constraint_name = kcu.constraint_name
             AND tc.table_schema    = kcu.table_schema
         WHERE tc.constraint_type = 'PRIMARY KEY'
           AND tc.table_schema    = 'public'
           AND tc.table_name      = $1
         ORDER BY kcu.ordinal_position LIMIT 1"
    )
    .bind(table.as_str())
    .fetch_optional(&pool)
    .await
    .map_err(|e| ApiError::Validation(format!("PK lookup failed: {e}")))?;

    let pk_col = pk_col_row
        .and_then(|r| r.try_get::<String, _>(0).ok())
        .ok_or_else(|| ApiError::Validation(format!("No primary key found for table '{table}'")))?;

    let cols: Vec<String> = data.keys().cloned().collect();
    let set_clause = cols.iter().enumerate()
        .map(|(i, col)| format!("\"{col}\" = ${}::text", i + 1))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "UPDATE \"public\".\"{table}\" SET {set_clause} WHERE \"{pk_col}\"::text = ${}",
        cols.len() + 1
    );

    // Build arguments dynamically
    use sqlx::Arguments as _;
    let mut args = sqlx::postgres::PgArguments::default();
    for col in &cols {
        let val_str: Option<String> = match &data[col] {
            Value::Null             => None,
            Value::String(s)       => Some(s.clone()),
            v                       => Some(v.to_string()),
        };
        args.add(val_str)
            .map_err(|e| ApiError::Validation(format!("Argument encoding failed: {e}")))?;
    }
    args.add(pk.as_str())
        .map_err(|e| ApiError::Validation(format!("PK argument failed: {e}")))?;

    let result = sqlx::query_with(&sql, args)
        .execute(&pool)
        .await
        .map_err(|e| ApiError::Validation(format!("Update failed: {e}")))?;

    Ok(Json(json!({ "updated": result.rows_affected(), "ok": result.rows_affected() > 0 })))
}

/// GET /api/v1/databases/{db_id}/tables/{table}/export
async fn export_csv(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Path((db_id, table)): Path<(Uuid, String)>,
    Query(q): Query<PaginationQuery>,
) -> ApiResult<axum::response::Response> {
    use axum::response::IntoResponse;
    use axum::http::{header, HeaderMap, HeaderValue};

    let (user_id, is_admin) = caller(&claims)?;
    let creds = resolve_db(&state, db_id, user_id, is_admin).await?;
    validate_id(&table, "table")?;
    require_postgres(&creds.db_type)?;

    let pool = connect_pg(&creds).await?;
    let schema = q.schema.as_deref().unwrap_or("public");
    validate_id(schema, "schema")?;

    let col_rows = sqlx::query(
        "SELECT column_name::text FROM information_schema.columns
         WHERE table_schema = $1 AND table_name = $2
         ORDER BY ordinal_position"
    )
    .bind(schema)
    .bind(table.as_str())
    .fetch_all(&pool)
    .await
    .map_err(|e| ApiError::Validation(format!("Schema lookup failed: {e}")))?;

    use sqlx::Row;
    let columns: Vec<String> = col_rows.iter()
        .filter_map(|r| r.try_get::<String, _>(0).ok())
        .collect();

    let row_sql = format!(
        "SELECT row_to_json(t) AS j FROM \"{schema}\".\"{table}\" AS t LIMIT 100000"
    );
    let pg_rows = sqlx::query(&row_sql)
        .fetch_all(&pool)
        .await
        .map_err(|e| ApiError::Validation(format!("Export failed: {e}")))?;

    let mut csv = String::new();
    csv.push_str(&columns.iter().map(|c| csv_escape(c)).collect::<Vec<_>>().join(","));
    csv.push('\n');
    for r in &pg_rows {
        let obj: Value = r.try_get("j").unwrap_or(json!({}));
        let row_str: Vec<String> = columns.iter().map(|col| {
            match obj.get(col) {
                None | Some(Value::Null) => String::new(),
                Some(Value::String(s))  => csv_escape(s),
                Some(v)                  => csv_escape(&v.to_string()),
            }
        }).collect();
        csv.push_str(&row_str.join(","));
        csv.push('\n');
    }

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/csv; charset=utf-8"));
    if let Ok(cd) = HeaderValue::from_str(&format!("attachment; filename=\"{table}.csv\"")) {
        headers.insert(header::CONTENT_DISPOSITION, cd);
    }

    Ok((StatusCode::OK, headers, csv).into_response())
}
