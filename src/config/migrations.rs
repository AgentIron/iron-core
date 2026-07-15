/// Embedded migration SQL for the config store schema.
///
/// Migrations are applied in order. Each migration is idempotent where possible.
use sha2::{Digest, Sha256};

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
    (
        9,
        r#"
            ALTER TABLE prompts ADD COLUMN display_name TEXT NOT NULL DEFAULT '';
            ALTER TABLE prompts ADD COLUMN normalized_name TEXT NOT NULL DEFAULT '';
            ALTER TABLE prompts ADD COLUMN identity_state TEXT NOT NULL DEFAULT 'ready';

            UPDATE prompts SET
                display_name = id,
                normalized_name = lower(trim(id));

            UPDATE prompts SET
                identity_state = 'needs_rename'
            WHERE normalized_name IN (
                SELECT normalized_name FROM prompts
                GROUP BY normalized_name
                HAVING COUNT(*) > 1
            );

            -- Repair handles are assigned by the post-migration Rust
            -- canonicalization pass (see apply_migrations) so that they are
            -- deterministic stable-id hashes rather than raw IDs.

            -- The unique index is created AFTER the post-migration Rust
            -- canonicalization pass (see apply_migrations) so that repair
            -- handles are guaranteed unique before index creation.

            INSERT OR IGNORE INTO schema_version (id, version) VALUES (1, 9);
            "#,
    ),
];

/// The current schema version.
#[allow(dead_code)]
pub const CURRENT_SCHEMA_VERSION: i64 = 10;

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

    // Refuse to migrate databases written by a newer binary. An older binary
    // must not mutate identity data or create indexes against schemas it does
    // not understand.
    if current_version > CURRENT_SCHEMA_VERSION {
        return Err(super::error::ConfigError::Migration(format!(
            "Database schema version {} is newer than the supported version {}; \
             use a newer binary or restore from a backup made before the upgrade",
            current_version, CURRENT_SCHEMA_VERSION
        )));
    }

    // Apply pending SQL migrations (versions 1-9).
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

    // Canonicalize prompt identity columns and create the unique handle index
    // in a single transaction. This runs every startup because the unique
    // index acts as a guard, and canonicalization repairs any records that
    // bypassed it. The schema version is bumped to 10 only when migrating from
    // an older version.
    let mut tx = pool.begin().await.map_err(|e| {
        super::error::ConfigError::Migration(format!(
            "Failed to begin prompt identity transaction: {}",
            e
        ))
    })?;

    #[allow(clippy::explicit_auto_deref)]
    canonicalize_prompt_identity(&mut *tx).await?;

    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_prompts_normalized_name ON prompts (normalized_name)",
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        super::error::ConfigError::Migration(format!(
            "Failed to create prompt handle index: {}",
            e
        ))
    })?;

    if current_version < 10 {
        sqlx::query("INSERT OR IGNORE INTO schema_version (id, version) VALUES (1, ?) ON CONFLICT(id) DO UPDATE SET version = excluded.version")
            .bind(10i64)
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                super::error::ConfigError::Migration(format!(
                    "Failed to update schema version to 10: {}",
                    e
                ))
            })?;
    }

    tx.commit().await.map_err(|e| {
        super::error::ConfigError::Migration(format!(
            "Failed to commit prompt identity transaction: {}",
            e
        ))
    })?;

    Ok(())
}

/// After SQL migrations, canonicalize prompt identity columns using the Rust
/// normalization function. This fixes values that the approximate SQL
/// backfill could not handle correctly (underscores, special characters,
/// multi-space collapsing).
///
/// Every record whose canonical handle collides with another record's
/// canonical handle is marked `needs_rename` with a deterministic repair
/// handle, so no collision participant retains a normal user-facing handle.
async fn canonicalize_prompt_identity(
    conn: &mut sqlx::SqliteConnection,
) -> Result<(), super::error::ConfigError> {
    use crate::stored_prompt::normalize_prompt_name;
    use sqlx::Row;
    use std::collections::HashMap;

    // Only proceed if the prompts table has the normalized_name column.
    let has_column: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('prompts') WHERE name = 'normalized_name'",
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(|e| super::error::ConfigError::Migration(e.to_string()))?;

    if has_column == 0 {
        return Ok(());
    }

    // Load all prompt identity values.
    let rows = sqlx::query("SELECT id, display_name, normalized_name, identity_state FROM prompts")
        .fetch_all(&mut *conn)
        .await
        .map_err(super::error::ConfigError::from)?;

    // Group records by canonical handle computed from display_name. Records
    // that have already been quarantined with a deterministic repair handle
    // (needs_rename + legacy-<hash>) are excluded from collision detection
    // because their display_name is stale (set by migration, not by the user).
    // Including them would cause a user-renamed collision participant to be
    // re-quarantined on every subsequent startup.
    let mut groups: HashMap<String, Vec<(String, String, String)>> = HashMap::new();
    for row in &rows {
        let id: String = row.get("id");
        let display_name: String = row.get("display_name");
        let stored: String = row.get("normalized_name");
        let state: String = row.get("identity_state");

        let is_quarantined = state == "needs_rename"
            && stored.starts_with("legacy-")
            && stored == legacy_repair_handle(&id);

        if is_quarantined {
            continue;
        }

        let canonical = normalize_prompt_name(&display_name);
        groups
            .entry(canonical)
            .or_default()
            .push((id, stored, state));
    }

    for (canonical, members) in &groups {
        if canonical.is_empty() {
            continue;
        }
        if members.len() > 1 {
            // Collision among active records: every member receives a
            // deterministic repair handle and is marked needs_rename.
            for (id, _stored, _state) in members {
                let repair = unique_repair_handle(conn, id).await?;
                sqlx::query(
                    "UPDATE prompts SET normalized_name = ?, identity_state = 'needs_rename' WHERE id = ?",
                )
                .bind(&repair)
                .bind(id)
                .execute(&mut *conn)
                .await
                .map_err(super::error::ConfigError::from)?;
            }
        } else {
            // Single owner: update ready records to the canonical handle.
            let (id, stored, state) = &members[0];
            if state == "ready" && canonical != stored {
                sqlx::query("UPDATE prompts SET normalized_name = ? WHERE id = ?")
                    .bind(canonical)
                    .bind(id)
                    .execute(&mut *conn)
                    .await
                    .map_err(super::error::ConfigError::from)?;
            }
        }
    }

    Ok(())
}

/// Generate a deterministic repair handle for a colliding prompt ID, ensuring
/// it does not collide with any existing `normalized_name` value.
///
/// The base form is `legacy-<stable-id-hash>`, derived from a SHA-256 hash
/// of the immutable record ID. If that value is already owned by a different
/// record (extraordinarily unlikely), a counter suffix is appended until unique.
async fn unique_repair_handle(
    conn: &mut sqlx::SqliteConnection,
    id: &str,
) -> Result<String, super::error::ConfigError> {
    let base = legacy_repair_handle(id);
    let mut handle = base.clone();
    let mut counter = 0u32;
    loop {
        let exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM prompts WHERE normalized_name = ? AND id != ?",
        )
        .bind(&handle)
        .bind(id)
        .fetch_one(&mut *conn)
        .await
        .map_err(super::error::ConfigError::from)?;
        if exists == 0 {
            return Ok(handle);
        }
        counter += 1;
        handle = format!("{}-{}", base, counter);
    }
}

/// Deterministic reserved repair handle derived from a stable record ID.
fn legacy_repair_handle(id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(id.as_bytes());
    let hash = hasher.finalize();
    let hex: String = hash.iter().map(|b| format!("{:02x}", b)).collect();
    format!("legacy-{}", &hex[..16])
}
