//! Administrator-owned virtual tool definitions. A virtual server is a normal
//! mcp_servers/profile_servers/tool_discoveries row for the canonical catalog,
//! but has no downstream MCP session.
use super::Database;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdapterDefinition {
    pub name: String,
    pub tool_name: String,
    pub description: String,
    pub input_schema: Value,
    pub risk_level: String,
    pub adapter: AdapterKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum AdapterKind {
    Api {
        #[serde(rename = "baseUrl")]
        base_url: String,
        path: String,
        method: String,
        #[serde(default)]
        headers: std::collections::HashMap<String, String>,
        #[serde(default, rename = "queryFields")]
        query_fields: std::collections::HashMap<String, String>,
        #[serde(default, rename = "bodyFields")]
        body_fields: Vec<String>,
        #[serde(default, rename = "credentialRef")]
        credential_ref: Option<String>,
        #[serde(default, rename = "allowLocalHttp")]
        allow_local_http: bool,
        #[serde(rename = "timeoutMs")]
        timeout_ms: u64,
        #[serde(rename = "maxResponseBytes")]
        max_response_bytes: usize,
    },
    Cli {
        #[serde(rename = "bridgeServerId")]
        bridge_server_id: String,
        #[serde(rename = "bridgeToolName")]
        bridge_tool_name: String,
        executable: String,
        #[serde(rename = "workingDirectory")]
        working_directory: String,
        #[serde(default, rename = "fixedArgs")]
        fixed_args: Vec<String>,
        #[serde(default, rename = "argumentFields")]
        argument_fields: Vec<String>,
        #[serde(rename = "approvalId")]
        approval_id: String,
        #[serde(rename = "timeoutMs")]
        timeout_ms: u64,
        #[serde(rename = "maxOutputBytes")]
        max_output_bytes: usize,
    },
}

impl AdapterKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Api { .. } => "api",
            Self::Cli { .. } => "cli",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredAdapter {
    pub id: String,
    #[serde(flatten)]
    pub definition: AdapterDefinition,
}

pub struct AdapterRepository<'a> {
    db: &'a Database,
}

impl<'a> AdapterRepository<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub fn get(&self, id: &str) -> Result<Option<StoredAdapter>, String> {
        self.db
            .query_one(
                "SELECT server_id, definition FROM adapter_operations WHERE server_id = ?1",
                &[&id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )?
            .map(|(id, encoded)| {
                Ok(StoredAdapter {
                    id,
                    definition: serde_json::from_str(&encoded)
                        .map_err(|_| "Invalid stored adapter definition".to_string())?,
                })
            })
            .transpose()
    }

    pub fn list(&self) -> Result<Vec<StoredAdapter>, String> {
        self.db
            .query_all(
                "SELECT server_id, definition FROM adapter_operations ORDER BY server_id",
                &[],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )?
            .into_iter()
            .map(|(id, encoded)| {
                Ok(StoredAdapter {
                    id,
                    definition: serde_json::from_str(&encoded)
                        .map_err(|_| "Invalid stored adapter definition".to_string())?,
                })
            })
            .collect()
    }

    pub fn ids(&self) -> Result<Vec<String>, String> {
        self.db
            .query_all("SELECT server_id FROM adapter_operations", &[], |row| {
                row.get(0)
            })
    }

    /// Insert the virtual server, its discoverable tool, and active-profile
    /// membership atomically. Profile assignment afterwards uses existing UI.
    pub fn create(&self, definition: &AdapterDefinition) -> Result<StoredAdapter, String> {
        let id = uuid::Uuid::new_v4().to_string();
        let serialized = serde_json::to_string(definition).map_err(|e| e.to_string())?;
        let schema = serde_json::to_string(&definition.input_schema).map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().to_rfc3339();
        self.db.transaction(|conn| {
            conn.execute("INSERT INTO mcp_servers (id, name, connection_type, status, instructions, created_at, updated_at)
                VALUES (?1, ?2, 'http', 'running', '', ?3, ?3)", rusqlite::params![id, definition.name, now])
                .map_err(|e| e.to_string())?;
            conn.execute("INSERT INTO adapter_operations (server_id, kind, definition) VALUES (?1, ?2, ?3)",
                rusqlite::params![id, definition.adapter.as_str(), serialized])
                .map_err(|e| e.to_string())?;
            conn.execute("INSERT INTO tool_discoveries (server_id, tool_name, exposed_name, description, input_schema, risk_level, risk_source, discovered_at)
                VALUES (?1, ?2, ?2, ?3, ?4, ?5, 'user', ?6)",
                rusqlite::params![id, definition.tool_name, definition.description, schema, definition.risk_level, now])
                .map_err(|e| e.to_string())?;
            conn.execute("INSERT OR IGNORE INTO profile_servers (profile_id, server_id, enabled, disabled_tools)
                SELECT id, ?1, 1, '[]' FROM profiles WHERE is_active = 1", [&id])
                .map_err(|e| e.to_string())?;
            Ok(())
        })?;
        Ok(StoredAdapter {
            id,
            definition: definition.clone(),
        })
    }

    /// Retain the stable server ID and tool name, including stored profile
    /// deny/pin lists. Updating a tool name requires deleting and recreating.
    pub fn update(
        &self,
        id: &str,
        definition: &AdapterDefinition,
    ) -> Result<StoredAdapter, String> {
        let old = self.get(id)?.ok_or("Adapter not found")?;
        if old.definition.tool_name != definition.tool_name {
            return Err("toolName is immutable; create a new adapter".into());
        }
        let serialized = serde_json::to_string(definition).map_err(|e| e.to_string())?;
        let schema = serde_json::to_string(&definition.input_schema).map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().to_rfc3339();
        self.db.transaction(|conn| {
            conn.execute("UPDATE mcp_servers SET name = ?1, updated_at = ?2 WHERE id = ?3",
                rusqlite::params![definition.name, now, id]).map_err(|e| e.to_string())?;
            conn.execute("UPDATE adapter_operations SET kind = ?1, definition = ?2 WHERE server_id = ?3",
                rusqlite::params![definition.adapter.as_str(), serialized, id]).map_err(|e| e.to_string())?;
            conn.execute("UPDATE tool_discoveries SET description = ?1, input_schema = ?2, risk_level = ?3, risk_source = 'user' WHERE server_id = ?4",
                rusqlite::params![definition.description, schema, definition.risk_level, id]).map_err(|e| e.to_string())?;
            Ok(())
        })?;
        Ok(StoredAdapter {
            id: id.into(),
            definition: definition.clone(),
        })
    }

    pub fn delete(&self, id: &str) -> Result<bool, String> {
        if self.get(id)?.is_none() {
            return Ok(false);
        }
        self.db
            .run("DELETE FROM mcp_servers WHERE id = ?1", &[&id])?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sidecar::db::profile_repo::ProfileRepository;
    use crate::sidecar::services::tool_catalog::ToolCatalogService;
    use serde_json::json;

    #[test]
    fn virtual_tools_persist_in_existing_profile_catalog() {
        let path = std::env::temp_dir().join(format!("moor-adapter-{}.db", uuid::Uuid::new_v4()));
        let db = Database::open(&path).unwrap();
        db.run_migrations().unwrap();
        ProfileRepository::new(&db).seed_default().unwrap();
        let def = AdapterDefinition {
            name: "Example".into(),
            tool_name: "read".into(),
            description: "Read only".into(),
            input_schema: json!({"type":"object","properties":{},"additionalProperties":false}),
            risk_level: "safe".into(),
            adapter: AdapterKind::Api {
                base_url: "https://example.com".into(),
                path: "/api".into(),
                method: "GET".into(),
                headers: Default::default(),
                query_fields: Default::default(),
                body_fields: vec![],
                credential_ref: None,
                allow_local_http: false,
                timeout_ms: 1000,
                max_response_bytes: 1024,
            },
        };
        let repo = AdapterRepository::new(&db);
        let adapter = repo.create(&def).unwrap();
        let tools = ToolCatalogService::load_active_profile(&db)
            .unwrap()
            .resolve(None)
            .callable_tools();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].exposed_name, "example__read");
        assert_eq!(tools[0].server_id, adapter.id);
        assert_eq!(tools[0].risk_level, "safe");
        assert_eq!(
            repo.get(&adapter.id).unwrap().unwrap().definition.tool_name,
            "read"
        );
        assert!(repo
            .update(
                &adapter.id,
                &AdapterDefinition {
                    tool_name: "new".into(),
                    ..def.clone()
                }
            )
            .is_err());
        repo.update(
            &adapter.id,
            &AdapterDefinition {
                description: "Changed".into(),
                ..def
            },
        )
        .unwrap();
        drop(db);
        let db = Database::open(&path).unwrap();
        db.run_migrations().unwrap();
        assert_eq!(
            AdapterRepository::new(&db)
                .get(&adapter.id)
                .unwrap()
                .unwrap()
                .definition
                .description,
            "Changed"
        );
        assert!(AdapterRepository::new(&db).delete(&adapter.id).unwrap());
        assert!(ToolCatalogService::load_active_profile(&db)
            .unwrap()
            .resolve(None)
            .callable_tools()
            .is_empty());
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}
