use rusqlite::{Connection, params};
use std::path::PathBuf;
use std::sync::Mutex;
use super::app::{PakeApp, AppMode, AppType};

pub struct AppStore {
    conn: Mutex<Connection>,
}

impl AppStore {
    pub fn new() -> Result<Self, String> {
        let db_path = Self::db_path();
        if let Some(parent) = db_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let conn = Connection::open(&db_path)
            .map_err(|e| format!("Failed to open apps.db: {}", e))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS apps (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                url TEXT,
                mode TEXT,
                dark_mode INTEGER DEFAULT 0,
                autostart INTEGER DEFAULT 0,
                show_nav INTEGER DEFAULT 0,
                created_at TEXT NOT NULL,
                app_type TEXT DEFAULT 'webapp',
                exec_command TEXT,
                env_vars TEXT
            );"
        ).map_err(|e| format!("Failed to init db: {}", e))?;

        // Migrations
        let _ = conn.execute("ALTER TABLE apps ADD COLUMN show_nav INTEGER DEFAULT 0", []);
        let _ = conn.execute("ALTER TABLE apps ADD COLUMN app_type TEXT DEFAULT 'webapp'", []);
        let _ = conn.execute("ALTER TABLE apps ADD COLUMN exec_command TEXT", []);
        let _ = conn.execute("ALTER TABLE apps ADD COLUMN env_vars TEXT", []);
        let _ = conn.execute("ALTER TABLE apps ADD COLUMN remote_debugging_port INTEGER", []);

        Ok(Self { conn: Mutex::new(conn) })
    }

    fn db_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("/root/.config"))
            .join("ivnc")
            .join("apps.db")
    }

    pub fn add(&self, app: &PakeApp) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();

        let url = app.url.as_deref().unwrap_or("");
        let mode = app.mode.map(|m| m.as_str()).unwrap_or("");
        let exec_command = app.exec_command.as_deref().unwrap_or("");
        let env_vars_json = app.env_vars.as_ref()
            .and_then(|v| serde_json::to_string(v).ok())
            .unwrap_or_default();

        conn.execute(
            "INSERT INTO apps (id, name, app_type, url, mode, show_nav, exec_command, env_vars, created_at, remote_debugging_port)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                app.id, app.name, app.app_type.as_str(), url, mode,
                app.show_nav as i32, exec_command, env_vars_json, app.created_at,
                app.remote_debugging_port,
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

    pub fn update(&self, app: &PakeApp) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();

        let url = app.url.as_deref().unwrap_or("");
        let mode = app.mode.map(|m| m.as_str()).unwrap_or("");
        let exec_command = app.exec_command.as_deref().unwrap_or("");
        let env_vars_json = app.env_vars.as_ref()
            .and_then(|v| serde_json::to_string(v).ok())
            .unwrap_or_default();

        let changed = conn.execute(
            "UPDATE apps SET app_type=?1, url=?2, mode=?3, show_nav=?4, exec_command=?5, env_vars=?6, remote_debugging_port=?7 WHERE id=?8",
            params![app.app_type.as_str(), url, mode, app.show_nav as i32, exec_command, env_vars_json, app.remote_debugging_port, app.id],
        ).map_err(|e| format!("Failed to update app: {}", e))?;
        if changed == 0 {
            return Err(format!("App '{}' not found", app.id));
        }
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        let changed = conn.execute("DELETE FROM apps WHERE id=?1", params![id])
            .map_err(|e| format!("Failed to delete app: {}", e))?;
        if changed == 0 {
            return Err(format!("App '{}' not found", id));
        }
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<PakeApp, String> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, name, app_type, url, mode, show_nav, exec_command, env_vars, created_at, remote_debugging_port FROM apps WHERE id=?1",
            params![id],
            |row| Ok(Self::row_to_app(row)),
        ).map_err(|e| format!("App not found: {}", e))
    }

    pub fn list(&self) -> Result<Vec<PakeApp>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, app_type, url, mode, show_nav, exec_command, env_vars, created_at, remote_debugging_port FROM apps ORDER BY created_at"
        ).map_err(|e| format!("Failed to list apps: {}", e))?;

        let apps = stmt.query_map([], |row| Ok(Self::row_to_app(row)))
            .map_err(|e| format!("Failed to query apps: {}", e))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(apps)
    }

    fn row_to_app(row: &rusqlite::Row) -> PakeApp {
        let app_type_str: String = row.get(2).unwrap_or_else(|_| "webapp".to_string());
        let app_type = AppType::from_str(&app_type_str).unwrap_or(AppType::WebApp);

        let url: Option<String> = row.get(3).ok().filter(|s: &String| !s.is_empty());
        let mode_str: Option<String> = row.get(4).ok().filter(|s: &String| !s.is_empty());
        let mode = mode_str.and_then(|s| AppMode::from_str(&s));
        let show_nav = row.get::<_, i32>(5).unwrap_or(0) != 0;

        let exec_command: Option<String> = row.get(6).ok().filter(|s: &String| !s.is_empty());
        let env_vars_json: Option<String> = row.get(7).ok().filter(|s: &String| !s.is_empty());
        let env_vars = env_vars_json.and_then(|json| serde_json::from_str(&json).ok());
        let remote_debugging_port: Option<u16> = row.get::<_, Option<i32>>(9)
            .unwrap_or(None).map(|p| p as u16);

        PakeApp {
            id: row.get(0).unwrap_or_default(),
            name: row.get(1).unwrap_or_default(),
            app_type,
            url,
            mode,
            show_nav,
            remote_debugging_port,
            exec_command,
            env_vars,
            created_at: row.get(8).unwrap_or_default(),
        }
    }
}
