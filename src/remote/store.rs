use crate::apps::error::ApiError;
use argon2::{password_hash::SaltString, Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use chacha20poly1305::{aead::Aead, ChaCha20Poly1305, KeyInit, Nonce};
use rand_core::{OsRng, RngCore};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::time::{Duration, Instant};
use std::{fs, os::unix::fs::OpenOptionsExt, path::Path, sync::Mutex};

#[derive(Clone, Debug, Serialize)]
pub struct Policy {
    pub enabled: bool,
    pub password_set: bool,
    /// Authorization revision. Session tokens are bound to it.
    pub revision: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Target {
    pub base_url: String,
    pub app_id: String,
    pub node_id: String,
    pub name: String,
    /// The application password on B; empty for connections saved by earlier versions.
    #[serde(skip_serializing, default)]
    pub password: String,
}
impl Target {
    /// The address this connection was made with.
    pub fn share_url(&self) -> String {
        format!("{}{}", self.base_url, super::share_path(&self.app_id))
    }
}

/// Session tokens expire after this long without use.
const SESSION_IDLE: Duration = Duration::from_secs(12 * 3600);
/// Sessions kept per application. Logins to one application never end the
/// sessions of another.
const SESSIONS_PER_APP: usize = 64;

struct Session {
    app: String,
    revision: u64,
    used: Instant,
}

pub struct RemoteStore {
    db: Mutex<Connection>,
    cipher: ChaCha20Poly1305,
    /// Session tokens by digest. They are kept in memory only, so a restart of
    /// this instance makes connected instances authenticate again.
    sessions: Mutex<HashMap<String, Session>>,
    changes: tokio::sync::watch::Sender<u64>,
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
                if let Err(e) = file.write_all(&key).and_then(|_| file.sync_all()) {
                    // Never leave a short key behind; it would fail every later start.
                    let _ = fs::remove_file(&key_path);
                    return Err(e.to_string());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                fs::File::open(&key_path)
                    .and_then(|mut f| f.read_exact(&mut key))
                    .map_err(|e| format!("Cannot read {}: {e}", key_path.display()))?;
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
        // Earlier versions kept an access log here.
        for log in ["remote-audit.log", "remote-audit.log.1"] {
            let _ = fs::remove_file(root.join(log));
        }
        Self::open(Connection::open(path).map_err(|e| e.to_string())?, &key)
    }

    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self::open(Connection::open_in_memory().unwrap(), &[7; 32]).unwrap()
    }

    fn open(mut db: Connection, key: &[u8; 32]) -> Result<Self, String> {
        db.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        // policies.desktop held runtime settings of earlier versions. It is kept,
        // unused, so that those versions can still open this database.
        db.execute_batch("PRAGMA foreign_keys=ON;
          CREATE TABLE IF NOT EXISTS identity (id INTEGER PRIMARY KEY CHECK(id=1), node TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS policies (app TEXT PRIMARY KEY, enabled INTEGER NOT NULL DEFAULT 0, password TEXT, revision INTEGER NOT NULL DEFAULT 0, desktop TEXT NOT NULL DEFAULT '{}', password_secret BLOB);
          CREATE TABLE IF NOT EXISTS targets (app TEXT PRIMARY KEY, metadata TEXT NOT NULL, secret BLOB NOT NULL);
        ").map_err(|e| e.to_string())?;
        migrate(&mut db).map_err(|e| e.to_string())?;
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
            sessions: Mutex::new(HashMap::new()),
            changes: tokio::sync::watch::channel(0).0,
            node_id,
        })
    }

    /// Wakes holders of remote access whenever a session may have become invalid.
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.changes.subscribe()
    }

    fn changed(&self, app: &str) {
        self.sessions.lock().unwrap().retain(|_, s| s.app != app);
        self.changes.send_modify(|v| *v = v.wrapping_add(1));
    }

    /// Encrypts a secret; `aad` binds it to the row it belongs to.
    fn seal(&self, aad: &[u8], secret: &[u8]) -> Result<Vec<u8>, String> {
        let mut nonce = [0u8; 12];
        OsRng.fill_bytes(&mut nonce);
        let encrypted = self
            .cipher
            .encrypt(
                &Nonce::from(nonce),
                chacha20poly1305::aead::Payload { msg: secret, aad },
            )
            .map_err(|_| "Cannot protect remote secret")?;
        let mut sealed = nonce.to_vec();
        sealed.extend(encrypted);
        Ok(sealed)
    }

    fn unseal(&self, aad: &[u8], sealed: &[u8]) -> Option<String> {
        if sealed.len() < 28 {
            return None;
        }
        let plain = self
            .cipher
            .decrypt(
                &Nonce::from(<[u8; 12]>::try_from(&sealed[..12]).unwrap()),
                chacha20poly1305::aead::Payload {
                    msg: &sealed[12..],
                    aad,
                },
            )
            .ok()?;
        String::from_utf8(plain).ok()
    }

    pub fn policy(&self, app: &str) -> Result<Policy, String> {
        let db = self.db.lock().unwrap();
        db.query_row(
            "SELECT enabled,password IS NOT NULL,revision FROM policies WHERE app=?1",
            [app],
            |r| {
                Ok(Policy {
                    enabled: r.get(0)?,
                    password_set: r.get(1)?,
                    revision: r.get(2)?,
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
            })
        })
    }

    /// The application password, for its owner to view. Passwords set by earlier
    /// versions were kept only as a hash and cannot be shown. The copy is bound to
    /// the hash it was saved with, so a password an earlier version changed later
    /// is never shown in place of the current one.
    pub fn policy_password(&self, app: &str) -> Result<Option<String>, String> {
        let row: Option<(String, Option<Vec<u8>>)> = self
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT password,password_secret FROM policies WHERE app=?1 AND password IS NOT NULL",
                [app],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        Ok(row.and_then(|(hash, sealed)| self.unseal(policy_aad(app, &hash).as_bytes(), &sealed?)))
    }

    /// Updates sharing. Access changes (enable/disable or a new password) advance
    /// the revision, which ends every session.
    pub fn set_policy(
        &self,
        app: &str,
        enabled: bool,
        password: Option<&str>,
    ) -> Result<Policy, ApiError> {
        if !super::valid_id(app) {
            return Err(ApiError::new(
                "app_id_invalid",
                "Invalid application identifier",
            ));
        }
        let password = password.filter(|p| !p.is_empty());
        if password.is_some_and(|p| p.len() < 8 || p.len() > 256) {
            return Err(ApiError::new(
                "share_password_length",
                "Application password must be 8–256 bytes",
            ));
        }
        let secret = password
            .map(|p| {
                let hash = Argon2::default()
                    .hash_password(p.as_bytes(), &SaltString::generate(&mut OsRng))
                    .map(|v| v.to_string())
                    .map_err(|e| e.to_string())?;
                let sealed = self.seal(policy_aad(app, &hash).as_bytes(), p.as_bytes())?;
                Ok::<_, String>((hash, sealed))
            })
            .transpose()?;
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let old: Option<(bool, Option<String>, Option<Vec<u8>>)> = tx
            .query_row(
                "SELECT enabled,password,password_secret FROM policies WHERE app=?1",
                [app],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let access_changed = secret.is_some()
            || old
                .as_ref()
                .is_none_or(|(old_enabled, _, _)| *old_enabled != enabled);
        let (hash, sealed) = match secret {
            Some((hash, sealed)) => (Some(hash), Some(sealed)),
            None => old.map_or((None, None), |(_, hash, sealed)| (hash, sealed)),
        };
        if enabled && hash.is_none() {
            return Err(ApiError::new(
                "share_password_required",
                "Set a password before enabling remote access",
            ));
        }
        if access_changed {
            tx.execute("INSERT INTO policies(app,enabled,password,password_secret,revision) VALUES(?1,?2,?3,?4,1) ON CONFLICT(app) DO UPDATE SET enabled=excluded.enabled,password=excluded.password,password_secret=excluded.password_secret,revision=policies.revision+1",params![app,enabled,hash,sealed]).map_err(|e|e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        drop(db);
        if access_changed {
            self.changed(app);
        }
        Ok(self.policy(app)?)
    }

    /// Exchanges an application password for a session token of that application.
    /// Applications that are not shared are refused without password work, so
    /// unauthenticated requests cannot occupy Argon2 capacity with random IDs.
    pub fn authenticate(&self, app: &str, password: &str) -> Result<String, ApiError> {
        let record: Option<(String, u64)> = if password.len() > 256 {
            None
        } else {
            self.db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT password,revision FROM policies WHERE app=?1 AND enabled=1 AND password IS NOT NULL",
                    [app],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(|e| e.to_string())?
        };
        let Some((hash, revision)) = record else {
            return Err(invalid_credentials());
        };
        if !verify_password(&hash, password) {
            return Err(invalid_credentials());
        }
        if self.revision(app)? != Some(revision) {
            return Err(ApiError::new(
                "auth_access_changed",
                "Application access changed; retry authentication",
            ));
        }
        Ok(self.open_session(app, revision))
    }

    /// Issues a session token. Beyond the per-application limit, the session of
    /// that application used least recently ends.
    fn open_session(&self, app: &str, revision: u64) -> String {
        let token = super::token();
        let mut sessions = self.sessions.lock().unwrap();
        sessions.retain(|_, s| s.used.elapsed() < SESSION_IDLE);
        while sessions.values().filter(|s| s.app == app).count() >= SESSIONS_PER_APP {
            let oldest = sessions
                .iter()
                .filter(|(_, s)| s.app == app)
                .min_by_key(|(_, s)| s.used)
                .map(|(k, _)| k.clone());
            match oldest {
                Some(key) => sessions.remove(&key),
                None => break,
            };
        }
        sessions.insert(
            digest(&token),
            Session {
                app: app.to_owned(),
                revision,
                used: Instant::now(),
            },
        );
        token
    }

    /// Ends a session of the application, e.g. one opened only to test a connection.
    pub fn close_session(&self, app: &str, token: &str) {
        let key = digest(token);
        let mut sessions = self.sessions.lock().unwrap();
        if sessions.get(&key).is_some_and(|s| s.app == app) {
            sessions.remove(&key);
        }
    }

    /// Revision of an application while it is shared.
    fn revision(&self, app: &str) -> Result<Option<u64>, String> {
        self.db
            .lock()
            .unwrap()
            .query_row(
                "SELECT revision FROM policies WHERE app=?1 AND enabled=1",
                [app],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    /// Whether a session token is valid for the application. Use keeps it alive.
    pub fn verify(&self, app: &str, token: &str) -> bool {
        if token.len() != 64 {
            return false;
        }
        let key = digest(token);
        let revision = match self.sessions.lock().unwrap().get(&key) {
            Some(s) if s.app == app && s.used.elapsed() < SESSION_IDLE => s.revision,
            _ => return false,
        };
        if self.revision(app).ok().flatten() != Some(revision) {
            return false;
        }
        if let Some(session) = self.sessions.lock().unwrap().get_mut(&key) {
            session.used = Instant::now();
            return true;
        }
        false
    }

    pub fn save_target(&self, app: &str, target: &Target) -> Result<(), String> {
        let secret = self.seal(target_aad(app).as_bytes(), target.password.as_bytes())?;
        self.db.lock().unwrap().execute("INSERT INTO targets VALUES(?1,?2,?3) ON CONFLICT(app) DO UPDATE SET metadata=excluded.metadata,secret=excluded.secret",params![app,serde_json::to_string(target).map_err(|e|e.to_string())?,secret]).map_err(|e|e.to_string())?;
        Ok(())
    }

    /// The saved connection. Earlier versions kept a credential of B in the same
    /// column, sealed for the row alone; it never opens as a password, so their
    /// connections, including any they save after a downgrade, have none.
    pub fn target(&self, app: &str) -> Result<Target, ApiError> {
        let (text, secret): (String, Vec<u8>) = self
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT metadata,secret FROM targets WHERE app=?1",
                [app],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| {
                ApiError::new(
                    "remote_not_configured",
                    "Remote connection is not configured",
                )
            })?;
        let mut target: Target = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        target.password = self
            .unseal(target_aad(app).as_bytes(), &secret)
            .unwrap_or_default();
        Ok(target)
    }

    pub fn delete(&self, app: &str) -> Result<(), String> {
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction().map_err(|e| e.to_string())?;
        for table in ["policies", "targets"] {
            tx.execute(&format!("DELETE FROM {table} WHERE app=?1"), [app])
                .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        drop(db);
        self.changed(app);
        Ok(())
    }
}

/// Upgrades databases of earlier versions. Version 1 replaces saved credentials
/// with session tokens: grants are dropped. Connections saved on A must be given
/// their password again; see `RemoteStore::target`.
fn migrate(db: &mut Connection) -> rusqlite::Result<()> {
    let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version >= 1 {
        return Ok(());
    }
    let tx = db.transaction()?;
    tx.execute("DROP TABLE IF EXISTS grants", [])?;
    let has_secret: bool = tx.query_row(
        "SELECT count(*) FROM pragma_table_info('policies') WHERE name='password_secret'",
        [],
        |r| r.get(0),
    )?;
    if !has_secret {
        tx.execute("ALTER TABLE policies ADD COLUMN password_secret BLOB", [])?;
    }
    tx.execute_batch("PRAGMA user_version=1")?;
    tx.commit()
}

/// Binds a viewable password to its application and to the hash it was saved with.
fn policy_aad(app: &str, hash: &str) -> String {
    format!("policy:{app}\n{hash}")
}

/// Binds a saved password to its connection. Earlier versions sealed their
/// credentials with the bare application ID.
fn target_aad(app: &str) -> String {
    format!("target-password:{app}")
}

/// The one answer to wrong passwords and unknown or unshared applications.
pub fn invalid_credentials() -> ApiError {
    ApiError::new("auth_invalid", "Invalid application credentials")
}

fn verify_password(hash: &str, password: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|parsed| {
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok()
    })
}

pub(super) fn truncate(value: &str, max: usize) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect()
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
    fn session_tokens_are_app_scoped_and_end_with_access_changes() {
        let s = store();
        assert!(!s.policy("one").unwrap().enabled);
        assert!(s.set_policy("one", true, None).is_err());
        s.set_policy("one", true, Some("password-one")).unwrap();
        s.set_policy("two", true, Some("password-two")).unwrap();
        let token = s.authenticate("one", "password-one").unwrap();
        assert!(s.verify("one", &token));
        assert!(!s.verify("two", &token));
        assert!(s.authenticate("two", "password-one").is_err());
        assert!(s.authenticate("missing", "password-one").is_err());
        s.set_policy("one", true, Some("password-new")).unwrap();
        assert!(!s.verify("one", &token));
        assert!(s.authenticate("one", "password-one").is_err());
        let new = s.authenticate("one", "password-new").unwrap();
        s.set_policy("one", false, None).unwrap();
        assert!(!s.verify("one", &new));
        s.set_policy("one", true, None).unwrap();
        assert!(!s.verify("one", &new));
        // Tokens expire after a period without use.
        let idle = s.authenticate("one", "password-new").unwrap();
        s.sessions
            .lock()
            .unwrap()
            .get_mut(&digest(&idle))
            .unwrap()
            .used = Instant::now() - SESSION_IDLE;
        assert!(!s.verify("one", &idle));
    }

    #[test]
    fn errors_carry_codes() {
        let s = store();
        let code = |r: Result<Policy, ApiError>| r.unwrap_err().code;
        assert_eq!(
            code(s.set_policy("one", true, None)),
            "share_password_required"
        );
        assert_eq!(
            code(s.set_policy("one", true, Some("short"))),
            "share_password_length"
        );
        assert_eq!(code(s.set_policy("bad id", false, None)), "app_id_invalid");
        assert_eq!(
            s.authenticate("one", "password-one").unwrap_err().code,
            "auth_invalid"
        );
        assert_eq!(s.target("one").unwrap_err().code, "remote_not_configured");
    }

    #[test]
    fn sessions_are_limited_per_application() {
        let s = store();
        let other = s.open_session("two", 1);
        let first = s.open_session("one", 1);
        let tokens: Vec<String> = (1..SESSIONS_PER_APP)
            .map(|_| s.open_session("one", 1))
            .collect();
        let count = |app: &str| {
            s.sessions
                .lock()
                .unwrap()
                .values()
                .filter(|v| v.app == app)
                .count()
        };
        assert_eq!(count("one"), SESSIONS_PER_APP);
        let session = |token: &str| s.sessions.lock().unwrap().contains_key(&digest(token));
        // One more login ends the oldest session of the same application only.
        s.open_session("one", 1);
        assert_eq!(count("one"), SESSIONS_PER_APP);
        assert!(!session(&first));
        assert!(tokens.iter().all(|t| session(t)));
        assert!(session(&other));
        // A token closes only sessions of its own application.
        s.close_session("one", &other);
        assert!(session(&other));
        s.close_session("two", &other);
        assert!(!session(&other));
    }

    #[test]
    fn unchanged_access_does_not_end_sessions() {
        let s = store();
        let mut changes = s.subscribe();
        s.set_policy("one", true, Some("password-one")).unwrap();
        let revision = s.policy("one").unwrap().revision;
        let token = s.authenticate("one", "password-one").unwrap();
        changes.mark_unchanged();
        let policy = s.set_policy("one", true, None).unwrap();
        assert_eq!(policy.revision, revision);
        assert!(s.verify("one", &token));
        assert!(!changes.has_changed().unwrap());
        s.set_policy("one", false, None).unwrap();
        assert!(changes.has_changed().unwrap());
        assert!(!s.verify("one", &token));
    }

    #[test]
    fn passwords_can_be_viewed_by_their_owner_only_when_saved_by_this_version() {
        let s = store();
        assert_eq!(s.policy_password("one").unwrap(), None);
        s.set_policy("one", false, Some("password-one")).unwrap();
        assert_eq!(
            s.policy_password("one").unwrap().as_deref(),
            Some("password-one")
        );
        // Keeping the password keeps its viewable copy.
        s.set_policy("one", true, None).unwrap();
        assert_eq!(
            s.policy_password("one").unwrap().as_deref(),
            Some("password-one")
        );
        // The copy is bound to its application.
        s.set_policy("two", false, Some("password-two")).unwrap();
        s.db.lock()
            .unwrap()
            .execute(
                "UPDATE policies SET password_secret=(SELECT password_secret FROM policies WHERE app='one') WHERE app='two'",
                [],
            )
            .unwrap();
        assert_eq!(s.policy_password("two").unwrap(), None);
        // An earlier version that changes the password leaves the copy behind.
        s.set_policy("three", true, Some("password-three")).unwrap();
        s.db.lock()
            .unwrap()
            .execute(
                "UPDATE policies SET password=(SELECT password FROM policies WHERE app='two') WHERE app='three'",
                [],
            )
            .unwrap();
        assert!(s.policy("three").unwrap().password_set);
        assert_eq!(s.policy_password("three").unwrap(), None);
        // Earlier versions stored only the hash.
        s.db.lock()
            .unwrap()
            .execute(
                "UPDATE policies SET password_secret=NULL WHERE app='one'",
                [],
            )
            .unwrap();
        let policy = s.policy("one").unwrap();
        assert!(policy.enabled && policy.password_set);
        assert_eq!(s.policy_password("one").unwrap(), None);
        assert!(s.authenticate("one", "password-one").is_ok());
    }

    #[test]
    fn policies_with_legacy_runtime_settings_still_load() {
        let s = store();
        s.db.lock()
            .unwrap()
            .execute(
                "INSERT INTO policies(app,enabled,password,revision,desktop) VALUES('one',0,NULL,3,?1)",
                [r#"{"x11":true,"isolation":"host","mounts":[{"path":"/data","writable":true}],"networks":["192.168.1.0/24"],"services":["svc"]}"#],
            )
            .unwrap();
        let policy = s.policy("one").unwrap();
        assert!(!policy.enabled && !policy.password_set);
        assert_eq!(policy.revision, 3);
        let policy = s.set_policy("one", true, Some("password-one")).unwrap();
        assert!(policy.enabled && policy.password_set);
        assert_eq!(policy.revision, 4);
        // Left for earlier versions, which still read it.
        let desktop: String =
            s.db.lock()
                .unwrap()
                .query_row("SELECT desktop FROM policies WHERE app='one'", [], |r| {
                    r.get(0)
                })
                .unwrap();
        assert!(desktop.contains("\"isolation\":\"host\""));
    }

    #[test]
    fn databases_of_earlier_versions_are_migrated_once() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("
          CREATE TABLE policies (app TEXT PRIMARY KEY, enabled INTEGER NOT NULL DEFAULT 0, password TEXT, revision INTEGER NOT NULL DEFAULT 0, desktop TEXT NOT NULL DEFAULT '{}');
          CREATE TABLE grants (hash TEXT PRIMARY KEY, app TEXT NOT NULL, revision INTEGER NOT NULL, created INTEGER NOT NULL DEFAULT (unixepoch()), client TEXT NOT NULL DEFAULT '');
          CREATE TABLE targets (app TEXT PRIMARY KEY, metadata TEXT NOT NULL, secret BLOB NOT NULL);
          INSERT INTO policies(app,enabled,password,revision) VALUES('one',1,'$argon2id$legacy',2);
          INSERT INTO grants(hash,app,revision) VALUES('digest','one',2);
          INSERT INTO targets VALUES('mapping','{\"base_url\":\"https://b.example\",\"app_id\":\"one\",\"node_id\":\"node\",\"name\":\"App\"}',x'00');
        ").unwrap();
        let s = RemoteStore::open(db, &[7; 32]).unwrap();
        let db = s.db.lock().unwrap();
        let version: i64 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 1);
        let grants: i64 = db
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='grants'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(grants, 0);
        drop(db);
        let policy = s.policy("one").unwrap();
        assert!(policy.enabled && policy.password_set);
        assert_eq!(policy.revision, 2);
        assert_eq!(s.policy_password("one").unwrap(), None);
        // The saved credential is not a password; A must ask for it again.
        let target = s.target("mapping").unwrap();
        assert_eq!(target.app_id, "one");
        assert!(target.password.is_empty());
        let saved = Target {
            password: "password-one".into(),
            ..target
        };
        s.save_target("mapping", &saved).unwrap();
        assert_eq!(s.target("mapping").unwrap().password, "password-one");
        // Opening again does not repeat the migration.
        let db = std::mem::replace(
            &mut *s.db.lock().unwrap(),
            Connection::open_in_memory().unwrap(),
        );
        let s = RemoteStore::open(db, &[7; 32]).unwrap();
        assert_eq!(s.target("mapping").unwrap().password, "password-one");
    }

    #[test]
    fn saved_passwords_are_encrypted_and_bound_to_mapping() {
        let s = store();
        let t = Target {
            base_url: "https://b.example".into(),
            app_id: "one".into(),
            node_id: "node".into(),
            name: "App".into(),
            password: "password-one".into(),
        };
        s.save_target("mapping", &t).unwrap();
        assert_eq!(s.target("mapping").unwrap().password, t.password);
        assert!(!serde_json::to_string(&t).unwrap().contains(&t.password));
        let stored: Vec<u8> =
            s.db.lock()
                .unwrap()
                .query_row("SELECT secret FROM targets", [], |r| r.get(0))
                .unwrap();
        assert!(!String::from_utf8_lossy(&stored).contains(&t.password));
        s.db.lock()
            .unwrap()
            .execute("UPDATE targets SET app='other'", [])
            .unwrap();
        assert!(s.target("other").unwrap().password.is_empty());
    }

    #[test]
    fn credentials_of_earlier_versions_are_not_passwords() {
        let s = store();
        let t = Target {
            base_url: "https://b.example".into(),
            app_id: "one".into(),
            node_id: "node".into(),
            name: "App".into(),
            password: "password-one".into(),
        };
        s.save_target("mapping", &t).unwrap();
        // After a downgrade, an earlier version saves the connection again with
        // a credential of B, sealed for the row alone and without any marker.
        let credential = s.seal(b"mapping", b"credential-token").unwrap();
        s.db.lock()
            .unwrap()
            .execute(
                "UPDATE targets SET secret=?1 WHERE app='mapping'",
                [credential],
            )
            .unwrap();
        let target = s.target("mapping").unwrap();
        assert_eq!(target.share_url(), "https://b.example/remote/v1/apps/one");
        assert!(target.password.is_empty());
    }
}
