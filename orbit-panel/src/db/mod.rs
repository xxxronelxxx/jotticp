// Database helpers and query utilities

/// Return the total row count for a query that can be paginated.
///
/// This runs a `SELECT COUNT(*) FROM (...)` wrapper around the caller's query.
/// The caller is responsible for providing a syntactically valid SQL string;
/// sqlx compile-time checks do not apply here (dynamic query).
///
/// # Example
/// ```rust
/// let total = db::paginate_count(
///     &state.db,
///     "SELECT id FROM sites WHERE status = 'active' AND deleted_at IS NULL",
/// ).await?;
/// ```
///
/// Note: for simple cases prefer `sqlx::query_scalar!("SELECT COUNT(*) FROM ...")`
/// directly — this helper is for when the inner query is built dynamically.
pub async fn paginate_count(db: &sqlx::PgPool, query: &str) -> anyhow::Result<i64> {
    let wrapped = format!("SELECT COUNT(*) FROM ({}) AS _paginate_count_inner", query);
    let count: i64 = sqlx::query_scalar(&wrapped)
        .fetch_one(db)
        .await
        .map_err(|e| anyhow::anyhow!("paginate_count failed: {}", e))?;
    Ok(count)
}

/// Run a paginated fetch and return total count alongside a page of results.
///
/// Because sqlx cannot do compile-time checks on fully dynamic SQL, callers that
/// need static query verification should use `sqlx::query_as!` / `sqlx::query!`
/// directly and call `paginate_count` separately for the total.
///
/// The `offset_clause` helper below makes it ergonomic to append LIMIT/OFFSET.
pub fn offset_clause(limit: i64, offset: i64) -> String {
    format!(" LIMIT {} OFFSET {}", limit.max(1).min(1000), offset.max(0))
}

/// Clamp a page limit to a sane range.
///
/// - Default: 100 rows
/// - Maximum: 500 rows (prevent huge result sets over the API)
pub fn clamp_limit(limit: Option<i64>) -> i64 {
    limit.unwrap_or(100).clamp(1, 500)
}

/// Clamp an offset to be non-negative.
pub fn clamp_offset(offset: Option<i64>) -> i64 {
    offset.unwrap_or(0).max(0)
}

/// A standard paginated response envelope.
///
/// All list endpoints should return this shape so the frontend can render
/// total-count pagination controls consistently.
#[derive(serde::Serialize, Debug)]
pub struct Page<T: serde::Serialize> {
    pub items:  Vec<T>,
    pub total:  i64,
    pub limit:  i64,
    pub offset: i64,
}

impl<T: serde::Serialize> Page<T> {
    pub fn new(items: Vec<T>, total: i64, limit: i64, offset: i64) -> Self {
        Self { items, total, limit, offset }
    }
}
