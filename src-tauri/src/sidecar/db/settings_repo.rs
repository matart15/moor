use super::Database;
use std::collections::HashMap;

pub fn load_entries(db: &Database) -> Result<HashMap<String, String>, String> {
    let rows = db.query_all("SELECT key, value FROM settings", &[], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    Ok(rows.into_iter().collect())
}

pub fn save_entries(db: &Database, entries: &[(String, String)]) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    db.transaction(|conn| {
        // Remove keys written by the old dotted-map format before saving its JSON leaf.
        conn.execute("DELETE FROM settings WHERE key LIKE 'bridge.agentProfiles.%'", [])
            .map_err(|e| e.to_string())?;
        for (key, value) in entries {
            conn.execute(
                "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)",
                rusqlite::params![key, value, &now],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
}
