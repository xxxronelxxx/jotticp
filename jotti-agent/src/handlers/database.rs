//! Database creation and deletion handlers: CreateDatabase, DeleteDatabase.

use crate::journal::Journal;
use crate::proto::{CreateDatabaseRequest, DatabaseResponse, DeleteDatabaseRequest, StatusResponse};
use crate::validation::{validate_db_ident, validate_db_type};
use std::process::Command;
use tonic::Status;
use tracing::{info, warn};

// ── CreateDatabase ────────────────────────────────────────────────────────────

pub async fn create_database(
    req: CreateDatabaseRequest,
    journal: &Journal,
) -> Result<tonic::Response<DatabaseResponse>, Status> {
    let db_type = validate_db_type(&req.db_type)?.to_string();
    let db_name = validate_db_ident(&req.db_name)?;
    let db_user = validate_db_ident(&req.db_user)?;

    if req.db_password.len() < 16 {
        return Err(Status::invalid_argument(
            "db_password must be at least 16 characters",
        ));
    }

    info!(db_type = %db_type, db_name = %db_name, db_user = %db_user, "create_database");

    let op_id = journal
        .begin_op(
            "create_database",
            None,
            &serde_json::json!({ "db_type": db_type, "db_name": db_name, "db_user": db_user }),
        )
        .await
        .map_err(|e| Status::internal(format!("journal error: {}", e)))?;

    let result = do_create_database(&db_type, &db_name, &db_user, &req.db_password);

    match &result {
        Ok(_)  => { let _ = journal.finish_op(&op_id).await; }
        Err(e) => { let _ = journal.fail_op(&op_id, &e.message()).await; }
    }

    result
}

fn do_create_database(
    db_type: &str,
    db_name: &str,
    db_user: &str,
    db_password: &str,
) -> Result<tonic::Response<DatabaseResponse>, Status> {
    let connection_string = match db_type {
        "mysql" | "mariadb" => {
            create_mysql_database(db_name, db_user, db_password)?;
            format!("mysql://{}:{}@127.0.0.1:3306/{}", db_user, db_password, db_name)
        }
        "postgresql" => {
            create_postgresql_database(db_name, db_user, db_password)?;
            format!("postgresql://{}:{}@127.0.0.1:5432/{}", db_user, db_password, db_name)
        }
        "ferretdb" => {
            // FerretDB uses PostgreSQL as backend; create a PG schema
            create_postgresql_database(db_name, db_user, db_password)?;
            format!("mongodb://{}:{}@127.0.0.1:27017/{}", db_user, db_password, db_name)
        }
        "surrealdb" => {
            // SurrealDB is provisioned as a separate service — return connection string
            let port = surrealdb_allocate_port(db_name)?;
            format!("ws://127.0.0.1:{}/{}", port, db_name)
        }
        other => {
            return Err(Status::invalid_argument(format!("unsupported db_type: {}", other)));
        }
    };

    Ok(tonic::Response::new(DatabaseResponse {
        connection_string,
        success: true,
        error_msg: String::new(),
    }))
}

fn create_mysql_database(db_name: &str, db_user: &str, db_password: &str) -> Result<(), Status> {
    // Use mysql CLI with --execute to avoid shell interpolation risks.
    // All identifiers are validated with validate_db_ident before this call.
    let sql = format!(
        "CREATE DATABASE IF NOT EXISTS `{db_name}` CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;\
         CREATE USER IF NOT EXISTS '{db_user}'@'localhost' IDENTIFIED BY '{db_password}';\
         GRANT ALL PRIVILEGES ON `{db_name}`.* TO '{db_user}'@'localhost';\
         FLUSH PRIVILEGES;",
        db_name   = db_name,
        db_user   = db_user,
        db_password = db_password,
    );

    let status = Command::new("/usr/bin/mysql")
        .args(["--defaults-file=/etc/jotticp/mysql-admin.cnf", "--execute", &sql])
        .status()
        .map_err(|e| Status::internal(format!("mysql exec failed: {}", e)))?;

    if !status.success() {
        return Err(Status::internal(format!(
            "mysql CREATE DATABASE/USER failed with exit code {:?}",
            status.code()
        )));
    }

    info!(db_name = %db_name, db_user = %db_user, "MySQL database created");
    Ok(())
}

fn create_postgresql_database(
    db_name: &str,
    db_user: &str,
    db_password: &str,
) -> Result<(), Status> {
    // Create role
    let sql_role = format!(
        "DO $$ BEGIN \
           IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = '{db_user}') \
           THEN CREATE ROLE \"{db_user}\" LOGIN PASSWORD '{db_password}'; END IF; \
         END $$;",
        db_user     = db_user,
        db_password = db_password,
    );

    run_psql(&sql_role)?;

    // Create database owned by role
    let sql_db = format!(
        "SELECT 'CREATE DATABASE \"{db_name}\" OWNER \"{db_user}\"' WHERE NOT EXISTS \
         (SELECT FROM pg_database WHERE datname = '{db_name}')\\gexec",
        db_name = db_name,
        db_user = db_user,
    );

    run_psql(&sql_db)?;

    // Revoke public schema access
    let sql_revoke = format!(
        "REVOKE ALL ON DATABASE \"{db_name}\" FROM PUBLIC;\
         GRANT ALL ON DATABASE \"{db_name}\" TO \"{db_user}\";",
        db_name = db_name,
        db_user = db_user,
    );

    run_psql(&sql_revoke)?;

    info!(db_name = %db_name, db_user = %db_user, "PostgreSQL database created");
    Ok(())
}

fn run_psql(sql: &str) -> Result<(), Status> {
    let status = Command::new("/usr/bin/psql")
        .args(["-U", "postgres", "-c", sql])
        .status()
        .map_err(|e| Status::internal(format!("psql exec failed: {}", e)))?;

    if !status.success() {
        return Err(Status::internal(format!(
            "psql failed with exit code {:?}",
            status.code()
        )));
    }
    Ok(())
}

fn surrealdb_allocate_port(db_name: &str) -> Result<u16, Status> {
    // Deterministic port from a hash of db_name in range 40100-49999
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    db_name.hash(&mut hasher);
    let h = hasher.finish();
    let port = 40100u16 + (h % 9900) as u16;
    Ok(port)
}

// ── DeleteDatabase ────────────────────────────────────────────────────────────

pub async fn delete_database(
    req: DeleteDatabaseRequest,
    journal: &Journal,
) -> Result<tonic::Response<StatusResponse>, Status> {
    let db_type = validate_db_type(&req.db_type)?.to_string();
    let db_name = validate_db_ident(&req.db_name)?;
    let db_user = validate_db_ident(&req.db_user)?;

    info!(db_type = %db_type, db_name = %db_name, db_user = %db_user, "delete_database");

    let op_id = journal
        .begin_op(
            "delete_database",
            None,
            &serde_json::json!({ "db_type": db_type, "db_name": db_name, "db_user": db_user }),
        )
        .await
        .map_err(|e| Status::internal(format!("journal error: {}", e)))?;

    let result = do_delete_database(&db_type, &db_name, &db_user);

    match &result {
        Ok(_)  => { let _ = journal.finish_op(&op_id).await; }
        Err(e) => { let _ = journal.fail_op(&op_id, &e.message()).await; }
    }

    result
}

fn do_delete_database(
    db_type: &str,
    db_name: &str,
    db_user: &str,
) -> Result<tonic::Response<StatusResponse>, Status> {
    match db_type {
        "mysql" | "mariadb" => drop_mysql_database(db_name, db_user)?,
        "postgresql" | "ferretdb" => drop_postgresql_database(db_name, db_user)?,
        "surrealdb" => {
            warn!(%db_name, "SurrealDB cleanup not yet automated — manual removal required");
        }
        other => {
            return Err(Status::invalid_argument(format!("unsupported db_type: {}", other)));
        }
    }

    Ok(tonic::Response::new(StatusResponse {
        success: true,
        message: format!("database {} deleted", db_name),
    }))
}

fn drop_mysql_database(db_name: &str, db_user: &str) -> Result<(), Status> {
    let sql = format!(
        "DROP DATABASE IF EXISTS `{db_name}`;\
         DROP USER IF EXISTS '{db_user}'@'localhost';\
         FLUSH PRIVILEGES;",
        db_name = db_name,
        db_user = db_user,
    );

    let status = Command::new("/usr/bin/mysql")
        .args(["--defaults-file=/etc/jotticp/mysql-admin.cnf", "--execute", &sql])
        .status()
        .map_err(|e| Status::internal(format!("mysql exec failed: {}", e)))?;

    if !status.success() {
        return Err(Status::internal(format!(
            "mysql DROP DATABASE failed with exit code {:?}",
            status.code()
        )));
    }

    info!(db_name = %db_name, "MySQL database dropped");
    Ok(())
}

fn drop_postgresql_database(db_name: &str, db_user: &str) -> Result<(), Status> {
    // Terminate active connections first
    let sql_terminate = format!(
        "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '{}';",
        db_name
    );
    let _ = run_psql(&sql_terminate);

    let sql_drop = format!("DROP DATABASE IF EXISTS \"{}\";", db_name);
    run_psql(&sql_drop)?;

    let sql_drop_user = format!("DROP ROLE IF EXISTS \"{}\";", db_user);
    run_psql(&sql_drop_user)?;

    info!(db_name = %db_name, "PostgreSQL database dropped");
    Ok(())
}
