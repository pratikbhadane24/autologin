//! SQLite database: opening, migrations, and a pre-migration backup.

use std::path::Path;

use include_dir::{include_dir, Dir};
use rusqlite::Connection;
use rusqlite_migration::{Migrations, M};
use thiserror::Error;

static MIGRATIONS_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/migrations");

pub const DB_FILE: &str = "autologin.db";

#[derive(Debug, Error)]
pub enum DbError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("database migration failed: {0}")]
    Migration(#[from] rusqlite_migration::Error),
    #[error("could not back up the database before upgrading: {0}")]
    Backup(std::io::Error),
}

fn migrations() -> Migrations<'static> {
    let mut files: Vec<_> = MIGRATIONS_DIR.files().collect();
    files.sort_by_key(|f| f.path());
    Migrations::new(files.into_iter().filter_map(|f| f.contents_utf8()).map(M::up).collect())
}

fn configure(conn: &Connection) -> Result<(), DbError> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "busy_timeout", 5_000)?;
    Ok(())
}

/// Open (creating if needed) and migrate the database in `data_dir`. If a
/// migration is pending on an existing database, it is copied to
/// `autologin.db.bak-v<old version>` first.
pub fn open(data_dir: &Path) -> Result<Connection, DbError> {
    let path = data_dir.join(DB_FILE);
    let existed = path.exists();
    let mut conn = Connection::open(&path)?;
    configure(&conn)?;

    let migrations = migrations();
    let current: usize = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let pending = migrations.pending_migrations(&conn)? > 0;
    if existed && pending {
        let backup = data_dir.join(format!("{DB_FILE}.bak-v{current}"));
        conn.execute("VACUUM INTO ?1", [backup.to_string_lossy()]).map_err(|e| {
            DbError::Backup(std::io::Error::other(e.to_string()))
        })?;
        tracing::info!(from = current, backup = %backup.display(), "backed up database before migration");
    }
    migrations.to_latest(&mut conn)?;
    Ok(conn)
}

/// In-memory database with the full schema, for tests.
pub fn open_in_memory() -> Result<Connection, DbError> {
    let mut conn = Connection::open_in_memory()?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    migrations().to_latest(&mut conn)?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_are_valid() {
        assert!(migrations().validate().is_ok());
    }

    #[test]
    fn creates_schema_on_disk_and_reopens() {
        let dir = tempfile::tempdir().unwrap();
        drop(open(dir.path()).unwrap());
        let conn = open(dir.path()).unwrap();
        let tables: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('accounts','runs','settings')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tables, 3);
    }

    #[test]
    fn same_client_in_two_tenants_is_allowed_but_not_twice_in_one() {
        let conn = open_in_memory().unwrap();
        let insert = "INSERT INTO accounts (tenant_id, broker_id, client_id, added_on) VALUES (?1, 'zerodha', 'AB1', 'x')";
        conn.execute(insert, ["cirrus"]).unwrap();
        conn.execute(insert, ["pocketful"]).unwrap();
        assert!(conn.execute(insert, ["cirrus"]).is_err());
    }
}
