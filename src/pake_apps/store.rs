use rusqlite::{Connection, params};
use std::path::PathBuf;
use std::sync::Mutex;
use super::app::{PakeApp, AppMode};

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
                url TEXT NOT NULL,
                mode TEXT NOT NULL,
                dark_mode INTEGER DEFAULT 0,
                autostart INTEGER DEFAULT 0,
                show_nav INTEGER DEFAULT 0,
                created_at TEXT NOT NULL
            );"
        ).map_err(|e| format!("Failed to init db: {}", e))?;

        // Migration: Add show_nav column if it doesn't exist
        let _ = conn.execute(
            "ALTER TABLE apps ADD COLUMN show_nav INTEGER DEFAULT 0",
            [],
        );

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
        conn.execute(
            "INSERT INTO apps (id, name, url, mode, dark_mode, autostart, show_nav, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                app.id, app.name, app.url, app.mode.as_str(),
                app.dark_mode as i32, app.autostart as i32, app.show_nav as i32, app.created_at,
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

    pub fn update(&self, id: &str, url: &str, mode: AppMode, dark_mode: bool, autostart: bool, show_nav: bool) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        let changed = conn.execute(
            "UPDATE apps SET url=?1, mode=?2, dark_mode=?3, autostart=?4, show_nav=?5 WHERE id=?6",
            params![url, mode.as_str(), dark_mode as i32, autostart as i32, show_nav as i32, id],
        ).map_err(|e| format!("Failed to update app: {}", e))?;
        if changed == 0 {
            return Err(format!("App '{}' not found", id));
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
            "SELECT id, name, url, mode, dark_mode, autostart, show_nav, created_at FROM apps WHERE id=?1",
            params![id],
            |row| Ok(Self::row_to_app(row)),
        ).map_err(|e| format!("App not found: {}", e))
    }

    pub fn list(&self) -> Result<Vec<PakeApp>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, url, mode, dark_mode, autostart, show_nav, created_at FROM apps ORDER BY created_at"
        ).map_err(|e| format!("Failed to list apps: {}", e))?;

        let apps = stmt.query_map([], |row| Ok(Self::row_to_app(row)))
            .map_err(|e| format!("Failed to query apps: {}", e))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(apps)
    }

    fn row_to_app(row: &rusqlite::Row) -> PakeApp {
        let mode_str: String = row.get(3).unwrap_or_default();
        PakeApp {
            id: row.get(0).unwrap_or_default(),
            name: row.get(1).unwrap_or_default(),
            url: row.get(2).unwrap_or_default(),
            mode: AppMode::from_str(&mode_str).unwrap_or(AppMode::Webview),
            dark_mode: row.get::<_, i32>(4).unwrap_or(0) != 0,
            autostart: row.get::<_, i32>(5).unwrap_or(0) != 0,
            show_nav: row.get::<_, i32>(6).unwrap_or(0) != 0,
            created_at: row.get(7).unwrap_or_default(),
        }
    }
}
