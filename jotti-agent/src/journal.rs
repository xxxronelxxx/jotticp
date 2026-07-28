//! SQLite-backed operation journal for replay safety.
//!
//! Before executing any destructive or long-running operation, the agent writes
//! a journal entry with status = "pending".  On completion it updates the entry
//! to "done" or "failed".  If the agent crashes mid-operation, jotti-panel can
//! query pending entries on reconnect and replay or roll them back.

use anyhow::Result;
use chrono::Utc;
use sqlx::{sqlite::SqliteConnectOptions, SqlitePool};
use std::str::FromStr;
use uuid::Uuid;

/// The status of a journalled operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpStatus {
    Pending,
    Done,
    Failed,
}

impl OpStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            OpStatus::Pending => "pending",
            OpStatus::Done    => "done",
            OpStatus::Failed  => "failed",
        }
    }
}

/// A journal entry describing a single agent operation.
#[derive(Debug)]
pub struct JournalEntry {
    pub id:         String,
    pub op_type:    String,
    pub domain:     Option<String>,
    pub payload:    serde_json::Value,
    pub status:     String,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
    pub error_msg:  Option<String>,
}

/// The journal is a thin wrapper around a SQLite pool.
#[derive(Debug, Clone)]
pub struct Journal {
    pool: SqlitePool,
}

impl Journal {
    /// Open (or create) the journal database at `path` and run migrations.
    pub async fn new(path: &str) -> Result<Self> {
        let url = format!("sqlite://{}?mode=rwc", path);
        let opts = SqliteConnectOptions::from_str(&url)?
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);

        let pool = SqlitePool::connect_with(opts).await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS journal (
                id          TEXT PRIMARY KEY,
                op_type     TEXT NOT NULL,
                domain      TEXT,
                payload     TEXT NOT NULL DEFAULT '{}',
                status      TEXT NOT NULL DEFAULT 'pending',
                created_at  TEXT NOT NULL,
                updated_at  TEXT NOT NULL,
                error_msg   TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_journal_status   ON journal(status);
            CREATE INDEX IF NOT EXISTS idx_journal_domain   ON journal(domain);
            CREATE INDEX IF NOT EXISTS idx_journal_created  ON journal(created_at);
            "#,
        )
        .execute(&pool)
        .await?;

        Ok(Journal { pool })
    }

    /// Record an operation as "pending" before execution.
    /// Returns the generated entry ID.
    pub async fn begin_op(
        &self,
        op_type: &str,
        domain: Option<&str>,
        payload: &serde_json::Value,
    ) -> Result<String> {
        let id  = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            "INSERT INTO journal (id, op_type, domain, payload, status, created_at, updated_at) \
             VALUES (?, ?, ?, ?, 'pending', ?, ?)",
        )
        .bind(&id)
        .bind(op_type)
        .bind(domain)
        .bind(payload.to_string())
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        Ok(id)
    }

    /// Mark a previously-begun operation as "done".
    pub async fn finish_op(&self, id: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE journal SET status = 'done', updated_at = ? WHERE id = ?",
        )
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Mark a previously-begun operation as "failed" with an error message.
    pub async fn fail_op(&self, id: &str, error: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE journal SET status = 'failed', error_msg = ?, updated_at = ? WHERE id = ?",
        )
        .bind(error)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// List all pending operations (useful on startup for replay).
    pub async fn pending_ops(&self) -> Result<Vec<JournalEntry>> {
        let rows: Vec<JournalEntryRow> = sqlx::query_as(
            "SELECT id, op_type, domain, payload, status, created_at, updated_at, error_msg \
             FROM journal WHERE status = 'pending' ORDER BY created_at ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| JournalEntry {
                id:         r.id,
                op_type:    r.op_type,
                domain:     r.domain,
                payload:    serde_json::from_str(&r.payload).unwrap_or(serde_json::Value::Null),
                status:     r.status,
                created_at: chrono::DateTime::parse_from_rfc3339(&r.created_at)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                updated_at: chrono::DateTime::parse_from_rfc3339(&r.updated_at)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                error_msg:  r.error_msg,
            })
            .collect())
    }
}

#[allow(dead_code)]
#[derive(sqlx::FromRow)]
struct JournalEntryRow {
    id:         String,
    op_type:    String,
    domain:     Option<String>,
    payload:    String,
    status:     String,
    created_at: String,
    updated_at: String,
    error_msg:  Option<String>,
}
