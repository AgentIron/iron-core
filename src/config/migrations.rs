/// Embedded migration SQL for the config store schema.
///
/// Migrations are applied in order. Each migration is idempotent where possible.
pub const MIGRATIONS: &[(i64, &str)] = &[
    (
        1,
        r#"
            CREATE TABLE IF NOT EXISTS schema_version (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                version INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS profiles (
                id TEXT PRIMARY KEY,
                schema_version INTEGER NOT NULL,
                payload TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS prompts (
                id TEXT PRIMARY KEY,
                schema_version INTEGER NOT NULL,
                payload TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS schedule (
                id TEXT PRIMARY KEY,
                schema_version INTEGER NOT NULL,
                payload TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS credentials (
                provider_slug TEXT PRIMARY KEY,
                credential_mode TEXT NOT NULL,
                encrypted_payload BLOB NOT NULL,
                nonce BLOB NOT NULL,
                encryption_metadata TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            INSERT OR IGNORE INTO schema_version (id, version) VALUES (1, 1);
            "#,
    ),
    (
        2,
        r#"
            CREATE TABLE IF NOT EXISTS provider_configs (
                provider_slug TEXT PRIMARY KEY,
                display_name TEXT NOT NULL,
                enabled INTEGER NOT NULL,
                base_url TEXT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS custom_models (
                provider_slug TEXT NOT NULL,
                model_id TEXT NOT NULL,
                display_name TEXT NOT NULL,
                context_window INTEGER NULL,
                output_limit INTEGER NULL,
                supports_tool_calls INTEGER NOT NULL DEFAULT 0,
                supports_reasoning INTEGER NOT NULL DEFAULT 0,
                supports_vision INTEGER NOT NULL DEFAULT 0,
                cost_input_per_million REAL NULL,
                cost_output_per_million REAL NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (provider_slug, model_id)
            );

            CREATE TABLE IF NOT EXISTS runtime_defaults (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                provider_slug TEXT NOT NULL,
                model_id TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS mcp_servers (
                id TEXT PRIMARY KEY,
                label TEXT NOT NULL,
                description TEXT NULL,
                transport_kind TEXT NOT NULL,
                command TEXT NULL,
                args_json TEXT NULL,
                env_json TEXT NULL,
                inherited_env_vars_json TEXT NULL,
                url TEXT NULL,
                headers_json TEXT NULL,
                working_dir TEXT NULL,
                enabled_by_default INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS skill_settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                trust_project_skills INTEGER NOT NULL,
                additional_skill_dirs_json TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            INSERT OR IGNORE INTO schema_version (id, version) VALUES (1, 2);
            "#,
    ),
    (
        3,
        r#"
            ALTER TABLE custom_models ADD COLUMN supports_streaming INTEGER NOT NULL DEFAULT 1;
            ALTER TABLE custom_models ADD COLUMN reasoning_effort_values_json TEXT NULL;

            INSERT OR IGNORE INTO schema_version (id, version) VALUES (1, 3);
            "#,
    ),
    (
        4,
        r#"
            CREATE TABLE IF NOT EXISTS saved_handoffs (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                bundle_json TEXT NOT NULL,
                bundle_version TEXT NOT NULL,
                source_session_id TEXT NULL,
                source_model TEXT NULL,
                source_provider TEXT NULL,
                size_estimate_tokens INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            INSERT OR IGNORE INTO schema_version (id, version) VALUES (1, 4);
            "#,
    ),
    (
        5,
        r#"
            CREATE TABLE IF NOT EXISTS bootstrap_metadata (
                domain TEXT NOT NULL,
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (domain, key)
            );

            INSERT OR IGNORE INTO schema_version (id, version) VALUES (1, 5);
            "#,
    ),
    (
        6,
        r#"
            CREATE TABLE IF NOT EXISTS provider_profiles (
                slug TEXT PRIMARY KEY,
                profile_json TEXT NOT NULL,
                source TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            INSERT OR IGNORE INTO schema_version (id, version) VALUES (1, 6);
            "#,
    ),
    (
        7,
        r#"
            CREATE TABLE IF NOT EXISTS automation_tasks (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                stored_prompt_id TEXT NOT NULL,
                expected_outcome TEXT NOT NULL,
                schema_version INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY (stored_prompt_id) REFERENCES prompts(id) ON DELETE RESTRICT
            );

            CREATE INDEX IF NOT EXISTS idx_automation_tasks_stored_prompt_id
                ON automation_tasks (stored_prompt_id);

            INSERT OR IGNORE INTO schema_version (id, version) VALUES (1, 7);
            "#,
    ),
    (
        8,
        r#"
            ALTER TABLE automation_tasks ADD COLUMN normalized_name TEXT NOT NULL DEFAULT '';
            ALTER TABLE automation_tasks ADD COLUMN project_root TEXT NOT NULL DEFAULT '';
            ALTER TABLE automation_tasks ADD COLUMN timeout_seconds INTEGER NOT NULL DEFAULT 0;

            -- Backfill normalized_name from existing name. Legacy tasks keep
            -- schema_version 1 so they remain distinguishable from complete v2
            -- records with valid project_root and timeout_seconds.
            UPDATE automation_tasks
                SET normalized_name = lower(trim(name));

            INSERT OR IGNORE INTO schema_version (id, version) VALUES (1, 8);
            "#,
    ),
];

/// The current schema version.
#[allow(dead_code)]
pub const CURRENT_SCHEMA_VERSION: i64 = 8;

/// Apply all pending migrations to the database.
pub async fn apply_migrations(pool: &sqlx::SqlitePool) -> Result<(), super::error::ConfigError> {
    // Create schema_version table if it doesn't exist
    sqlx::query("CREATE TABLE IF NOT EXISTS schema_version (id INTEGER PRIMARY KEY CHECK (id = 1), version INTEGER NOT NULL)")
        .execute(pool)
        .await
        .map_err(|e| super::error::ConfigError::Migration(format!("Failed to initialize schema_version table: {}", e)))?;

    // Check current version
    let current_version: Option<i64> =
        sqlx::query_scalar("SELECT version FROM schema_version WHERE id = 1")
            .fetch_optional(pool)
            .await
            .map_err(|e| {
                super::error::ConfigError::Migration(format!(
                    "Failed to read schema version: {}",
                    e
                ))
            })?;

    let current_version = current_version.unwrap_or(0);

    // Apply pending migrations
    for (version, sql) in MIGRATIONS {
        if *version > current_version {
            let mut tx = pool
                .begin()
                .await
                .map_err(|e| super::error::ConfigError::Migration(e.to_string()))?;

            sqlx::raw_sql(*sql).execute(&mut *tx).await.map_err(|e| {
                super::error::ConfigError::Migration(format!("Migration {} failed: {}", version, e))
            })?;

            sqlx::query("INSERT INTO schema_version (id, version) VALUES (1, ?) ON CONFLICT(id) DO UPDATE SET version = excluded.version")
                .bind(*version)
                .execute(&mut *tx)
                .await
                .map_err(|e| {
                    super::error::ConfigError::Migration(format!(
                        "Failed to update schema version to {}: {}",
                        version, e
                    ))
                })?;

            tx.commit().await.map_err(|e| {
                super::error::ConfigError::Migration(format!(
                    "Failed to commit migration {}: {}",
                    version, e
                ))
            })?;
        }
    }

    Ok(())
}
