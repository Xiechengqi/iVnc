use super::app::{AppMode, AppType, PakeApp};
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
                mode TEXT,
                dark_mode INTEGER DEFAULT 0,
                autostart INTEGER DEFAULT 0,
                show_nav INTEGER DEFAULT 0,
                created_at TEXT NOT NULL,
                app_type TEXT DEFAULT 'webapp',
                exec_command TEXT,
                env_vars TEXT,
                remote_debugging_port INTEGER,
                proxy_server TEXT,
                launch_command TEXT,
                launch_env_vars TEXT,
                launch_cwd TEXT,
                launch_wait_url TEXT,
                launch_wait_timeout_secs INTEGER
            );",
        )
        .map_err(|e| format!("Failed to init db: {}", e))?;

        // Migrations
        let _ = conn.execute("ALTER TABLE apps ADD COLUMN show_nav INTEGER DEFAULT 0", []);
        let _ = conn.execute(
            "ALTER TABLE apps ADD COLUMN app_type TEXT DEFAULT 'webapp'",
            [],
        );
        let _ = conn.execute("ALTER TABLE apps ADD COLUMN exec_command TEXT", []);
        let _ = conn.execute("ALTER TABLE apps ADD COLUMN env_vars TEXT", []);
        let _ = conn.execute(
            "ALTER TABLE apps ADD COLUMN remote_debugging_port INTEGER",
            [],
        );
        let _ = conn.execute("ALTER TABLE apps ADD COLUMN proxy_server TEXT", []);
        let _ = conn.execute("ALTER TABLE apps ADD COLUMN launch_command TEXT", []);
        let _ = conn.execute("ALTER TABLE apps ADD COLUMN launch_env_vars TEXT", []);
        let _ = conn.execute("ALTER TABLE apps ADD COLUMN launch_cwd TEXT", []);
        let _ = conn.execute("ALTER TABLE apps ADD COLUMN launch_wait_url TEXT", []);
        let _ = conn.execute(
            "ALTER TABLE apps ADD COLUMN launch_wait_timeout_secs INTEGER",
            [],
        );

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

    pub fn add(&self, app: &PakeApp) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();

        let url = app.url.as_deref().unwrap_or("");
        let mode = app.mode.map(|m| m.as_str()).unwrap_or("");
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

        conn.execute(
            "INSERT INTO apps (id, name, app_type, url, mode, autostart, show_nav, exec_command, env_vars, created_at, remote_debugging_port, proxy_server, launch_command, launch_env_vars, launch_cwd, launch_wait_url, launch_wait_timeout_secs)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            params![
                app.id, app.name, app.app_type.as_str(), url, mode,
                app.autostart as i32, app.show_nav as i32, exec_command, env_vars_json, app.created_at,
                app.remote_debugging_port, app.proxy_server, app.launch_command, launch_env_vars_json,
                app.launch_cwd, app.launch_wait_url, app.launch_wait_timeout_secs.map(|v| v as i64),
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

        let changed = conn.execute(
            "UPDATE apps SET app_type=?1, url=?2, mode=?3, autostart=?4, show_nav=?5, exec_command=?6, env_vars=?7, remote_debugging_port=?8, proxy_server=?9, launch_command=?10, launch_env_vars=?11, launch_cwd=?12, launch_wait_url=?13, launch_wait_timeout_secs=?14 WHERE id=?15",
            params![app.app_type.as_str(), url, mode, app.autostart as i32, app.show_nav as i32, exec_command, env_vars_json, app.remote_debugging_port, app.proxy_server, app.launch_command, launch_env_vars_json, app.launch_cwd, app.launch_wait_url, app.launch_wait_timeout_secs.map(|v| v as i64), app.id],
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

    pub fn get(&self, id: &str) -> Result<PakeApp, String> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, name, app_type, url, mode, autostart, show_nav, exec_command, env_vars, created_at, remote_debugging_port, proxy_server, launch_command, launch_env_vars, launch_cwd, launch_wait_url, launch_wait_timeout_secs FROM apps WHERE id=?1",
            params![id],
            |row| Ok(Self::row_to_app(row)),
        ).map_err(|e| format!("App not found: {}", e))
    }

    pub fn list(&self) -> Result<Vec<PakeApp>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, app_type, url, mode, autostart, show_nav, exec_command, env_vars, created_at, remote_debugging_port, proxy_server, launch_command, launch_env_vars, launch_cwd, launch_wait_url, launch_wait_timeout_secs FROM apps ORDER BY created_at"
        ).map_err(|e| format!("Failed to list apps: {}", e))?;

        let apps = stmt
            .query_map([], |row| Ok(Self::row_to_app(row)))
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
        let autostart = row.get::<_, i32>(5).unwrap_or(0) != 0;
        let show_nav = row.get::<_, i32>(6).unwrap_or(0) != 0;

        let exec_command: Option<String> = row.get(7).ok().filter(|s: &String| !s.is_empty());
        let env_vars_json: Option<String> = row.get(8).ok().filter(|s: &String| !s.is_empty());
        let env_vars = env_vars_json.and_then(|json| serde_json::from_str(&json).ok());
        let remote_debugging_port: Option<u16> = row
            .get::<_, Option<i32>>(10)
            .unwrap_or(None)
            .map(|p| p as u16);
        let proxy_server: Option<String> = row.get(11).ok().filter(|s: &String| !s.is_empty());
        let launch_command: Option<String> = row.get(12).ok().filter(|s: &String| !s.is_empty());
        let launch_env_vars_json: Option<String> =
            row.get(13).ok().filter(|s: &String| !s.is_empty());
        let launch_env_vars =
            launch_env_vars_json.and_then(|json| serde_json::from_str(&json).ok());
        let launch_cwd: Option<String> = row.get(14).ok().filter(|s: &String| !s.is_empty());
        let launch_wait_url: Option<String> = row.get(15).ok().filter(|s: &String| !s.is_empty());
        let launch_wait_timeout_secs: Option<u64> = row
            .get::<_, Option<i64>>(16)
            .unwrap_or(None)
            .and_then(|v| u64::try_from(v).ok());

        PakeApp {
            id: row.get(0).unwrap_or_default(),
            name: row.get(1).unwrap_or_default(),
            app_type,
            autostart,
            url,
            mode,
            show_nav,
            remote_debugging_port,
            proxy_server,
            launch_command,
            launch_env_vars,
            launch_cwd,
            launch_wait_url,
            launch_wait_timeout_secs,
            exec_command,
            env_vars,
            created_at: row.get(9).unwrap_or_default(),
        }
    }
}
