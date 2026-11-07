use argon2::{password_hash::SaltString, Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use chacha20poly1305::{aead::Aead, ChaCha20Poly1305, KeyInit, Nonce};
use rand_core::{OsRng, RngCore};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::{fs, net::Ipv4Addr, os::unix::fs::OpenOptionsExt, path::Path, sync::Mutex};

/// How much of the host a managed desktop application can reach.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Isolation {
    /// Private filesystem, network and services. Required for sharing.
    #[default]
    Strict,
    /// Host filesystem, network and environment, as before isolation existed.
    /// Only the display, audio and session bus remain private to the application.
    Host,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Mount {
    pub path: String,
    #[serde(default)]
    pub writable: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct DesktopSettings {
    pub x11: bool,
    /// IDs of registered background apps, resolved by the owner before launch.
    pub services: Vec<String>,
    pub isolation: Isolation,
    /// Extra host paths visible inside a strict runtime.
    pub mounts: Vec<Mount>,
    /// Private IPv4 networks (CIDR) a strict runtime may reach.
    pub networks: Vec<String>,
}

impl DesktopSettings {
    /// Syntax checks that do not depend on the host. Launch re-validates paths.
    pub fn validate(&self) -> Result<(), String> {
        if self.services.len() > 32 || self.services.iter().any(|id| !super::valid_id(id)) {
            return Err("Invalid service identifier".into());
        }
        if self.mounts.len() > 16 || self.networks.len() > 16 {
            return Err("Too many mounts or networks".into());
        }
        for mount in &self.mounts {
            // Rejected textually: `Path::components` silently drops "." segments.
            if !mount.path.starts_with('/')
                || mount.path.len() > 1024
                || mount.path.contains('\0')
                || mount.path[1..]
                    .split('/')
                    .any(|segment| segment.is_empty() || segment == "." || segment == "..")
            {
                return Err(format!(
                    "Mount path must be a normalized absolute path: {}",
                    mount.path
                ));
            }
        }
        for network in &self.networks {
            parse_network(network)?;
        }
        if self.isolation == Isolation::Host
            && (!self.mounts.is_empty() || !self.networks.is_empty())
        {
            return Err("Host mode already uses the host filesystem and network".into());
        }
        Ok(())
    }
}

/// slirp4netns' private network, 10.0.2.0/24, used inside every strict runtime.
const SLIRP_NETWORK: u32 = 0x0a00_0200;

/// Parses an allowed IPv4 network and returns its normalized address and prefix.
pub fn parse_network(value: &str) -> Result<(Ipv4Addr, u8), String> {
    let invalid = || format!("Invalid IPv4 network: {value}");
    let (ip, prefix) = value.trim().split_once('/').unwrap_or((value.trim(), "32"));
    let ip: Ipv4Addr = ip.parse().map_err(|_| invalid())?;
    let prefix: u8 = prefix.parse().map_err(|_| invalid())?;
    if !(8..=32).contains(&prefix) {
        return Err(format!("Network prefix must be between 8 and 32: {value}"));
    }
    let mask = u32::MAX << (32 - prefix as u32);
    let network = Ipv4Addr::from(u32::from(ip) & mask);
    let [a, b, _, _] = network.octets();
    // Loopback, the runtime's own NAT network and non-unicast space never become reachable.
    if a == 0 || a == 127 || a >= 224 || (a == 169 && b == 254) {
        return Err(format!("Network cannot be allowed: {value}"));
    }
    // Two networks overlap when the shorter prefix contains the other's address.
    let shared = u32::MAX << (32 - prefix.min(24) as u32);
    if u32::from(network) & shared == SLIRP_NETWORK & shared {
        return Err(format!(
            "Network overlaps the application runtime network: {value}"
        ));
    }
    Ok((network, prefix))
}

#[derive(Clone, Debug, Serialize)]
pub struct Policy {
    pub enabled: bool,
    pub password_set: bool,
    /// Authorization revision. Saved credentials are bound to it.
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

/// Identity reported by the connecting instance; informational only.
#[derive(Clone, Debug, Default)]
pub struct Client {
    pub node: String,
    pub label: String,
    pub ip: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Grant {
    pub id: String,
    pub client: String,
    pub label: String,
    pub ip: String,
    pub created: i64,
    pub last_used: Option<i64>,
}

const GRANT_LIFETIME: i64 = 90 * 24 * 3600;

pub struct RemoteStore {
    db: Mutex<Connection>,
    cipher: ChaCha20Poly1305,
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
        let columns: Vec<String> = db
            .prepare("SELECT name FROM pragma_table_info('grants')")
            .and_then(|mut s| s.query_map([], |r| r.get(0))?.collect())
            .map_err(|e| e.to_string())?;
        for (column, kind) in [
            ("client", "TEXT NOT NULL DEFAULT ''"),
            ("label", "TEXT NOT NULL DEFAULT ''"),
            ("ip", "TEXT NOT NULL DEFAULT ''"),
            ("last_used", "INTEGER"),
        ] {
            if !columns.iter().any(|c| c == column) {
                db.execute(
                    &format!("ALTER TABLE grants ADD COLUMN {column} {kind}"),
                    [],
                )
                .map_err(|e| e.to_string())?;
            }
        }
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
            changes: tokio::sync::watch::channel(0).0,
            node_id,
        })
    }

    /// Wakes holders of remote access whenever a credential may have become invalid.
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.changes.subscribe()
    }

    fn changed(&self) {
        self.changes.send_modify(|v| *v = v.wrapping_add(1));
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

    /// Updates sharing and runtime settings. Only access changes (enable/disable or a
    /// new password) advance the revision and revoke saved credentials; runtime
    /// settings take effect when the application restarts.
    pub fn set_policy(
        &self,
        app: &str,
        enabled: bool,
        password: Option<&str>,
        desktop: DesktopSettings,
    ) -> Result<Policy, String> {
        if !super::valid_id(app) {
            return Err("Invalid application identifier".into());
        }
        desktop.validate()?;
        if enabled && desktop.isolation == Isolation::Host {
            return Err("Shared applications must use strict isolation".into());
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
        let access_changed = hash.is_some()
            || old
                .as_ref()
                .is_none_or(|(old_enabled, _, _)| *old_enabled != enabled);
        let desktop_changed = old.as_ref().is_none_or(|(_, _, d)| *d != desktop_json);
        let hash = hash.or_else(|| old.and_then(|(_, password, _)| password));
        if enabled && hash.is_none() {
            return Err("Set a password before enabling remote access".into());
        }
        if access_changed {
            tx.execute("INSERT INTO policies(app,enabled,password,revision,desktop) VALUES(?1,?2,?3,1,?4) ON CONFLICT(app) DO UPDATE SET enabled=excluded.enabled,password=excluded.password,revision=policies.revision+1,desktop=excluded.desktop",params![app,enabled,hash,desktop_json]).map_err(|e|e.to_string())?;
            tx.execute("DELETE FROM grants WHERE app=?1", [app])
                .map_err(|e| e.to_string())?;
        } else if desktop_changed {
            tx.execute(
                "UPDATE policies SET desktop=?2 WHERE app=?1",
                params![app, desktop_json],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        drop(db);
        if access_changed {
            self.changed();
        }
        self.policy(app)
    }

    /// Exchanges an application password for an application-scoped credential.
    /// Applications that are not shared are refused without password work, so
    /// unauthenticated requests cannot occupy Argon2 capacity with random IDs.
    pub fn authenticate(
        &self,
        app: &str,
        password: &str,
        client: &Client,
    ) -> Result<String, String> {
        const INVALID: &str = "Invalid application credentials";
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
            return Err(INVALID.into());
        };
        if !verify_password(&hash, password) {
            return Err(INVALID.into());
        }
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
        tx.execute(
            "DELETE FROM grants WHERE created < unixepoch()-?1",
            [GRANT_LIFETIME],
        )
        .map_err(|e| e.to_string())?;
        let count: u64 = tx
            .query_row("SELECT count(*) FROM grants WHERE app=?1", [app], |r| {
                r.get(0)
            })
            .map_err(|e| e.to_string())?;
        if count >= 128 {
            return Err(
                "Too many saved connections; revoke unused connections on B or reset the application password".into(),
            );
        }
        let token = super::token();
        tx.execute(
            "INSERT INTO grants(hash,app,revision,client,label,ip,last_used) VALUES(?1,?2,?3,?4,?5,?6,unixepoch())",
            params![
                digest(&token),
                app,
                revision,
                truncate(&client.node, 64),
                truncate(&client.label, 128),
                truncate(&client.ip, 64)
            ],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(token)
    }

    pub fn verify(&self, app: &str, token: &str) -> bool {
        self.grant(app, token).is_some()
    }

    /// Returns the public grant ID and the reported client label for a valid credential.
    pub fn grant(&self, app: &str, token: &str) -> Option<(String, String)> {
        if token.len() != 64 {
            return None;
        }
        let hash = digest(token);
        self.db.lock().unwrap().query_row("SELECT g.label FROM grants g JOIN policies p ON p.app=g.app AND p.revision=g.revision WHERE g.hash=?1 AND g.app=?2 AND p.enabled=1 AND g.created>=unixepoch()-?3",params![hash,app,GRANT_LIFETIME], |r|r.get::<_,String>(0)).optional().ok().flatten().map(|label|(grant_id(&hash),label))
    }

    /// Records use of a credential, at most once a minute per credential.
    pub fn touch(&self, app: &str, token: &str, ip: &str) {
        let _ = self.db.lock().unwrap().execute(
            "UPDATE grants SET last_used=unixepoch(), ip=?3 WHERE hash=?1 AND app=?2 AND (last_used IS NULL OR last_used<unixepoch()-60 OR ip<>?3)",
            params![digest(token), app, truncate(ip, 64)],
        );
    }

    pub fn grants(&self, app: &str) -> Result<Vec<Grant>, String> {
        let db = self.db.lock().unwrap();
        let mut statement = db
            .prepare("SELECT g.hash,g.client,g.label,g.ip,g.created,g.last_used FROM grants g JOIN policies p ON p.app=g.app AND p.revision=g.revision WHERE g.app=?1 AND g.created>=unixepoch()-?2 ORDER BY coalesce(g.last_used,g.created) DESC")
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map(params![app, GRANT_LIFETIME], |r| {
                Ok(Grant {
                    id: grant_id(&r.get::<_, String>(0)?),
                    client: r.get(1)?,
                    label: r.get(2)?,
                    ip: r.get(3)?,
                    created: r.get(4)?,
                    last_used: r.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
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
        self.changed();
        Ok(())
    }

    /// Revokes a credential by its public ID; returns whether one was removed.
    pub fn revoke_grant_id(&self, app: &str, id: &str) -> Result<bool, String> {
        if id.len() != 16 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(false);
        }
        let removed = self
            .db
            .lock()
            .unwrap()
            .execute(
                "DELETE FROM grants WHERE app=?1 AND substr(hash,1,16)=?2",
                params![app, id],
            )
            .map_err(|e| e.to_string())?;
        if removed > 0 {
            self.changed();
        }
        Ok(removed > 0)
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
        tx.commit().map_err(|e| e.to_string())?;
        drop(db);
        self.changed();
        Ok(())
    }
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

/// The public grant ID is a prefix of the stored digest, never of the credential.
pub fn grant_id(hash: &str) -> String {
    hash.chars().take(16).collect()
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
    fn client() -> Client {
        Client {
            node: "a-node".into(),
            label: "A".into(),
            ip: "192.0.2.1".into(),
        }
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
        let grant = s.authenticate("one", "password-one", &client()).unwrap();
        assert!(s.verify("one", &grant));
        assert!(!s.verify("two", &grant));
        assert!(s.authenticate("two", "password-one", &client()).is_err());
        assert!(s
            .authenticate("missing", "password-one", &client())
            .is_err());
        s.set_policy(
            "one",
            true,
            Some("password-new"),
            DesktopSettings::default(),
        )
        .unwrap();
        assert!(!s.verify("one", &grant));
        assert!(s.authenticate("one", "password-one", &client()).is_err());
        let new = s.authenticate("one", "password-new", &client()).unwrap();
        s.set_policy("one", false, None, DesktopSettings::default())
            .unwrap();
        assert!(!s.verify("one", &new));
    }

    #[test]
    fn runtime_settings_do_not_revoke_credentials() {
        let s = store();
        let mut changes = s.subscribe();
        s.set_policy(
            "one",
            true,
            Some("password-one"),
            DesktopSettings::default(),
        )
        .unwrap();
        let revision = s.policy("one").unwrap().revision;
        let grant = s.authenticate("one", "password-one", &client()).unwrap();
        changes.mark_unchanged();
        let desktop = DesktopSettings {
            x11: true,
            networks: vec!["192.168.10.7/24".into()],
            ..Default::default()
        };
        let policy = s.set_policy("one", true, None, desktop).unwrap();
        assert_eq!(policy.revision, revision);
        assert!(policy.desktop.x11);
        assert!(s.verify("one", &grant));
        assert!(!changes.has_changed().unwrap());
        // Sharing requires strict isolation.
        let host = DesktopSettings {
            isolation: Isolation::Host,
            ..Default::default()
        };
        assert!(s.set_policy("one", true, None, host.clone()).is_err());
        s.set_policy("one", false, None, host).unwrap();
        assert!(changes.has_changed().unwrap());
    }

    #[test]
    fn grants_are_listed_touched_and_revoked_by_public_id() {
        let s = store();
        s.set_policy(
            "one",
            true,
            Some("password-one"),
            DesktopSettings::default(),
        )
        .unwrap();
        let token = s.authenticate("one", "password-one", &client()).unwrap();
        let grants = s.grants("one").unwrap();
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0].client, "a-node");
        assert_eq!(grants[0].label, "A");
        assert!(!token.starts_with(&grants[0].id));
        assert_eq!(s.grant("one", &token).unwrap().0, grants[0].id);
        s.touch("one", &token, "198.51.100.2");
        assert_eq!(s.grants("one").unwrap()[0].ip, "198.51.100.2");
        assert!(!s.revoke_grant_id("one", "not-an-id").unwrap());
        assert!(!s.revoke_grant_id("two", &grants[0].id).unwrap());
        assert!(s.revoke_grant_id("one", &grants[0].id).unwrap());
        assert!(!s.verify("one", &token));
    }

    #[test]
    fn desktop_settings_validate_paths_and_networks() {
        assert_eq!(
            parse_network("192.168.1.9/24").unwrap(),
            ("192.168.1.0".parse().unwrap(), 24)
        );
        assert!(parse_network("172.20.0.5").is_ok());
        for bad in [
            "127.0.0.0/8",
            "10.0.0.0/8",
            "10.0.2.0/24",
            "10.0.0.0/16",
            "169.254.0.0/16",
            "0.0.0.0/0",
            "224.0.0.0/4",
            "192.168.1.0/7",
            "nonsense",
        ] {
            assert!(parse_network(bad).is_err(), "{bad}");
        }
        let mount = |path: &str| DesktopSettings {
            mounts: vec![Mount {
                path: path.into(),
                writable: false,
            }],
            ..Default::default()
        };
        assert!(mount("/data/projects").validate().is_ok());
        for bad in [
            "relative",
            "/data/../etc",
            "/data/./x",
            "/",
            "/data/",
            "/a//b",
        ] {
            assert!(mount(bad).validate().is_err(), "{bad}");
        }
        let mut host = mount("/data");
        host.isolation = Isolation::Host;
        assert!(host.validate().is_err());
        // Settings saved before these fields existed still load.
        let old: DesktopSettings = serde_json::from_str(r#"{"x11":true,"services":[]}"#).unwrap();
        assert_eq!(old.isolation, Isolation::Strict);
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
