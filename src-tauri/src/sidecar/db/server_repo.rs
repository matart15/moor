use super::Database;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    pub id: String,
    pub name: String,
    pub connection_type: String,
    pub status: String,
    pub auto_start: bool,
    pub instructions: String,
    pub command: Option<String>,
    pub args: Option<serde_json::Value>,
    pub url: Option<String>,
    pub env: Option<serde_json::Value>,
    pub headers: Option<serde_json::Value>,
    pub working_dir: Option<String>,
    pub error_message: Option<String>,
    pub sort_order: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

/// 写入 mcp_servers 所需的字段。repo 负责生成 id/时间戳/sort_order/status,
/// 并把 args/env/headers 序列化成 JSON 文本列。调用方只需提供领域字段。
pub struct ServerInsertInput {
    pub name: String,
    pub connection_type: String,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub url: Option<String>,
    pub env: Option<HashMap<String, String>>,
    pub headers: Option<HashMap<String, String>>,
    pub working_dir: Option<String>,
    pub auto_start: bool,
    pub instructions: Option<String>,
}

fn serialize_nullable_text<T: serde::Serialize>(
    field: &str,
    value: &Option<T>,
) -> Result<Option<String>, String> {
    value
        .as_ref()
        .map(|v| serde_json::to_string(v).map_err(|e| format!("serialize {field}: {e}")))
        .transpose()
}

pub(crate) fn map_server(row: &rusqlite::Row<'_>) -> rusqlite::Result<Server> {
    let args_str: Option<String> = row.get("args")?;
    let env_str: Option<String> = row.get("env")?;
    let headers_str: Option<String> = row.get("headers")?;
    let auto_start: i64 = row.get("auto_start")?;
    let sort_order: i64 = row.get("sort_order")?;

    Ok(Server {
        id: row.get("id")?,
        name: row.get("name")?,
        connection_type: row.get("connection_type")?,
        status: row.get("status")?,
        auto_start: auto_start != 0,
        instructions: row.get("instructions")?,
        command: row.get("command")?,
        args: args_str
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .or_else(|| Some(serde_json::Value::Array(vec![]))),
        url: row.get("url")?,
        env: env_str.and_then(|s| serde_json::from_str(&s).ok()),
        headers: headers_str.and_then(|s| serde_json::from_str(&s).ok()),
        working_dir: row.get("working_dir")?,
        error_message: row.get("error_message")?,
        sort_order: if sort_order == 0 {
            Some(0)
        } else {
            Some(sort_order)
        },
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

pub struct ServerRepository<'a> {
    pub db: &'a Database,
}

impl<'a> ServerRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub fn find_all(&self) -> Result<Vec<Server>, String> {
        self.db.query_all(
            "SELECT * FROM mcp_servers ORDER BY sort_order ASC, created_at DESC",
            &[],
            map_server,
        )
    }

    pub fn find_all_names(&self) -> Result<Vec<(String, String)>, String> {
        self.db.query_all(
            "SELECT id, name FROM mcp_servers ORDER BY name ASC",
            &[],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
    }

    pub fn find_ids(&self) -> Result<Vec<String>, String> {
        self.db
            .query_all("SELECT id FROM mcp_servers", &[], |row| row.get(0))
    }

    pub fn find_by_id(&self, id: &str) -> Result<Option<Server>, String> {
        self.db.query_one(
            "SELECT * FROM mcp_servers WHERE id = ?1",
            &[&id],
            map_server,
        )
    }

    pub fn find_by_ids(&self, ids: &[String]) -> Result<Vec<Server>, String> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let unique: Vec<String> = ids
            .iter()
            .cloned()
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        let mut all_rows = Vec::new();
        for chunk in unique.chunks(500) {
            let placeholders: Vec<String> = (1..=chunk.len()).map(|i| format!("?{i}")).collect();
            let sql = format!(
                "SELECT * FROM mcp_servers WHERE id IN ({})",
                placeholders.join(",")
            );
            let params: Vec<&dyn rusqlite::types::ToSql> = chunk
                .iter()
                .map(|s| s as &dyn rusqlite::types::ToSql)
                .collect();
            let rows = self.db.query_all(&sql, &params, map_server)?;
            all_rows.extend(rows);
        }
        let by_id: std::collections::HashMap<String, Server> =
            all_rows.into_iter().map(|s| (s.id.clone(), s)).collect();
        Ok(ids.iter().filter_map(|id| by_id.get(id).cloned()).collect())
    }

    /// 事务内批量插入 mcp_servers,并把每条新服务器挂到当前活动 profile 上。
    /// SQL、JSON 序列化和 profile_servers 关联保持在 db 模块内。
    pub fn insert_batch_with_active_profile(
        &self,
        inputs: &[ServerInsertInput],
    ) -> Result<Vec<Server>, String> {
        if inputs.is_empty() {
            return Ok(vec![]);
        }
        self.db.transaction(|conn| {
            let mut next_sort_order = match conn.query_row(
                "SELECT MIN(sort_order) FROM mcp_servers",
                [],
                |row| row.get::<_, Option<i64>>(0),
            ) {
                Ok(Some(value)) => value - 1,
                Ok(None) => 0,
                Err(e) => return Err(e.to_string()),
            };
            let active_profile_id = match conn.query_row(
                "SELECT id FROM profiles WHERE is_active = 1",
                [],
                |row| row.get::<_, String>(0),
            ) {
                Ok(id) => Some(id),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(e) => return Err(e.to_string()),
            };

            let mut servers = Vec::with_capacity(inputs.len());
            for input in inputs {
                let id = uuid::Uuid::new_v4().to_string();
                let now = chrono::Utc::now().to_rfc3339();
                let args_json = serialize_nullable_text("args", &input.args)?;
                let env_json = serialize_nullable_text("env", &input.env)?;
                let headers_json = serialize_nullable_text("headers", &input.headers)?;

                conn.execute(
                    "INSERT INTO mcp_servers (id, name, connection_type, command, args, url, env, headers, working_dir, auto_start, instructions, sort_order, status, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'stopped', ?13, ?14)",
                    rusqlite::params![
                        &id,
                        &input.name,
                        &input.connection_type,
                        input.command.as_deref(),
                        args_json.as_deref(),
                        input.url.as_deref(),
                        env_json.as_deref(),
                        headers_json.as_deref(),
                        input.working_dir.as_deref(),
                        input.auto_start as i64,
                        input.instructions.as_deref().unwrap_or(""),
                        next_sort_order,
                        &now,
                        &now,
                    ],
                )
                .map_err(|e| e.to_string())?;

                if let Some(profile_id) = &active_profile_id {
                    conn.execute(
                        "INSERT OR IGNORE INTO profile_servers (profile_id, server_id, enabled, disabled_tools) VALUES (?1, ?2, 1, '[]')",
                        rusqlite::params![profile_id, &id],
                    )
                    .map_err(|e| e.to_string())?;
                }

                let server = conn
                    .query_row(
                        "SELECT * FROM mcp_servers WHERE id = ?1",
                        rusqlite::params![&id],
                        map_server,
                    )
                    .map_err(|e| e.to_string())?;
                servers.push(server);
                next_sort_order -= 1;
            }
            Ok(servers)
        })
    }

    /// 测试专用:用指定 id/时间戳/sort_order 插入单行,字段以结构体形式传入。
    /// 生产路径请用 [insert_batch_with_active_profile],它会自动生成标识符
    /// 并把新服务器挂到活动 profile 上。
    #[cfg(test)]
    pub fn insert_one_with_id(
        &self,
        id: &str,
        sort_order: i64,
        input: &ServerInsertInput,
    ) -> Result<(), String> {
        let now = chrono::Utc::now().to_rfc3339();
        let args_json = serialize_nullable_text("args", &input.args)?;
        let env_json = serialize_nullable_text("env", &input.env)?;
        let headers_json = serialize_nullable_text("headers", &input.headers)?;
        self.db.run(
            "INSERT INTO mcp_servers (id, name, connection_type, command, args, url, env, headers, working_dir, auto_start, instructions, sort_order, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'stopped', ?13, ?14)",
            &[
                &id as &dyn rusqlite::types::ToSql,
                &input.name,
                &input.connection_type,
                &input.command,
                &args_json,
                &input.url,
                &env_json,
                &headers_json,
                &input.working_dir,
                &(input.auto_start as i64),
                &input.instructions.as_deref().unwrap_or(""),
                &sort_order,
                &now,
                &now,
            ],
        )
    }

    pub fn update(
        &self,
        id: &str,
        set_clauses: &str,
        params: &[&dyn rusqlite::types::ToSql],
    ) -> Result<(), String> {
        let sql = format!("UPDATE mcp_servers SET {set_clauses} WHERE id = ?");
        let mut all_params: Vec<&dyn rusqlite::types::ToSql> = params.to_vec();
        all_params.push(&id);
        self.db.run(&sql, &all_params)
    }

    pub fn remove(&self, id: &str) -> Result<(), String> {
        self.db.transaction(|conn| {
            conn.execute(
                "UPDATE audit_logs SET server_id = NULL WHERE server_id = ?1",
                [id],
            )
            .map_err(|e| e.to_string())?;
            conn.execute("DELETE FROM mcp_servers WHERE id = ?1", [id])
                .map_err(|e| e.to_string())?;
            Ok(())
        })
    }

    pub fn reorder(&self, ids: &[String]) -> Result<(), String> {
        let now = chrono::Utc::now().to_rfc3339();
        self.db.transaction(|conn| {
            for (index, id) in ids.iter().enumerate() {
                conn.execute(
                    "UPDATE mcp_servers SET sort_order = ?1, updated_at = ?2 WHERE id = ?3",
                    rusqlite::params![index as i64, &now, id],
                )
                .map_err(|e| e.to_string())?;
            }
            Ok(())
        })
    }

    pub fn update_status(
        &self,
        id: &str,
        status: &str,
        error_message: Option<&str>,
    ) -> Result<(), String> {
        let now = chrono::Utc::now().to_rfc3339();
        self.db.run(
            "UPDATE mcp_servers SET status = ?1, error_message = ?2, updated_at = ?3 WHERE id = ?4",
            &[&status, &error_message, &now, &id],
        )
    }

    pub fn reset_running_statuses(&self) -> Result<(), String> {
        self.db.run(
            "UPDATE mcp_servers SET status = 'stopped', error_message = NULL WHERE status IN ('running', 'starting')",
            &[],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn temp_db() -> Database {
        let ts = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("moor-server-repo-{ts}.db"));
        let db = Database::open(&path).expect("open db");
        db.run_migrations().expect("migrate");
        db
    }

    fn sample_input(name: &str) -> ServerInsertInput {
        ServerInsertInput {
            name: name.into(),
            connection_type: "stdio".into(),
            command: Some("node".into()),
            args: None,
            url: None,
            env: None,
            headers: None,
            working_dir: None,
            auto_start: false,
            instructions: None,
        }
    }

    #[test]
    fn find_by_ids_preserves_requested_order_and_skips_missing() {
        let db = temp_db();
        let repo = ServerRepository::new(&db);
        let first = repo
            .insert_batch_with_active_profile(&[sample_input("first")])
            .expect("insert first")[0]
            .id
            .clone();
        let second = repo
            .insert_batch_with_active_profile(&[sample_input("second")])
            .expect("insert second")[0]
            .id
            .clone();

        let rows = repo
            .find_by_ids(&[second.clone(), "missing".to_string(), first.clone()])
            .expect("find_by_ids");

        assert_eq!(
            rows.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            vec![second.as_str(), first.as_str()]
        );
    }

    #[test]
    fn insert_batch_assigns_decreasing_sort_order_and_returns_full_rows() {
        let db = temp_db();
        let repo = ServerRepository::new(&db);
        let inputs = vec![sample_input("a"), sample_input("b"), sample_input("c")];
        let servers = repo
            .insert_batch_with_active_profile(&inputs)
            .expect("insert batch");

        assert_eq!(servers.len(), 3);
        // 空表时 MIN(sort_order) 返回 NULL → next=0;之后每条 -1。
        let orders: Vec<i64> = servers.iter().map(|s| s.sort_order.unwrap()).collect();
        assert_eq!(orders, vec![0, -1, -2]);
        // 所有行 status = stopped,字段回读完整
        for s in &servers {
            assert_eq!(s.status, "stopped");
            assert_eq!(s.connection_type, "stdio");
            assert_eq!(s.command.as_deref(), Some("node"));
        }
    }

    #[test]
    fn insert_batch_with_empty_input_returns_empty_vec() {
        let db = temp_db();
        let repo = ServerRepository::new(&db);
        let servers = repo
            .insert_batch_with_active_profile(&[])
            .expect("empty batch");
        assert!(servers.is_empty());
    }
}
