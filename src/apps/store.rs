use super::app::{AppType, ManagedApp};
use rusqlite::{params, Connection};
use std::path::PathBuf;
use std::sync::Mutex;

pub struct AppStore {
    conn: Mutex<Connection>,
}

impl AppStore {
    pub fn new() -> Result<Self, String> {
        let db_path = Self::db_path();
        if let Some(parent) = db_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let conn =
            Connection::open(&db_path).map_err(|e| format!("Failed to open apps.db: {}", e))?;

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

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn db_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("/root/.config"))
            .join("ivnc")
            .join("apps.db")
    }

    fn ensure_column(conn: &Connection, name: &str, ty: &str) -> Result<(), String> {
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
        Ok(())
    }

    pub fn add(&self, app: &ManagedApp) -> Result<(), String> {
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
            "INSERT INTO apps (id, name, app_type, url, autostart, exec_command, env_vars, created_at, launch_command, launch_env_vars, launch_cwd, launch_wait_timeout_secs, cli_binary_path, cli_env_vars)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                app.id, app.name, app.app_type.as_str(), url,
                app.autostart as i32, exec_command, env_vars_json, app.created_at,
                app.launch_command.as_deref(), launch_env_vars_json,
                app.launch_cwd.as_deref(), app.launch_wait_timeout_secs.map(|v| v as i64),
                app.cli_binary_path.as_deref(), cli_env_vars_json,
            ],
        ).map_err(|e| {
            if e.to_string().contains("UNIQUE") {
                format!("App name '{}' already exists", app.name)
            } else {
                format!("Failed to add app: {}", e)
            }
        })?;
        Ok(())
    }

    pub fn update(&self, app: &ManagedApp) -> Result<(), String> {
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

        let changed = conn.execute(
            "UPDATE apps SET app_type=?1, url=?2, autostart=?3, exec_command=?4, env_vars=?5, launch_command=?6, launch_env_vars=?7, launch_cwd=?8, launch_wait_timeout_secs=?9, cli_binary_path=?10, cli_env_vars=?11 WHERE id=?12",
            params![app.app_type.as_str(), url, app.autostart as i32, exec_command, env_vars_json, app.launch_command.as_deref(), launch_env_vars_json, app.launch_cwd.as_deref(), app.launch_wait_timeout_secs.map(|v| v as i64), app.cli_binary_path.as_deref(), cli_env_vars_json, app.id],
        ).map_err(|e| format!("Failed to update app: {}", e))?;
        if changed == 0 {
            return Err(format!("App '{}' not found", app.id));
        }
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        let changed = conn
            .execute("DELETE FROM apps WHERE id=?1", params![id])
            .map_err(|e| format!("Failed to delete app: {}", e))?;
        if changed == 0 {
            return Err(format!("App '{}' not found", id));
        }
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<ManagedApp, String> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, name, app_type, url, autostart, exec_command, env_vars, created_at, launch_command, launch_env_vars, launch_cwd, launch_wait_timeout_secs, cli_binary_path, cli_env_vars FROM apps WHERE id=?1",
            params![id],
            |row| Ok(Self::row_to_app(row)),
        ).map_err(|e| format!("App not found: {}", e))
    }

    pub fn list(&self) -> Result<Vec<ManagedApp>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, app_type, url, autostart, exec_command, env_vars, created_at, launch_command, launch_env_vars, launch_cwd, launch_wait_timeout_secs, cli_binary_path, cli_env_vars FROM apps ORDER BY created_at"
        ).map_err(|e| format!("Failed to list apps: {}", e))?;

        let apps = stmt
            .query_map([], |row| Ok(Self::row_to_app(row)))
            .map_err(|e| format!("Failed to query apps: {}", e))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(apps)
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
        }
    }
}
