use super::app::{AppType, ManagedApp};
use super::error::ApiError;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct AppStore {
    conn: Mutex<Connection>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct AppOrder {
    pub ordered_ids: Vec<String>,
    pub order_revision: u64,
}

#[derive(Default)]
pub struct OrderedApps {
    pub apps: Vec<ManagedApp>,
    pub order: AppOrder,
}

#[derive(Debug, Serialize)]
pub struct AppName {
    pub id: String,
    pub name: String,
    pub name_revision: u64,
}

pub fn validate_app_name(name: &str) -> Result<String, ApiError> {
    if name
        .chars()
        .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
    {
        return Err(ApiError::new(
            "app_name_invalid",
            "Application name cannot contain control characters or line breaks",
        ));
    }
    let name = name.trim();
    if name.is_empty() {
        return Err(ApiError::new(
            "app_name_required",
            "Enter an application name",
        ));
    }
    if name.len() > 256 {
        return Err(ApiError::new(
            "app_name_too_long",
            "Application name exceeds 256 UTF-8 bytes",
        ));
    }
    Ok(name.to_owned())
}

impl AppStore {
    pub fn new() -> Result<Self, String> {
        let db_path = Self::db_path();
        if let Some(parent) = db_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let conn =
            Connection::open(&db_path).map_err(|e| format!("Failed to open apps.db: {}", e))?;

        Self::from_connection(conn)
    }

    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self::from_connection(Connection::open_in_memory().unwrap()).unwrap()
    }

    fn from_connection(mut conn: Connection) -> Result<Self, String> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS apps (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                url TEXT,
                autostart INTEGER DEFAULT 0,
                created_at TEXT NOT NULL,
                app_type TEXT DEFAULT 'background',
                exec_command TEXT,
                env_vars TEXT,
                launch_command TEXT,
                launch_env_vars TEXT,
                launch_cwd TEXT,
                launch_wait_timeout_secs INTEGER,
                cli_binary_path TEXT,
                cli_env_vars TEXT
            );",
        )
        .map_err(|e| format!("Failed to init db: {}", e))?;
        Self::ensure_column(&conn, "cli_binary_path", "TEXT")?;
        Self::ensure_column(&conn, "cli_env_vars", "TEXT")?;
        Self::ensure_column(&conn, "name_revision", "INTEGER NOT NULL DEFAULT 0")?;

        // Adding the column and backfilling are one migration. Later opens must
        // never reset positions saved by the user (including timestamp ties).
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        if Self::ensure_column(&tx, "sort_order", "INTEGER NOT NULL DEFAULT 0")? {
            let ids = {
                let mut stmt = tx
                    .prepare("SELECT id FROM apps ORDER BY created_at, rowid")
                    .map_err(|e| e.to_string())?;
                let rows = stmt
                    .query_map([], |r| r.get::<_, String>(0))
                    .map_err(|e| e.to_string())?;
                rows.collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())?
            };
            for (position, id) in ids.iter().enumerate() {
                tx.execute(
                    "UPDATE apps SET sort_order=?1 WHERE id=?2",
                    params![position as i64, id],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS app_order_state (
                id INTEGER PRIMARY KEY CHECK(id=1),
                revision INTEGER NOT NULL DEFAULT 0
            );
            INSERT OR IGNORE INTO app_order_state(id, revision) VALUES(1, 0);
            CREATE TRIGGER IF NOT EXISTS apps_order_insert AFTER INSERT ON apps BEGIN
                UPDATE app_order_state SET revision=revision+1 WHERE id=1;
            END;
            CREATE TRIGGER IF NOT EXISTS apps_order_delete AFTER DELETE ON apps BEGIN
                UPDATE app_order_state SET revision=revision+1 WHERE id=1;
            END;",
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn db_path() -> PathBuf {
        crate::paths::apps_db()
    }

    fn ensure_column(conn: &Connection, name: &str, ty: &str) -> Result<bool, String> {
        let mut stmt = conn
            .prepare("PRAGMA table_info(apps)")
            .map_err(|e| format!("Failed to inspect apps table: {}", e))?;
        let exists = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| format!("Failed to read apps table info: {}", e))?
            .filter_map(Result::ok)
            .any(|column| column == name);
        if !exists {
            conn.execute(&format!("ALTER TABLE apps ADD COLUMN {} {}", name, ty), [])
                .map_err(|e| format!("Failed to add apps.{} column: {}", name, e))?;
        }
        Ok(!exists)
    }

    pub fn add(&self, app: &ManagedApp) -> Result<(), ApiError> {
        let conn = self.conn.lock().unwrap();

        let url = app.url.as_deref().unwrap_or("");
        let exec_command = app.exec_command.as_deref().unwrap_or("");
        let env_vars_json = app
            .env_vars
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok())
            .unwrap_or_default();
        let launch_env_vars_json = app
            .launch_env_vars
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok())
            .unwrap_or_default();
        let cli_env_vars_json = app
            .cli_env_vars
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok())
            .unwrap_or_default();

        conn.execute(
            "INSERT INTO apps (id, name, app_type, url, autostart, exec_command, env_vars, created_at, launch_command, launch_env_vars, launch_cwd, launch_wait_timeout_secs, cli_binary_path, cli_env_vars, sort_order)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, (SELECT COALESCE(MAX(sort_order), -1)+1 FROM apps))",
            params![
                app.id, app.name, app.app_type.as_str(), url,
                app.autostart as i32, exec_command, env_vars_json, app.created_at,
                app.launch_command.as_deref(), launch_env_vars_json,
                app.launch_cwd.as_deref(), app.launch_wait_timeout_secs.map(|v| v as i64),
                app.cli_binary_path.as_deref(), cli_env_vars_json,
            ],
        ).map_err(|e| {
            if e.to_string().contains("UNIQUE") {
                ApiError::new(
                    "app_name_exists",
                    format!("App name '{}' already exists", app.name),
                )
                .param("name", app.name.as_str())
            } else {
                format!("Failed to add app: {}", e).into()
            }
        })?;
        Ok(())
    }

    pub fn update(&self, app: &ManagedApp) -> Result<(), ApiError> {
        self.update_with_name(app, None)
    }

    /// Configuration writes never restore a name from an earlier read. The
    /// legacy remote edit API can explicitly rename in this same transaction.
    pub fn update_with_name(
        &self,
        app: &ManagedApp,
        name: Option<(&str, u64)>,
    ) -> Result<(), ApiError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        if let Some((name, revision)) = name {
            Self::rename_in(&tx, &app.id, name, revision)?;
        }

        let url = app.url.as_deref().unwrap_or("");
        let exec_command = app.exec_command.as_deref().unwrap_or("");
        let env_vars_json = app
            .env_vars
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok())
            .unwrap_or_default();
        let launch_env_vars_json = app
            .launch_env_vars
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok())
            .unwrap_or_default();
        let cli_env_vars_json = app
            .cli_env_vars
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok())
            .unwrap_or_default();

        let changed = tx.execute(
            "UPDATE apps SET app_type=?1, url=?2, autostart=?3, exec_command=?4, env_vars=?5, launch_command=?6, launch_env_vars=?7, launch_cwd=?8, launch_wait_timeout_secs=?9, cli_binary_path=?10, cli_env_vars=?11 WHERE id=?12",
            params![app.app_type.as_str(), url, app.autostart as i32, exec_command, env_vars_json, app.launch_command.as_deref(), launch_env_vars_json, app.launch_cwd.as_deref(), app.launch_wait_timeout_secs.map(|v| v as i64), app.cli_binary_path.as_deref(), cli_env_vars_json, app.id],
        ).map_err(|e| format!("Failed to update app: {}", e))?;
        if changed == 0 {
            return Err(ApiError::new(
                "app_not_found",
                format!("App '{}' not found", app.id),
            ));
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn rename(&self, id: &str, name: &str, base_revision: u64) -> Result<AppName, ApiError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let renamed = Self::rename_in(&tx, id, name, base_revision)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(renamed)
    }

    fn rename_in(
        conn: &Connection,
        id: &str,
        name: &str,
        base_revision: u64,
    ) -> Result<AppName, ApiError> {
        let name = validate_app_name(name)?;
        let current = conn
            .query_row(
                "SELECT name, name_revision FROM apps WHERE id=?1",
                [id],
                |row| {
                    Ok(AppName {
                        id: id.to_owned(),
                        name: row.get(0)?,
                        name_revision: row.get(1)?,
                    })
                },
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| ApiError::new("app_not_found", "Application no longer exists"))?;
        if current.name_revision != base_revision {
            return Err(ApiError::new(
                "app_name_conflict",
                "Application name changed; review the latest name and retry",
            )
            .param("name", current.name)
            .param("name_revision", current.name_revision));
        }
        if current.name == name {
            return Ok(current);
        }
        conn.execute(
            "UPDATE apps SET name=?1, name_revision=name_revision+1 WHERE id=?2",
            params![name, id],
        )
        .map_err(|e| {
            if e.to_string().contains("UNIQUE") {
                ApiError::new("app_name_exists", "Application name already exists")
                    .param("name", name.as_str())
            } else {
                ApiError::from(e.to_string())
            }
        })?;
        Ok(AppName {
            id: id.to_owned(),
            name,
            name_revision: current.name_revision + 1,
        })
    }

    pub fn delete(&self, id: &str) -> Result<(), ApiError> {
        let conn = self.conn.lock().unwrap();
        let changed = conn
            .execute("DELETE FROM apps WHERE id=?1", params![id])
            .map_err(|e| format!("Failed to delete app: {}", e))?;
        if changed == 0 {
            return Err(ApiError::new(
                "app_not_found",
                format!("App '{}' not found", id),
            ));
        }
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<ManagedApp, ApiError> {
        self.get_optional(id)?
            .ok_or_else(|| ApiError::new("app_not_found", format!("App not found: {}", id)))
    }

    pub fn get_optional(&self, id: &str) -> Result<Option<ManagedApp>, String> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, name, app_type, url, autostart, exec_command, env_vars, created_at, launch_command, launch_env_vars, launch_cwd, launch_wait_timeout_secs, cli_binary_path, cli_env_vars, sort_order, name_revision FROM apps WHERE id=?1",
            params![id],
            |row| Ok(Self::row_to_app(row)),
        ).optional().map_err(|e| format!("Failed to read app: {}", e))
    }

    pub fn list(&self) -> Result<Vec<ManagedApp>, String> {
        let conn = self.conn.lock().unwrap();
        Self::read_apps(&conn, "created_at, rowid")
    }

    fn read_apps(conn: &Connection, order: &str) -> Result<Vec<ManagedApp>, String> {
        let mut stmt = conn.prepare(&format!(
            "SELECT id, name, app_type, url, autostart, exec_command, env_vars, created_at, launch_command, launch_env_vars, launch_cwd, launch_wait_timeout_secs, cli_binary_path, cli_env_vars, sort_order, name_revision FROM apps ORDER BY {order}"
        )).map_err(|e| format!("Failed to list apps: {}", e))?;

        let apps = stmt
            .query_map([], |row| Ok(Self::row_to_app(row)))
            .map_err(|e| format!("Failed to query apps: {}", e))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(apps)
    }

    fn read_order(conn: &Connection) -> Result<AppOrder, String> {
        let order_revision = conn
            .query_row("SELECT revision FROM app_order_state WHERE id=1", [], |r| {
                r.get(0)
            })
            .map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT id FROM apps ORDER BY sort_order, created_at, id")
            .map_err(|e| e.to_string())?;
        let ordered_ids = stmt
            .query_map([], |r| r.get(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<String>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(AppOrder {
            ordered_ids,
            order_revision,
        })
    }

    /// The rows and revision must describe the same committed database state.
    pub fn list_ordered(&self) -> Result<OrderedApps, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let order = Self::read_order(&tx)?;
        let apps = Self::read_apps(&tx, "sort_order, created_at, id")?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(OrderedApps { apps, order })
    }

    pub fn order(&self) -> Result<AppOrder, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let order = Self::read_order(&tx)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(order)
    }

    /// Permutes only the submitted apps' slots, preserving all hidden apps.
    pub fn reorder(&self, ids: &[String], base_revision: u64) -> Result<AppOrder, ApiError> {
        let selected: HashSet<_> = ids.iter().collect();
        if ids.is_empty() || selected.len() != ids.len() {
            return Err(ApiError::new(
                "app_order_invalid",
                "Application order must contain distinct application IDs",
            ));
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let mut order = Self::read_order(&tx)?;
        if base_revision != order.order_revision {
            return Err(ApiError::new(
                "app_order_conflict",
                "Application order changed; refreshed to the latest order",
            ));
        }
        let existing: HashSet<_> = order.ordered_ids.iter().collect();
        if ids.iter().any(|id| !existing.contains(id)) {
            return Err(ApiError::new(
                "app_order_invalid",
                "Application order contains an unknown application ID",
            ));
        }
        let previous = order.ordered_ids.clone();
        let mut replacements = ids.iter();
        for id in &mut order.ordered_ids {
            if selected.contains(id) {
                *id = replacements.next().unwrap().clone();
            }
        }
        if previous != order.ordered_ids {
            for (position, id) in order.ordered_ids.iter().enumerate() {
                tx.execute(
                    "UPDATE apps SET sort_order=?1 WHERE id=?2",
                    params![position as i64, id],
                )
                .map_err(|e| e.to_string())?;
            }
            tx.execute(
                "UPDATE app_order_state SET revision=revision+1 WHERE id=1",
                [],
            )
            .map_err(|e| e.to_string())?;
            order.order_revision += 1;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(order)
    }

    fn row_to_app(row: &rusqlite::Row) -> ManagedApp {
        let app_type_str: String = row.get(2).unwrap_or_else(|_| "background".to_string());
        let app_type = AppType::from_str(&app_type_str).unwrap_or(AppType::BackgroundApp);

        let url: Option<String> = row.get(3).ok().filter(|s: &String| !s.is_empty());
        let autostart = row.get::<_, i32>(4).unwrap_or(0) != 0;

        let exec_command: Option<String> = row.get(5).ok().filter(|s: &String| !s.is_empty());
        let env_vars_json: Option<String> = row.get(6).ok().filter(|s: &String| !s.is_empty());
        let env_vars = env_vars_json.and_then(|json| serde_json::from_str(&json).ok());
        let launch_command: Option<String> = row.get(8).ok().filter(|s: &String| !s.is_empty());
        let launch_env_vars_json: Option<String> =
            row.get(9).ok().filter(|s: &String| !s.is_empty());
        let launch_env_vars =
            launch_env_vars_json.and_then(|json| serde_json::from_str(&json).ok());
        let launch_cwd: Option<String> = row.get(10).ok().filter(|s: &String| !s.is_empty());
        let launch_wait_timeout_secs: Option<u64> = row
            .get::<_, Option<i64>>(11)
            .unwrap_or(None)
            .and_then(|v| u64::try_from(v).ok());
        let cli_binary_path: Option<String> = row.get(12).ok().filter(|s: &String| !s.is_empty());
        let cli_env_vars_json: Option<String> = row.get(13).ok().filter(|s: &String| !s.is_empty());
        let cli_env_vars = cli_env_vars_json.and_then(|json| serde_json::from_str(&json).ok());

        ManagedApp {
            id: row.get(0).unwrap_or_default(),
            name: row.get(1).unwrap_or_default(),
            name_revision: row.get(15).unwrap_or_default(),
            app_type,
            autostart,
            url,
            launch_command,
            launch_env_vars,
            launch_cwd,
            launch_wait_timeout_secs,
            exec_command,
            env_vars,
            cli_binary_path,
            cli_env_vars,
            created_at: row.get(7).unwrap_or_default(),
            sort_order: row.get(14).unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(store: &AppStore, ids: &[&str]) {
        for id in ids {
            let mut app = ManagedApp::for_test(id, AppType::DesktopApp);
            app.name = id.to_string();
            app.created_at = "same timestamp".into();
            store.add(&app).unwrap();
        }
    }

    fn ids(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn subset_reorder_preserves_hidden_slots_and_launch_order() {
        let store = AppStore::in_memory();
        seed(&store, &["a", "background", "b", "cli", "c"]);
        let order = store.reorder(&ids(&["c", "a", "b"]), 5).unwrap();
        assert_eq!(
            order.ordered_ids,
            ids(&["c", "background", "a", "cli", "b"])
        );
        assert_eq!(order.order_revision, 6);
        let listed = store.list_ordered().unwrap();
        assert_eq!(
            listed.apps.iter().map(|a| a.id.clone()).collect::<Vec<_>>(),
            order.ordered_ids
        );
        assert_eq!(
            listed.apps.iter().map(|a| a.sort_order).collect::<Vec<_>>(),
            vec![0, 1, 2, 3, 4]
        );
        assert_eq!(
            store
                .list()
                .unwrap()
                .iter()
                .map(|a| a.id.clone())
                .collect::<Vec<_>>(),
            ids(&["a", "background", "b", "cli", "c"])
        );
        assert_eq!(store.reorder(&ids(&["c", "a", "b"]), 6).unwrap(), order);
    }

    #[test]
    fn invalid_or_stale_order_is_atomic_and_does_not_erase_new_apps() {
        let store = AppStore::in_memory();
        seed(&store, &["a", "b"]);
        let initial = store.order().unwrap();
        for invalid in [ids(&[]), ids(&["a", "a"]), ids(&["b", "missing"])] {
            assert_eq!(
                store
                    .reorder(&invalid, initial.order_revision)
                    .unwrap_err()
                    .code,
                "app_order_invalid"
            );
            assert_eq!(store.order().unwrap(), initial);
        }
        seed(&store, &["c"]);
        assert_eq!(
            store
                .reorder(&ids(&["b", "a"]), initial.order_revision)
                .unwrap_err()
                .code,
            "app_order_conflict"
        );
        assert_eq!(store.order().unwrap().ordered_ids, ids(&["a", "b", "c"]));
        store.delete("b").unwrap();
        assert_eq!(
            store.reorder(&ids(&["c", "a"]), 3).unwrap_err().code,
            "app_order_conflict"
        );
    }

    #[test]
    fn reopening_and_editing_preserve_positions_and_new_apps_append() {
        let store = AppStore::in_memory();
        seed(&store, &["a", "b"]);
        let order = store.reorder(&ids(&["b", "a"]), 2).unwrap();
        let store = AppStore::from_connection(store.conn.into_inner().unwrap()).unwrap();
        assert_eq!(store.order().unwrap(), order);
        let mut app = store.get("a").unwrap();
        app.name = "renamed".into();
        app.sort_order = 999; // Only the reorder operation may change position.
        store.update(&app).unwrap();
        assert_eq!(store.get("a").unwrap().sort_order, 1);
        assert_eq!(store.order().unwrap(), order);
        seed(&store, &["c"]);
        assert_eq!(store.get("c").unwrap().sort_order, 2);
        assert_eq!(store.order().unwrap().ordered_ids, ids(&["b", "a", "c"]));
    }

    #[test]
    fn legacy_migration_preserves_creation_order_with_timestamp_ties() {
        let store = AppStore::in_memory();
        seed(&store, &["z", "a", "b"]);
        let conn = store.conn.into_inner().unwrap();
        conn.execute_batch(
            "DROP TRIGGER apps_order_insert; DROP TRIGGER apps_order_delete;
            DROP TABLE app_order_state; ALTER TABLE apps DROP COLUMN sort_order;",
        )
        .unwrap();
        let migrated = AppStore::from_connection(conn).unwrap();
        assert_eq!(migrated.order().unwrap().ordered_ids, ids(&["z", "a", "b"]));
        assert_eq!(migrated.order().unwrap().order_revision, 0);
        assert_eq!(migrated.get("a").unwrap().sort_order, 1);
    }

    #[test]
    fn names_use_independent_cas_and_stale_configuration_cannot_overwrite_them() {
        let store = AppStore::in_memory();
        seed(&store, &["a", "b"]);
        let mut stale = store.get("a").unwrap();
        let order = store.order().unwrap();
        let renamed = store.rename("a", "  中文 app  ", 0).unwrap();
        assert_eq!(renamed.name, "中文 app");
        assert_eq!(renamed.name_revision, 1);
        assert_eq!(store.rename("a", "中文 app", 1).unwrap().name_revision, 1);
        assert_eq!(
            store.rename("a", "old", 0).unwrap_err().code,
            "app_name_conflict"
        );
        assert_eq!(
            store.rename("a", "b", 1).unwrap_err().code,
            "app_name_exists"
        );
        stale.exec_command = Some("new command".into());
        stale.name = "stale name".into();
        store.update(&stale).unwrap();
        assert_eq!(store.get("a").unwrap().name, "中文 app");
        assert_eq!(store.get("a").unwrap().exec_command, stale.exec_command);
        stale.exec_command = Some("must roll back".into());
        assert_eq!(
            store
                .update_with_name(&stale, Some(("changed", 0)))
                .unwrap_err()
                .code,
            "app_name_conflict"
        );
        assert_ne!(store.get("a").unwrap().exec_command, stale.exec_command);
        assert_eq!(store.order().unwrap(), order);
        let reopened = AppStore::from_connection(store.conn.into_inner().unwrap()).unwrap();
        assert_eq!(reopened.get("a").unwrap().name_revision, 1);
        assert_eq!(reopened.get("a").unwrap().name, "中文 app");
    }

    #[test]
    fn names_validate_utf8_bytes_and_control_characters_without_mutation() {
        let store = AppStore::in_memory();
        seed(&store, &["a"]);
        for name in [
            "",
            "   ",
            "a\nb",
            "\na",
            "a\rb",
            "a\tb",
            "a\0b",
            "a\u{7f}",
            "a\u{85}",
            "a\u{2028}",
            &"中".repeat(86),
        ] {
            assert!(store.rename("a", name, 0).is_err(), "accepted {name:?}");
            assert_eq!(store.get("a").unwrap().name_revision, 0);
        }
        let max = format!("{}a", "中".repeat(85));
        assert_eq!(store.rename("a", &max, 0).unwrap().name, max);
        assert_eq!(
            store.rename("missing", "valid", 0).unwrap_err().code,
            "app_not_found"
        );
        let conn = store.conn.into_inner().unwrap();
        conn.execute_batch("ALTER TABLE apps DROP COLUMN name_revision;")
            .unwrap();
        let migrated = AppStore::from_connection(conn).unwrap();
        assert_eq!(migrated.get("a").unwrap().name, max);
        assert_eq!(migrated.get("a").unwrap().name_revision, 0);
    }
}
