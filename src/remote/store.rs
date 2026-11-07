use argon2::{password_hash::SaltString, Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use chacha20poly1305::{aead::Aead, ChaCha20Poly1305, KeyInit, Nonce};
use rand_core::{OsRng, RngCore};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::{fs, os::unix::fs::OpenOptionsExt, path::Path, sync::Mutex};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct DesktopSettings {
    pub x11: bool,
    /// IDs of registered background apps, resolved by the owner before launch.
    pub services: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Policy {
    pub enabled: bool,
    pub password_set: bool,
    pub revision: u64,
    pub desktop: DesktopSettings,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Target {
    pub base_url: String,
    pub app_id: String,
    pub node_id: String,
    pub name: String,
    #[serde(skip_serializing, default)]
    pub credential: String,
}

pub struct RemoteStore {
    db: Mutex<Connection>,
    cipher: ChaCha20Poly1305,
    pub node_id: String,
}

impl RemoteStore {
    pub fn new() -> Result<Self, String> {
        let root = crate::paths::ivnc_home();
        fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        let key_path = root.join("remote.key");
        let mut key = [0u8; 32];
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&key_path)
        {
            Ok(mut file) => {
                OsRng.fill_bytes(&mut key);
                file.write_all(&key).map_err(|e| e.to_string())?;
                file.sync_all().map_err(|e| e.to_string())?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                fs::File::open(&key_path)
                    .and_then(|mut f| f.read_exact(&mut key))
                    .map_err(|e| e.to_string())?;
            }
            Err(e) => return Err(e.to_string()),
        }
        let path = root.join("remote.db");
        fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&path)
            .map_err(|e| e.to_string())?;
        fs::set_permissions(
            &key_path,
            std::os::unix::fs::PermissionsExt::from_mode(0o600),
        )
        .map_err(|e| e.to_string())?;
        fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600))
            .map_err(|e| e.to_string())?;
        Self::open(Connection::open(path).map_err(|e| e.to_string())?, &key)
    }

    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self::open(Connection::open_in_memory().unwrap(), &[7; 32]).unwrap()
    }

    fn open(db: Connection, key: &[u8; 32]) -> Result<Self, String> {
        db.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        db.execute_batch("PRAGMA foreign_keys=ON;
          CREATE TABLE IF NOT EXISTS identity (id INTEGER PRIMARY KEY CHECK(id=1), node TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS policies (app TEXT PRIMARY KEY, enabled INTEGER NOT NULL DEFAULT 0, password TEXT, revision INTEGER NOT NULL DEFAULT 0, desktop TEXT NOT NULL DEFAULT '{}');
          CREATE TABLE IF NOT EXISTS grants (hash TEXT PRIMARY KEY, app TEXT NOT NULL, revision INTEGER NOT NULL, created INTEGER NOT NULL DEFAULT (unixepoch()));
          CREATE TABLE IF NOT EXISTS targets (app TEXT PRIMARY KEY, metadata TEXT NOT NULL, secret BLOB NOT NULL);
        ").map_err(|e| e.to_string())?;
        db.execute(
            "INSERT OR IGNORE INTO identity VALUES(1,?1)",
            [uuid::Uuid::new_v4().to_string()],
        )
        .map_err(|e| e.to_string())?;
        let node_id = db
            .query_row("SELECT node FROM identity WHERE id=1", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        Ok(Self {
            db: Mutex::new(db),
            cipher: ChaCha20Poly1305::new(key.into()),
            node_id,
        })
    }

    pub fn policy(&self, app: &str) -> Result<Policy, String> {
        let db = self.db.lock().unwrap();
        db.query_row(
            "SELECT enabled,password IS NOT NULL,revision,desktop FROM policies WHERE app=?1",
            [app],
            |r| {
                let text: String = r.get(3)?;
                Ok(Policy {
                    enabled: r.get(0)?,
                    password_set: r.get(1)?,
                    revision: r.get(2)?,
                    desktop: serde_json::from_str(&text).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            3,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?,
                })
            },
        )
        .optional()
        .map_err(|e| e.to_string())
        .map(|p| {
            p.unwrap_or(Policy {
                enabled: false,
                password_set: false,
                revision: 0,
                desktop: DesktopSettings::default(),
            })
        })
    }

    pub fn set_policy(
        &self,
        app: &str,
        enabled: bool,
        password: Option<&str>,
        desktop: DesktopSettings,
    ) -> Result<Policy, String> {
        if !super::valid_id(app)
            || desktop.services.len() > 32
            || desktop.services.iter().any(|id| !super::valid_id(id))
        {
            return Err("Invalid application or service identifier".into());
        }
        let password = password.filter(|p| !p.is_empty());
        if password.is_some_and(|p| p.len() < 8 || p.len() > 256) {
            return Err("Application password must be 8–256 bytes".into());
        }
        let hash = password
            .map(|p| {
                Argon2::default()
                    .hash_password(p.as_bytes(), &SaltString::generate(&mut OsRng))
                    .map(|v| v.to_string())
                    .map_err(|e| e.to_string())
            })
            .transpose()?;
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let old: Option<(bool, Option<String>, String)> = tx
            .query_row(
                "SELECT enabled,password,desktop FROM policies WHERE app=?1",
                [app],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let desktop_json = serde_json::to_string(&desktop).map_err(|e| e.to_string())?;
        let changed = hash.is_some()
            || old.as_ref().is_none_or(|(old_enabled, _, old_desktop)| {
                *old_enabled != enabled || *old_desktop != desktop_json
            });
        let hash = hash.or_else(|| old.and_then(|(_, password, _)| password));
        if enabled && hash.is_none() {
            return Err("Set a password before enabling remote access".into());
        }
        if changed {
            tx.execute("INSERT INTO policies(app,enabled,password,revision,desktop) VALUES(?1,?2,?3,1,?4) ON CONFLICT(app) DO UPDATE SET enabled=excluded.enabled,password=excluded.password,revision=policies.revision+1,desktop=excluded.desktop",params![app,enabled,hash,desktop_json]).map_err(|e|e.to_string())?;
            tx.execute("DELETE FROM grants WHERE app=?1", [app])
                .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        drop(db);
        self.policy(app)
    }

    pub fn authenticate(&self, app: &str, password: &str) -> Result<String, String> {
        if password.len() > 256 {
            return Err("Invalid application credentials".into());
        }
        let record: Option<(String, u64)> = self
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT password,revision FROM policies WHERE app=?1 AND enabled=1",
                [app],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let (hash, revision) = record.ok_or("Invalid application credentials")?;
        let parsed = PasswordHash::new(&hash).map_err(|_| "Invalid application credentials")?;
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .map_err(|_| "Invalid application credentials")?;
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let current: Option<u64> = tx
            .query_row(
                "SELECT revision FROM policies WHERE app=?1 AND enabled=1",
                [app],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if current != Some(revision) {
            return Err("Application access changed; retry authentication".into());
        }
        // Bound saved credentials per application; old entries expire after 90 days.
        tx.execute("DELETE FROM grants WHERE created < unixepoch()-7776000", [])
            .map_err(|e| e.to_string())?;
        let count: u64 = tx
            .query_row("SELECT count(*) FROM grants WHERE app=?1", [app], |r| {
                r.get(0)
            })
            .map_err(|e| e.to_string())?;
        if count >= 128 {
            return Err(
                "Too many saved connections; reset the application password to revoke them".into(),
            );
        }
        let token = super::token();
        tx.execute(
            "INSERT INTO grants(hash,app,revision) VALUES(?1,?2,?3)",
            params![digest(&token), app, revision],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(token)
    }

    pub fn verify(&self, app: &str, token: &str) -> bool {
        if token.len() != 64 {
            return false;
        }
        self.db.lock().unwrap().query_row("SELECT 1 FROM grants g JOIN policies p ON p.app=g.app AND p.revision=g.revision WHERE g.hash=?1 AND g.app=?2 AND p.enabled=1 AND g.created>=unixepoch()-7776000",params![digest(token),app], |_|Ok(())).optional().ok().flatten().is_some()
    }

    pub fn revoke_grant(&self, app: &str, token: &str) -> Result<(), String> {
        self.db
            .lock()
            .unwrap()
            .execute(
                "DELETE FROM grants WHERE app=?1 AND hash=?2",
                params![app, digest(token)],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn save_target(&self, app: &str, target: &Target) -> Result<(), String> {
        let mut nonce = [0u8; 12];
        OsRng.fill_bytes(&mut nonce);
        let encrypted = self
            .cipher
            .encrypt(
                &Nonce::from(nonce),
                chacha20poly1305::aead::Payload {
                    msg: target.credential.as_bytes(),
                    aad: app.as_bytes(),
                },
            )
            .map_err(|_| "Cannot protect remote credential")?;
        let mut secret = nonce.to_vec();
        secret.extend(encrypted);
        self.db.lock().unwrap().execute("INSERT INTO targets VALUES(?1,?2,?3) ON CONFLICT(app) DO UPDATE SET metadata=excluded.metadata,secret=excluded.secret",params![app,serde_json::to_string(target).map_err(|e|e.to_string())?,secret]).map_err(|e|e.to_string())?;
        Ok(())
    }

    pub fn target(&self, app: &str) -> Result<Target, String> {
        let (text, secret): (String, Vec<u8>) = self
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT metadata,secret FROM targets WHERE app=?1",
                [app],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| "Remote connection is not configured")?;
        if secret.len() < 28 {
            return Err("Invalid saved credential".into());
        }
        let plain = self
            .cipher
            .decrypt(
                &Nonce::from(<[u8; 12]>::try_from(&secret[..12]).unwrap()),
                chacha20poly1305::aead::Payload {
                    msg: &secret[12..],
                    aad: app.as_bytes(),
                },
            )
            .map_err(|_| "Cannot decrypt remote credential; configure the connection again")?;
        let mut target: Target = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        target.credential = String::from_utf8(plain).map_err(|e| e.to_string())?;
        Ok(target)
    }

    pub fn delete(&self, app: &str) -> Result<(), String> {
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction().map_err(|e| e.to_string())?;
        for table in ["grants", "policies", "targets"] {
            tx.execute(&format!("DELETE FROM {table} WHERE app=?1"), [app])
                .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }
}

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

pub fn private_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension(format!("{}.tmp", super::token()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|e| e.to_string())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn store() -> RemoteStore {
        RemoteStore::open(Connection::open_in_memory().unwrap(), &[7; 32]).unwrap()
    }
    #[test]
    fn credentials_are_app_scoped_revocable_and_never_exported() {
        let s = store();
        assert!(!s.policy("one").unwrap().enabled);
        assert!(s
            .set_policy("one", true, None, DesktopSettings::default())
            .is_err());
        s.set_policy(
            "one",
            true,
            Some("password-one"),
            DesktopSettings::default(),
        )
        .unwrap();
        s.set_policy(
            "two",
            true,
            Some("password-two"),
            DesktopSettings::default(),
        )
        .unwrap();
        let grant = s.authenticate("one", "password-one").unwrap();
        assert!(s.verify("one", &grant));
        assert!(!s.verify("two", &grant));
        assert!(s.authenticate("two", "password-one").is_err());
        s.set_policy(
            "one",
            true,
            Some("password-new"),
            DesktopSettings::default(),
        )
        .unwrap();
        assert!(!s.verify("one", &grant));
        assert!(s.authenticate("one", "password-one").is_err());
        let new = s.authenticate("one", "password-new").unwrap();
        s.set_policy("one", false, None, DesktopSettings::default())
            .unwrap();
        assert!(!s.verify("one", &new));
    }
    #[test]
    fn saved_credentials_are_encrypted_and_bound_to_mapping() {
        let s = store();
        let t = Target {
            base_url: "https://b.example".into(),
            app_id: "one".into(),
            node_id: "node".into(),
            name: "App".into(),
            credential: super::super::token(),
        };
        s.save_target("mapping", &t).unwrap();
        assert_eq!(s.target("mapping").unwrap().credential, t.credential);
        assert!(!serde_json::to_string(&t).unwrap().contains(&t.credential));
        s.db.lock()
            .unwrap()
            .execute("UPDATE targets SET app='other'", [])
            .unwrap();
        assert!(s.target("other").is_err());
    }
}
