pub mod adapter_repo;
pub mod audit_log_repo;
mod migrations;
pub mod profile_repo;
pub mod server_repo;
pub mod settings_repo;
pub mod tool_discovery_repo;

use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    pub fn open(db_path: &Path) -> Result<Self, String> {
        let conn = Connection::open(db_path).map_err(|e| e.to_string())?;
        conn.execute_batch("PRAGMA foreign_keys = ON")
            .map_err(|e| e.to_string())?;
        conn.execute_batch("PRAGMA journal_mode = WAL")
            .map_err(|e| e.to_string())?;
        conn.execute_batch("PRAGMA busy_timeout = 5000")
            .map_err(|e| e.to_string())?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn run(&self, sql: &str, params: &[&dyn rusqlite::types::ToSql]) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(sql, params).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn exec(&self, sql: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute_batch(sql).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn query_one<F, T>(
        &self,
        sql: &str,
        params: &[&dyn rusqlite::types::ToSql],
        map_row: F,
    ) -> Result<Option<T>, String>
    where
        F: FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
        match stmt.query_row(params, map_row) {
            Ok(row) => Ok(Some(row)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn query_all<F, T>(
        &self,
        sql: &str,
        params: &[&dyn rusqlite::types::ToSql],
        map_row: F,
    ) -> Result<Vec<T>, String>
    where
        F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
        let rows = stmt.query_map(params, map_row).map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    /// Execute a function inside a transaction. The closure receives `&Connection`
    /// with the lock held for the entire transaction scope.
    pub fn transaction<T, F>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&Connection) -> Result<T, String>,
    {
        let guard = self.conn.lock().map_err(|e| e.to_string())?;
        guard
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|e| e.to_string())?;
        match f(&guard) {
            Ok(result) => {
                guard.execute_batch("COMMIT").map_err(|e| e.to_string())?;
                Ok(result)
            }
            Err(e) => {
                guard
                    .execute_batch("ROLLBACK")
                    .map_err(|e2| format!("{e}; rollback: {e2}"))?;
                Err(e)
            }
        }
    }

    pub fn run_migrations(&self) -> Result<(), String> {
        migrations::run_migrations(self)
    }
}
