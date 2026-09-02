use std::{path::Path, time::Duration};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{ConnectOptions, Row, SqlitePool};
use uuid::Uuid;
#[cfg(test)]
use webtop_contracts::OperationKind;
use webtop_contracts::{
    ApiError, EnvironmentSpec, Operation, OperationPhase, ServerSettings, Template,
};

#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentRecord {
    pub id: Uuid,
    pub name: String,
    pub container_id: String,
    pub config_path: String,
    pub desired_running: bool,
    pub local_port: Option<u16>,
    pub template_id: Option<Uuid>,
    pub spec: EnvironmentSpec,
    pub created_at: DateTime<Utc>,
}

impl Database {
    pub async fn open(path: &Path) -> Result<Self> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5))
            .foreign_keys(true)
            .disable_statement_logging();
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .context("open controller database")?;
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS environments (
              id TEXT PRIMARY KEY,
              name TEXT NOT NULL UNIQUE,
              container_id TEXT NOT NULL UNIQUE,
              config_path TEXT NOT NULL,
              desired_running INTEGER NOT NULL DEFAULT 1,
              local_port INTEGER,
              spec_json TEXT NOT NULL,
              created_at TEXT NOT NULL
            );
            "#,
        )
        .execute(&pool)
        .await?;
        ensure_column(&pool, "environments", "template_id", "TEXT").await?;
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS operations (
              id TEXT PRIMARY KEY,
              kind TEXT NOT NULL,
              phase TEXT NOT NULL,
              progress_percent INTEGER NOT NULL,
              cancellable INTEGER NOT NULL,
              resource_id TEXT,
              error_code TEXT,
              error_params_json TEXT,
              updated_at TEXT NOT NULL
            );
            "#,
        )
        .execute(&pool)
        .await?;
        ensure_column(&pool, "operations", "result_json", "TEXT").await?;
        ensure_column(&pool, "operations", "created_at", "TEXT").await?;
        ensure_column(
            &pool,
            "operations",
            "log_json",
            "TEXT NOT NULL DEFAULT '[]'",
        )
        .await?;
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS templates (
              id TEXT PRIMARY KEY,
              name TEXT NOT NULL UNIQUE,
              parent_template_id TEXT,
              source_environment_id TEXT,
              record_json TEXT NOT NULL,
              created_at TEXT NOT NULL,
              FOREIGN KEY(parent_template_id) REFERENCES templates(id) ON DELETE RESTRICT
            );
            "#,
        )
        .execute(&pool)
        .await?;
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS server_settings (
              singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
              settings_json TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );
            "#,
        )
        .execute(&pool)
        .await?;
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS frpc_service_state (
              singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
              desired_running INTEGER NOT NULL DEFAULT 0,
              updated_at TEXT NOT NULL
            );
            "#,
        )
        .execute(&pool)
        .await?;
        Ok(Self { pool })
    }

    pub async fn get_server_settings(&self) -> Result<ServerSettings> {
        let value: Option<String> =
            sqlx::query_scalar("SELECT settings_json FROM server_settings WHERE singleton = 1")
                .fetch_optional(&self.pool)
                .await?;
        value
            .map(|json| serde_json::from_str(&json).context("parse server settings"))
            .transpose()
            .map(|settings| settings.unwrap_or_default())
    }

    pub async fn save_server_settings(&self, settings: &ServerSettings) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO server_settings (singleton, settings_json, updated_at)
               VALUES (1, ?, ?)
               ON CONFLICT(singleton) DO UPDATE SET
                 settings_json = excluded.settings_json,
                 updated_at = excluded.updated_at"#,
        )
        .bind(serde_json::to_string(settings)?)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn frpc_desired_running(&self) -> Result<bool> {
        let value: Option<bool> = sqlx::query_scalar(
            "SELECT desired_running FROM frpc_service_state WHERE singleton = 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(value.unwrap_or(false))
    }

    pub async fn set_frpc_desired_running(&self, running: bool) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO frpc_service_state (singleton, desired_running, updated_at)
               VALUES (1, ?, ?)
               ON CONFLICT(singleton) DO UPDATE SET
                 desired_running = excluded.desired_running,
                 updated_at = excluded.updated_at"#,
        )
        .bind(running)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn environment_name_exists(&self, name: &str) -> Result<bool> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM environments WHERE name = ?")
            .bind(name)
            .fetch_one(&self.pool)
            .await?;
        Ok(count != 0)
    }

    pub async fn insert_environment(&self, record: &EnvironmentRecord) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO environments
               (id, name, container_id, config_path, desired_running, local_port, template_id, spec_json, created_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(record.id.to_string())
        .bind(&record.name)
        .bind(&record.container_id)
        .bind(&record.config_path)
        .bind(record.desired_running)
        .bind(record.local_port.map(i64::from))
        .bind(record.template_id.map(|id| id.to_string()))
        .bind(serde_json::to_string(&record.spec)?)
        .bind(record.created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_environments(&self) -> Result<Vec<EnvironmentRecord>> {
        let rows = sqlx::query(
            "SELECT id, name, container_id, config_path, desired_running, local_port, template_id, spec_json, created_at FROM environments ORDER BY created_at",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(parse_environment).collect()
    }

    pub async fn get_environment(&self, id: Uuid) -> Result<Option<EnvironmentRecord>> {
        let row = sqlx::query(
            "SELECT id, name, container_id, config_path, desired_running, local_port, template_id, spec_json, created_at FROM environments WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.map(parse_environment).transpose()
    }

    pub async fn set_desired_running(&self, id: Uuid, running: bool) -> Result<()> {
        sqlx::query("UPDATE environments SET desired_running = ? WHERE id = ?")
            .bind(running)
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_environment_local_port(
        &self,
        id: Uuid,
        local_port: Option<u16>,
    ) -> Result<()> {
        sqlx::query("UPDATE environments SET local_port = ? WHERE id = ?")
            .bind(local_port.map(i64::from))
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn update_environment_spec(&self, id: Uuid, spec: &EnvironmentSpec) -> Result<()> {
        sqlx::query("UPDATE environments SET spec_json = ? WHERE id = ?")
            .bind(serde_json::to_string(spec)?)
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_environment(&self, id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM environments WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn template_name_exists(&self, name: &str) -> Result<bool> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM templates WHERE name = ?")
            .bind(name)
            .fetch_one(&self.pool)
            .await?;
        Ok(count != 0)
    }

    pub async fn insert_template(&self, template: &Template) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO templates
               (id, name, parent_template_id, source_environment_id, record_json, created_at)
               VALUES (?, ?, ?, ?, ?, ?)"#,
        )
        .bind(template.id.to_string())
        .bind(&template.name)
        .bind(template.parent_template_id.map(|id| id.to_string()))
        .bind(template.source_environment_id.map(|id| id.to_string()))
        .bind(serde_json::to_string(template)?)
        .bind(template.created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_template(&self, template: &Template) -> Result<()> {
        sqlx::query("UPDATE templates SET name = ?, record_json = ? WHERE id = ?")
            .bind(&template.name)
            .bind(serde_json::to_string(template)?)
            .bind(template.id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_templates(&self) -> Result<Vec<Template>> {
        let values: Vec<String> =
            sqlx::query_scalar("SELECT record_json FROM templates ORDER BY created_at DESC")
                .fetch_all(&self.pool)
                .await?;
        values
            .into_iter()
            .map(|json| serde_json::from_str(&json).context("parse template record"))
            .collect()
    }

    pub async fn get_template(&self, id: Uuid) -> Result<Option<Template>> {
        let value: Option<String> =
            sqlx::query_scalar("SELECT record_json FROM templates WHERE id = ?")
                .bind(id.to_string())
                .fetch_optional(&self.pool)
                .await?;
        value
            .map(|json| serde_json::from_str(&json).context("parse template record"))
            .transpose()
    }

    pub async fn template_dependency_counts(&self, id: Uuid) -> Result<(i64, i64)> {
        let environments: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM environments WHERE template_id = ?")
                .bind(id.to_string())
                .fetch_one(&self.pool)
                .await?;
        let children: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM templates WHERE parent_template_id = ?")
                .bind(id.to_string())
                .fetch_one(&self.pool)
                .await?;
        Ok((environments, children))
    }

    pub async fn delete_template(&self, id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM templates WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn insert_operation(&self, operation: &Operation) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO operations
               (id, kind, phase, progress_percent, cancellable, resource_id, error_code,
                error_params_json, result_json, log_json, created_at, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(operation.id.to_string())
        .bind(serde_json::to_string(&operation.kind)?)
        .bind(serde_json::to_string(&operation.phase)?)
        .bind(operation.progress_percent.map(i64::from).unwrap_or(-1))
        .bind(operation.cancellable)
        .bind(operation.resource_id.map(|id| id.to_string()))
        .bind(
            operation
                .error
                .as_ref()
                .map(|error| serde_json::to_string(&error.code))
                .transpose()?,
        )
        .bind(
            operation
                .error
                .as_ref()
                .map(|error| serde_json::to_string(&error.params))
                .transpose()?,
        )
        .bind(
            operation
                .result
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?,
        )
        .bind(serde_json::to_string(&operation.log_lines)?)
        .bind(operation.created_at.to_rfc3339())
        .bind(operation.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn append_operation_log(&self, id: Uuid, line: &str) -> Result<()> {
        sqlx::query(
            r#"UPDATE operations
               SET log_json = CASE
                 WHEN json_array_length(log_json) >= 200
                   THEN json_insert(json_remove(log_json, '$[0]'), '$[#]', ?)
                 ELSE json_insert(log_json, '$[#]', ?)
               END,
               updated_at = ?
               WHERE id = ?"#,
        )
        .bind(line)
        .bind(line)
        .bind(Utc::now().to_rfc3339())
        .bind(id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_operation(
        &self,
        id: Uuid,
        phase: OperationPhase,
        progress_percent: Option<u8>,
        error: Option<&ApiError>,
        result: Option<&serde_json::Value>,
    ) -> Result<()> {
        sqlx::query(
            r#"UPDATE operations SET phase = ?, progress_percent = ?, error_code = ?,
               error_params_json = ?, result_json = ?, updated_at = ? WHERE id = ?"#,
        )
        .bind(serde_json::to_string(&phase)?)
        .bind(progress_percent.map(i64::from).unwrap_or(-1))
        .bind(
            error
                .map(|value| serde_json::to_string(&value.code))
                .transpose()?,
        )
        .bind(
            error
                .map(|value| serde_json::to_string(&value.params))
                .transpose()?,
        )
        .bind(result.map(serde_json::to_string).transpose()?)
        .bind(Utc::now().to_rfc3339())
        .bind(id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_operation(&self, id: Uuid) -> Result<Option<Operation>> {
        let row = sqlx::query("SELECT * FROM operations WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.map(parse_operation).transpose()
    }

    pub async fn list_unfinished_operations(&self) -> Result<Vec<Operation>> {
        let rows = sqlx::query("SELECT * FROM operations")
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(parse_operation)
            .filter(|operation| {
                operation.as_ref().is_ok_and(|operation| {
                    !matches!(
                        operation.phase,
                        OperationPhase::Succeeded
                            | OperationPhase::Failed
                            | OperationPhase::Cancelled
                            | OperationPhase::Retryable
                    )
                })
            })
            .collect()
    }

    pub async fn recover_unfinished_operations(&self) -> Result<usize> {
        let unfinished = self.list_unfinished_operations().await?;
        for operation in &unfinished {
            self.update_operation(
                operation.id,
                OperationPhase::Retryable,
                operation.progress_percent,
                None,
                None,
            )
            .await?;
        }
        Ok(unfinished.len())
    }
}

async fn ensure_column(
    pool: &SqlitePool,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<()> {
    let rows = sqlx::query(&format!("PRAGMA table_info({table})"))
        .fetch_all(pool)
        .await?;
    if !rows
        .iter()
        .any(|row| row.try_get::<String, _>("name").ok().as_deref() == Some(column))
    {
        sqlx::query(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {definition}"
        ))
        .execute(pool)
        .await?;
    }
    Ok(())
}

fn parse_environment(row: sqlx::sqlite::SqliteRow) -> Result<EnvironmentRecord> {
    let created_at = DateTime::parse_from_rfc3339(row.try_get("created_at")?)?.with_timezone(&Utc);
    Ok(EnvironmentRecord {
        id: Uuid::parse_str(row.try_get("id")?)?,
        name: row.try_get("name")?,
        container_id: row.try_get("container_id")?,
        config_path: row.try_get("config_path")?,
        desired_running: row.try_get("desired_running")?,
        local_port: row
            .try_get::<Option<i64>, _>("local_port")?
            .map(|value| value as u16),
        template_id: row
            .try_get::<Option<String>, _>("template_id")?
            .map(|value| Uuid::parse_str(&value))
            .transpose()?,
        spec: serde_json::from_str(row.try_get("spec_json")?)?,
        created_at,
    })
}

fn parse_operation(row: sqlx::sqlite::SqliteRow) -> Result<Operation> {
    let error_code = row.try_get::<Option<String>, _>("error_code")?;
    let error_params = row.try_get::<Option<String>, _>("error_params_json")?;
    let created_at = row
        .try_get::<Option<String>, _>("created_at")?
        .unwrap_or_else(|| {
            row.try_get::<String, _>("updated_at")
                .expect("updated_at exists")
        });
    let progress: i64 = row.try_get("progress_percent")?;
    Ok(Operation {
        id: Uuid::parse_str(row.try_get("id")?)?,
        kind: parse_enum(row.try_get("kind")?)?,
        phase: parse_enum(row.try_get("phase")?)?,
        progress_percent: (progress >= 0).then_some(progress as u8),
        cancellable: row.try_get("cancellable")?,
        resource_id: row
            .try_get::<Option<String>, _>("resource_id")?
            .map(|value| Uuid::parse_str(&value))
            .transpose()?,
        error: error_code
            .map(|code| {
                Ok::<ApiError, anyhow::Error>(ApiError {
                    code: serde_json::from_str(&code)?,
                    params: error_params
                        .as_deref()
                        .map(serde_json::from_str)
                        .transpose()?
                        .unwrap_or_default(),
                })
            })
            .transpose()?,
        result: row
            .try_get::<Option<String>, _>("result_json")?
            .map(|value| serde_json::from_str(&value))
            .transpose()?,
        log_lines: row
            .try_get::<Option<String>, _>("log_json")?
            .as_deref()
            .map(serde_json::from_str)
            .transpose()?
            .unwrap_or_default(),
        created_at: DateTime::parse_from_rfc3339(&created_at)?.with_timezone(&Utc),
        updated_at: DateTime::parse_from_rfc3339(row.try_get("updated_at")?)?.with_timezone(&Utc),
    })
}

fn parse_enum<T: serde::de::DeserializeOwned>(value: &str) -> Result<T> {
    if let Ok(parsed) = serde_json::from_str(value) {
        return Ok(parsed);
    }
    let quoted = serde_json::to_string(value)?;
    serde_json::from_str(&quoted).context("parse persisted enum")
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[tokio::test]
    async fn updates_an_environment_local_port_after_docker_reassigns_it() {
        let directory = tempdir().unwrap();
        let database = Database::open(&directory.path().join("controller.sqlite3"))
            .await
            .unwrap();
        let id = Uuid::new_v4();
        let mut spec = EnvironmentSpec::default();
        spec.name = "test desktop".into();
        let record = EnvironmentRecord {
            id,
            name: spec.name.clone(),
            container_id: "container-id".into(),
            config_path: directory.path().join("config").display().to_string(),
            desired_running: true,
            local_port: Some(32772),
            template_id: None,
            spec,
            created_at: Utc::now(),
        };
        database.insert_environment(&record).await.unwrap();

        database
            .set_environment_local_port(id, Some(32774))
            .await
            .unwrap();

        let refreshed = database.get_environment(id).await.unwrap().unwrap();
        assert_eq!(refreshed.local_port, Some(32774));
    }

    #[tokio::test]
    async fn interrupted_operations_become_retryable() {
        let directory = tempdir().unwrap();
        let database = Database::open(&directory.path().join("controller.sqlite3"))
            .await
            .unwrap();
        let now = Utc::now();
        let operation = Operation {
            id: Uuid::new_v4(),
            kind: OperationKind::CreateTemplate,
            phase: OperationPhase::Running,
            progress_percent: Some(45),
            cancellable: false,
            resource_id: Some(Uuid::new_v4()),
            error: None,
            result: None,
            log_lines: vec!["[controller] operation started".into()],
            created_at: now,
            updated_at: now,
        };
        database.insert_operation(&operation).await.unwrap();
        database
            .append_operation_log(operation.id, "[worker] snapshot complete")
            .await
            .unwrap();

        assert_eq!(database.recover_unfinished_operations().await.unwrap(), 1);
        let recovered = database.get_operation(operation.id).await.unwrap().unwrap();
        assert_eq!(recovered.phase, OperationPhase::Retryable);
        assert_eq!(recovered.progress_percent, Some(45));
        assert_eq!(
            recovered.log_lines,
            vec![
                "[controller] operation started".to_string(),
                "[worker] snapshot complete".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn operation_updates_wait_for_a_concurrent_sqlite_writer() {
        let directory = tempdir().unwrap();
        let database = Database::open(&directory.path().join("controller.sqlite3"))
            .await
            .unwrap();
        let now = Utc::now();
        let operation = Operation {
            id: Uuid::new_v4(),
            kind: OperationKind::ExportTemplate,
            phase: OperationPhase::RollingBack,
            progress_percent: Some(10),
            cancellable: true,
            resource_id: Some(Uuid::new_v4()),
            error: None,
            result: None,
            log_lines: vec![],
            created_at: now,
            updated_at: now,
        };
        database.insert_operation(&operation).await.unwrap();

        let mut writer = database.pool.acquire().await.unwrap();
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *writer)
            .await
            .unwrap();
        let updating_database = database.clone();
        let operation_id = operation.id;
        let update = tokio::spawn(async move {
            updating_database
                .update_operation(operation_id, OperationPhase::Cancelled, None, None, None)
                .await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        sqlx::query("COMMIT").execute(&mut *writer).await.unwrap();

        update.await.unwrap().unwrap();
        let updated = database.get_operation(operation.id).await.unwrap().unwrap();
        assert_eq!(updated.phase, OperationPhase::Cancelled);
    }

    #[tokio::test]
    async fn concurrent_operation_logs_are_not_lost() {
        let directory = tempdir().unwrap();
        let database = Database::open(&directory.path().join("controller.sqlite3"))
            .await
            .unwrap();
        let now = Utc::now();
        let operation = Operation {
            id: Uuid::new_v4(),
            kind: OperationKind::ExportTemplate,
            phase: OperationPhase::Running,
            progress_percent: Some(10),
            cancellable: true,
            resource_id: Some(Uuid::new_v4()),
            error: None,
            result: None,
            log_lines: vec![],
            created_at: now,
            updated_at: now,
        };
        database.insert_operation(&operation).await.unwrap();

        let mut appends = Vec::new();
        for index in 0..40 {
            let appending_database = database.clone();
            let operation_id = operation.id;
            appends.push(tokio::spawn(async move {
                let line = format!("line-{index}");
                appending_database
                    .append_operation_log(operation_id, &line)
                    .await
            }));
        }
        for append in appends {
            append.await.unwrap().unwrap();
        }

        let updated = database.get_operation(operation.id).await.unwrap().unwrap();
        assert_eq!(updated.log_lines.len(), 40);
        for index in 0..40 {
            assert!(updated.log_lines.contains(&format!("line-{index}")));
        }
    }

    #[tokio::test]
    async fn template_dependencies_count_derived_environments_and_children() {
        let directory = tempdir().unwrap();
        let database = Database::open(&directory.path().join("controller.sqlite3"))
            .await
            .unwrap();
        let parent = Uuid::new_v4();
        let child = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO templates (id, name, record_json, created_at) VALUES (?, ?, '{}', ?)",
        )
        .bind(parent.to_string())
        .bind("parent")
        .bind(Utc::now().to_rfc3339())
        .execute(&database.pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO templates (id, name, parent_template_id, record_json, created_at) VALUES (?, ?, ?, '{}', ?)")
            .bind(child.to_string()).bind("child").bind(parent.to_string()).bind(Utc::now().to_rfc3339())
            .execute(&database.pool).await.unwrap();
        let mut spec = EnvironmentSpec::default();
        spec.name = "derived".into();
        let environment = EnvironmentRecord {
            id: Uuid::new_v4(),
            name: spec.name.clone(),
            container_id: "derived-container".into(),
            config_path: directory.path().join("config").display().to_string(),
            desired_running: false,
            local_port: None,
            template_id: Some(parent),
            spec,
            created_at: Utc::now(),
        };
        database.insert_environment(&environment).await.unwrap();

        assert_eq!(
            database.template_dependency_counts(parent).await.unwrap(),
            (1, 1)
        );
    }
}
