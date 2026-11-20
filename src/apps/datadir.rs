use super::app::ManagedApp;
use std::path::{Path, PathBuf};

pub(crate) const BUILTIN_CHROME_ID: &str = "builtin-chrome";

/// Selective data categories offered by managed Chrome applications.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClearCategory {
    Cache,
    Cookies,
    History,
    Passwords,
    Bookmarks,
    Extensions,
    /// Web Storage (localStorage/sessionStorage). Never selected by default:
    /// extension localStorage shares these stores, so clearing them also
    /// removes extension data that cannot be restored.
    SiteStorage,
}

impl ClearCategory {
    /// Category ids in dialog order; the frontend renders them as-is.
    pub const ALL: [ClearCategory; 7] = [
        ClearCategory::Cache,
        ClearCategory::Cookies,
        ClearCategory::History,
        ClearCategory::Passwords,
        ClearCategory::Bookmarks,
        ClearCategory::Extensions,
        ClearCategory::SiteStorage,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ClearCategory::Cache => "cache",
            ClearCategory::Cookies => "cookies",
            ClearCategory::History => "history",
            ClearCategory::Passwords => "passwords",
            ClearCategory::Bookmarks => "bookmarks",
            ClearCategory::Extensions => "extensions",
            ClearCategory::SiteStorage => "site_storage",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        ClearCategory::ALL
            .into_iter()
            .find(|category| category.as_str() == value)
    }
}

/// What a clear-data request removes for a managed Chrome application.
pub struct ClearSelection {
    pub categories: Vec<ClearCategory>,
    /// Remove the whole profile and sidecar data (the legacy behavior).
    pub full_reset: bool,
}

impl ClearSelection {
    /// Default clear: everything except extensions and site storage.
    /// Both are excluded because clearing them destroys extension data that
    /// cannot be restored from the profile.
    pub fn default_chrome() -> Self {
        Self {
            categories: ClearCategory::ALL
                .into_iter()
                .filter(|category| {
                    *category != ClearCategory::Extensions
                        && *category != ClearCategory::SiteStorage
                })
                .collect(),
            full_reset: false,
        }
    }
}

/// Persistent data root for a managed app (`$IVNC_HOME/apps/<id>/data`).
#[allow(dead_code)]
pub fn data_dir(app: &ManagedApp) -> PathBuf {
    crate::paths::app_data_dir(&app.id)
}

/// App root under `$IVNC_HOME/apps/<id>` (data + home + profile + logs).
pub fn app_root(app: &ManagedApp) -> PathBuf {
    if app.id == crate::proxy_panel::BUILTIN_MIAO_ID {
        return crate::paths::host_home().join(".miao");
    }
    crate::paths::app_dir(&app.id)
}

/// Directories that hold durable app state (cleared by clear-data).
fn durable_dirs(app: &ManagedApp) -> Vec<PathBuf> {
    if app.id == crate::proxy_panel::BUILTIN_MIAO_ID {
        return vec![app_root(app)];
    }
    let mut dirs = vec![
        crate::paths::app_data_dir(&app.id),
        crate::paths::app_home(&app.id),
    ];
    if super::chrome::generated(app) {
        if let Ok(chrome) = super::chrome::resolve(app) {
            dirs.extend([chrome.profile_dir, chrome.crash_dir, chrome.temp_dir]);
        }
    }
    // Nested profile/crash/home paths must never be counted or removed twice.
    dirs.sort_by_key(|path| path.components().count());
    let mut unique = Vec::<PathBuf>::new();
    for dir in dirs {
        if !unique.iter().any(|parent| dir.starts_with(parent)) {
            unique.push(dir);
        }
    }
    unique
}

fn should_recreate_after_clear(dir: &Path) -> bool {
    // Keep the managed app layout ready; leave sidecar ~/.config/google-chrome absent.
    dir.starts_with(crate::paths::ivnc_home().join("apps"))
}

pub fn dir_size(path: &PathBuf) -> u64 {
    if !std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir()) {
        return 0;
    }
    std::fs::read_dir(path)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| {
                    let p = e.path();
                    match e.file_type() {
                        Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
                        Ok(t) if t.is_dir() => dir_size(&p),
                        _ => 0,
                    }
                })
                .sum()
        })
        .unwrap_or(0)
}

/// Total managed data size, counting nested profile/crash paths only once.
pub fn durable_size(app: &ManagedApp) -> u64 {
    durable_dirs(app).iter().map(dir_size).sum()
}

pub fn clear(app: &ManagedApp, selection: &ClearSelection) -> Result<(), super::error::ApiError> {
    if app.id == crate::proxy_panel::BUILTIN_MIAO_ID {
        return Err("Miao configuration cannot be cleared as ordinary app data".into());
    }
    let _resources = if app.chrome.is_some() {
        Some(super::chrome::cleanup_lock(app)?)
    } else {
        None
    };
    if app.chrome.is_some() && !selection.full_reset {
        return clear_chrome_selective(app, selection).map_err(Into::into);
    }
    if app.chrome.is_some() {
        let resolved = super::chrome::resolve(app)?;
        if [&resolved.profile_dir, &resolved.crash_dir]
            .iter()
            .any(|p| !p.starts_with(&resolved.app_root))
        {
            return Err(super::error::ApiError::new("chrome_external_full_reset", "Full reset is only available for directories inside this application's directory; use category cleanup for external profiles"));
        }
    }
    for dir in durable_dirs(app) {
        if dir.exists() {
            std::fs::remove_dir_all(&dir)
                .map_err(|e| format!("Failed to clear {}: {}", dir.display(), e))?;
        }
        if should_recreate_after_clear(&dir) {
            std::fs::create_dir_all(&dir)
                .map_err(|e| format!("Failed to recreate {}: {}", dir.display(), e))?;
        }
    }
    Ok(())
}

/// Remove files/directories, tolerating missing entries; failures are
/// collected so one unreadable entry does not abort the whole clear.
fn remove_entries(paths: Vec<PathBuf>, errors: &mut Vec<String>) {
    for path in paths {
        // Unlinking a selected symlink is safe; traversing a symlink in one of
        // its parents could delete data outside the configured profile.
        if path
            .ancestors()
            .skip(1)
            .any(|p| std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink()))
        {
            errors.push(format!(
                "Refusing to clear through a symbolic link: {}",
                path.display()
            ));
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        let result = if meta.is_dir() && !meta.file_type().is_symlink() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        if let Err(err) = result {
            errors.push(format!("Failed to remove {}: {}", path.display(), err));
        }
    }
}

// The category path lists below only name well-known Chrome profile entries.
// Unknown entries are always preserved by a selective clear (only a full
// reset removes everything), so a Chrome update adding new files can leave
// some data behind but can never delete too much. Whole sidecar config
// directories are deliberately NOT part of any category: they may contain the
// host user's real Chrome profile and are never part of application cleanup.

fn chrome_cache_paths(profile: &Path) -> Vec<PathBuf> {
    let default_dir = profile.join("Default");
    let service_worker = default_dir.join("Service Worker");
    let mut paths: Vec<PathBuf> = [
        "GrShaderCache",
        "GraphiteDawnCache",
        "ShaderCache",
        "component_crx_cache",
        "extensions_crx_cache",
        "BrowserMetrics",
        "BrowserMetrics-spare.pma",
        "Crash Reports",
    ]
    .iter()
    .map(|name| profile.join(name))
    .collect();
    paths.extend(
        [
            "Cache",
            "Code Cache",
            "GPUCache",
            "DawnGraphiteCache",
            "DawnWebGPUCache",
            "AutofillAiModelCache",
            "Shared Dictionary",
            "blob_storage",
            "optimization_guide_hint_cache_store",
            "Media Cache",
        ]
        .iter()
        .map(|name| default_dir.join(name)),
    );
    // Site caches only; registrations stay so installed PWAs keep working.
    paths.push(service_worker.join("CacheStorage"));
    paths.push(service_worker.join("ScriptCache"));
    paths
}

fn chrome_cookie_paths(default_dir: &Path) -> Vec<PathBuf> {
    [
        "Cookies",
        "Cookies-journal",
        "Safe Browsing Cookies",
        "Safe Browsing Cookies-journal",
        "Trust Tokens",
        "Trust Tokens-journal",
        "DIPS",
        "DIPS-wal",
        "DIPS-journal",
        "SharedStorage",
        "SharedStorage-wal",
        "SharedStorage-journal",
        "BrowsingTopicsSiteData",
        "BrowsingTopicsSiteData-journal",
        "BrowsingTopicsState",
        "Network Persistent State",
        "TransportSecurity",
        "Reporting and NEL",
        "Reporting and NEL-journal",
    ]
    .iter()
    .map(|name| default_dir.join(name))
    .collect()
}

/// Web Storage holds both site and extension localStorage under one LevelDB,
/// keyed by origin. Removing it also destroys extension data that cannot be
/// restored, so this category is never selected by default.
fn chrome_site_storage_paths(default_dir: &Path) -> Vec<PathBuf> {
    ["Local Storage", "Session Storage", "WebStorage"]
        .iter()
        .map(|name| default_dir.join(name))
        .collect()
}

fn chrome_history_paths(default_dir: &Path) -> Vec<PathBuf> {
    [
        "History",
        "History-journal",
        "Top Sites",
        "Top Sites-journal",
        "Favicons",
        "Favicons-journal",
        "Visited Links",
        "Network Action Predictor",
        "Network Action Predictor-journal",
        "Shortcuts",
        "Shortcuts-journal",
        "Site Characteristics Database",
        "Sessions",
        "Download Service",
        "chrome_cart_db",
        "commerce_subscription_db",
        "discount_infos_db",
        "discounts_db",
        "parcel_tracking_db",
    ]
    .iter()
    .map(|name| default_dir.join(name))
    .collect()
}

fn chrome_password_paths(default_dir: &Path) -> Vec<PathBuf> {
    [
        "Login Data",
        "Login Data-journal",
        "Login Data For Account",
        "Login Data For Account-journal",
        "Web Data",
        "Web Data-journal",
        "Account Web Data",
        "Account Web Data-journal",
        "Affiliation Database",
        "Affiliation Database-journal",
        "AutofillStrikeDatabase",
        "passkey_enclave_state",
        "trusted_vault.pb",
    ]
    .iter()
    .map(|name| default_dir.join(name))
    .collect()
}

fn chrome_bookmark_paths(default_dir: &Path) -> Vec<PathBuf> {
    [
        "Bookmarks",
        "Bookmarks.bak",
        "BookmarkMergedSurfaceOrdering",
    ]
    .iter()
    .map(|name| default_dir.join(name))
    .collect()
}

fn chrome_extension_paths(default_dir: &Path) -> Vec<PathBuf> {
    [
        "Extensions",
        "Local Extension Settings",
        "Sync Extension Settings",
        "Managed Extension Settings",
        "Extension Rules",
        "Extension Scripts",
        "Extension State",
    ]
    .iter()
    .map(|name| default_dir.join(name))
    .collect()
}

/// Extension IndexedDB entries use a chrome-extension_ name prefix, so a
/// site-data clear can keep them while an extension clear removes only them.
fn clear_indexeddb(default_dir: &Path, only_extensions: bool, errors: &mut Vec<String>) {
    let dir = default_dir.join("IndexedDB");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let paths = entries
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .map(|name| name.starts_with("chrome-extension_") == only_extensions)
                .unwrap_or(false)
        })
        .map(|entry| entry.path())
        .collect();
    remove_entries(paths, errors);
}

/// Extension install records live in Preferences under the extensions key and
/// are integrity-protected by Secure Preferences MACs. Drop the records and
/// the MAC store together so Chrome rebuilds them on next launch. When
/// Preferences is not readable JSON, fall back to deleting it as well so
/// Chrome regenerates a consistent pair.
fn scrub_extension_registration(profile: &Path, errors: &mut Vec<String>) {
    let default_dir = profile.join("Default");
    let prefs = default_dir.join("Preferences");
    let secure = default_dir.join("Secure Preferences");
    match std::fs::read_to_string(&prefs) {
        Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
            Ok(mut value) => {
                if let Some(obj) = value.as_object_mut() {
                    obj.remove("extensions");
                }
                match serde_json::to_string(&value) {
                    Ok(out) => {
                        if let Err(err) = std::fs::write(&prefs, out) {
                            errors.push(format!("Failed to write {}: {}", prefs.display(), err));
                        }
                    }
                    Err(err) => {
                        errors.push(format!("Failed to serialize {}: {}", prefs.display(), err))
                    }
                }
            }
            Err(err) => {
                log::warn!(
                    "Chrome Preferences is not valid JSON ({}); removing it too",
                    err
                );
                remove_entries(vec![prefs], errors);
            }
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => errors.push(format!("Failed to read {}: {}", prefs.display(), err)),
    }
    remove_entries(vec![secure], errors);
}

/// Selective Chrome clear: removes only the requested categories. Extension
/// registration state (Preferences and Secure Preferences) is always kept
/// unless the extensions category is selected, so surviving extensions never
/// trip Chrome tamper detection. Local State holds browser-wide profile
/// metadata rather than extension registrations and is always preserved.
fn clear_chrome_selective(app: &ManagedApp, selection: &ClearSelection) -> Result<(), String> {
    let resolved = super::chrome::resolve(app).map_err(String::from)?;
    let profile = resolved.profile_dir;
    let default_dir = profile.join("Default");
    for path in [
        &default_dir,
        &default_dir.join("Preferences"),
        &default_dir.join("IndexedDB"),
    ] {
        if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(format!(
                "Refusing to clear a linked Chrome profile entry: {}",
                path.display()
            ));
        }
    }
    let mut errors: Vec<String> = Vec::new();

    // Runtime lock artifacts are transient and always safe to drop.
    remove_entries(
        [
            "SingletonLock",
            "SingletonSocket",
            "SingletonCookie",
            "DevToolsActivePort",
        ]
        .iter()
        .map(|name| profile.join(name))
        .collect(),
        &mut errors,
    );
    remove_entries(vec![resolved.temp_dir], &mut errors);

    for category in &selection.categories {
        match category {
            ClearCategory::Cache => {
                let mut paths = chrome_cache_paths(&profile);
                paths.push(crate::paths::app_home(&app.id).join(".cache"));
                if resolved.crash_dir.starts_with(&resolved.app_root) {
                    paths.push(resolved.crash_dir.clone());
                } else {
                    paths.extend(
                        ["pending", "completed", "attachments"]
                            .map(|name| resolved.crash_dir.join(name)),
                    );
                }
                remove_entries(paths, &mut errors);
            }
            ClearCategory::Cookies => {
                remove_entries(chrome_cookie_paths(&default_dir), &mut errors);
                clear_indexeddb(&default_dir, false, &mut errors);
            }
            ClearCategory::History => {
                remove_entries(chrome_history_paths(&default_dir), &mut errors)
            }
            ClearCategory::Passwords => {
                remove_entries(chrome_password_paths(&default_dir), &mut errors)
            }
            ClearCategory::Bookmarks => {
                remove_entries(chrome_bookmark_paths(&default_dir), &mut errors)
            }
            ClearCategory::Extensions => {
                remove_entries(chrome_extension_paths(&default_dir), &mut errors);
                clear_indexeddb(&default_dir, true, &mut errors);
                scrub_extension_registration(&profile, &mut errors);
            }
            ClearCategory::SiteStorage => {
                remove_entries(chrome_site_storage_paths(&default_dir), &mut errors);
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

pub fn size_human(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::app::AppType;

    fn chrome_app() -> ManagedApp {
        ManagedApp {
            id: BUILTIN_CHROME_ID.to_string(),
            name: "Chrome".to_string(),
            app_type: AppType::DesktopApp,
            autostart: false,
            url: None,
            launch_command: None,
            launch_env_vars: None,
            launch_cwd: None,
            launch_wait_timeout_secs: None,
            exec_command: None,
            env_vars: None,
            desktop_launch: Default::default(),
            independent_runtime: true,
            chrome: Some(super::super::chrome::ChromeConfig::builtin()),
            proxy_url: None,
            cli_binary_path: None,
            cli_env_vars: None,
            created_at: String::new(),
            sort_order: 0,
            name_revision: 0,
        }
    }

    #[test]
    fn chrome_durable_dirs_are_scoped_and_deduplicated() {
        let app = chrome_app();
        let root = crate::paths::app_dir(&app.id);
        let dirs = durable_dirs(&app);
        assert!(dirs.iter().any(|p| p.ends_with("profile")));
        let temp = super::super::chrome::resolve(&app).unwrap().temp_dir;
        assert!(dirs.contains(&temp));
        assert!(dirs.iter().all(|p| p.starts_with(&root) || p == &temp));
        assert!(!dirs.iter().any(|p| p.ends_with("Crash Reports")));
    }

    // Environment variables are process-wide. Like paths::tests, run the
    // filesystem fixtures in a subprocess so concurrent tests never observe
    // a temporary IVNC_HOME/HOME, and the full-reset case never touches the
    // real host sidecar directories.
    fn isolated_home(test_name: &str) -> Option<PathBuf> {
        if let Some(root) = std::env::var_os("IVNC_DATADIR_TEST_CHILD") {
            return Some(PathBuf::from(root));
        }
        let tmp = std::env::temp_dir().join(format!("ivnc-datadir-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(tmp.join("home")).unwrap();
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test_name, "--test-threads=1"])
            .env("IVNC_DATADIR_TEST_CHILD", &tmp)
            .env("HOME", tmp.join("home"))
            .env("IVNC_HOME", tmp.join("ivnc-home"))
            .output();
        let _ = std::fs::remove_dir_all(&tmp);
        let output = result.expect("run isolated datadir test");
        assert!(
            output.status.success(),
            "isolated datadir test failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        None
    }

    fn test_profile(root: &Path) -> PathBuf {
        root.join("ivnc-home")
            .join("apps")
            .join(BUILTIN_CHROME_ID)
            .join("profile")
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"data").unwrap();
    }

    fn seed_chrome_profile(profile: &Path) {
        let default_dir = profile.join("Default");
        let service_worker = default_dir.join("Service Worker");
        for dir in [
            profile.join("GrShaderCache"),
            profile.join("component_crx_cache"),
            default_dir.join("Cache"),
            default_dir.join("Local Storage"),
            default_dir.join("Session Storage"),
            default_dir.join("WebStorage"),
            default_dir.join("Extensions").join("ext-id").join("1.0"),
            default_dir.join("Local Extension Settings").join("ext-id"),
            default_dir
                .join("IndexedDB")
                .join("https_example_com_0.indexeddb.leveldb"),
            default_dir
                .join("IndexedDB")
                .join("chrome-extension_ext-id_0.indexeddb.leveldb"),
            service_worker.join("CacheStorage"),
            service_worker.join("Database"),
        ] {
            std::fs::create_dir_all(dir).unwrap();
        }
        for file in [
            profile.join("Local State"),
            default_dir.join("Secure Preferences"),
            default_dir.join("Cookies"),
            default_dir.join("History"),
            default_dir.join("Login Data"),
            default_dir.join("Bookmarks"),
        ] {
            touch(&file);
        }
        std::fs::write(
            default_dir.join("Preferences"),
            serde_json::json!({"extensions": {"settings": {"ext-id": {}}}, "homepage": "about:blank"})
                .to_string(),
        )
        .unwrap();
        std::os::unix::fs::symlink(
            format!(
                "{}-2147483647",
                std::fs::read_to_string("/proc/sys/kernel/hostname")
                    .unwrap()
                    .trim()
            ),
            profile.join("SingletonLock"),
        )
        .unwrap();
        // Unknown entries must survive every selective clear.
        touch(&profile.join("FutureChromeData"));
    }

    fn clear_chrome(categories: &[ClearCategory]) -> Result<(), super::super::error::ApiError> {
        clear(
            &chrome_app(),
            &ClearSelection {
                categories: categories.to_vec(),
                full_reset: false,
            },
        )
    }

    #[test]
    fn chrome_default_clear_preserves_extensions_and_registration() {
        let Some(root) = isolated_home(
            "apps::datadir::tests::chrome_default_clear_preserves_extensions_and_registration",
        ) else {
            return;
        };
        let profile = test_profile(&root);
        seed_chrome_profile(&profile);
        let default_dir = profile.join("Default");
        clear_chrome(&ClearSelection::default_chrome().categories).unwrap();

        for removed in ["Cache", "Cookies", "History", "Login Data", "Bookmarks"] {
            assert!(
                !default_dir.join(removed).exists(),
                "{removed} should be removed"
            );
        }
        assert!(!profile.join("GrShaderCache").exists());
        assert!(!profile.join("SingletonLock").exists());
        assert!(!default_dir
            .join("IndexedDB")
            .join("https_example_com_0.indexeddb.leveldb")
            .exists());

        for kept in [
            "Extensions",
            "Local Extension Settings",
            "Preferences",
            "Secure Preferences",
            // localStorage and SW registrations are shared with extensions and
            // must survive the default clear.
            "Local Storage",
            "Service Worker",
        ] {
            assert!(default_dir.join(kept).exists(), "{kept} should be kept");
        }
        assert!(profile.join("Local State").exists());
        assert!(profile.join("FutureChromeData").exists());
        assert!(default_dir
            .join("IndexedDB")
            .join("chrome-extension_ext-id_0.indexeddb.leveldb")
            .exists());
        let prefs: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(default_dir.join("Preferences")).unwrap(),
        )
        .unwrap();
        assert!(
            prefs["extensions"].is_object(),
            "extensions stay registered"
        );
    }

    #[test]
    fn chrome_cookies_clear_keeps_web_storage_and_service_workers() {
        let Some(root) = isolated_home(
            "apps::datadir::tests::chrome_cookies_clear_keeps_web_storage_and_service_workers",
        ) else {
            return;
        };
        let profile = test_profile(&root);
        seed_chrome_profile(&profile);
        let default_dir = profile.join("Default");
        clear_chrome(&[ClearCategory::Cookies]).unwrap();

        assert!(!default_dir.join("Cookies").exists());
        for kept in [
            "Local Storage",
            "Session Storage",
            "WebStorage",
            "Service Worker",
            "IndexedDB",
            "Extensions",
        ] {
            assert!(
                default_dir.join(kept).exists(),
                "{kept} must survive a cookies-only clear"
            );
        }
    }

    #[test]
    fn chrome_site_storage_clear_removes_only_web_storage() {
        let Some(root) = isolated_home(
            "apps::datadir::tests::chrome_site_storage_clear_removes_only_web_storage",
        ) else {
            return;
        };
        let profile = test_profile(&root);
        seed_chrome_profile(&profile);
        let default_dir = profile.join("Default");
        clear_chrome(&[ClearCategory::SiteStorage]).unwrap();

        for removed in ["Local Storage", "Session Storage", "WebStorage"] {
            assert!(
                !default_dir.join(removed).exists(),
                "{removed} should be removed"
            );
        }
        for kept in [
            "Cookies",
            "Cache",
            "History",
            "IndexedDB",
            "Extensions",
            "Service Worker",
        ] {
            assert!(
                default_dir.join(kept).exists(),
                "{kept} must survive a site-storage-only clear"
            );
        }
    }

    #[test]
    fn chrome_extensions_clear_removes_registration_but_keeps_other_data() {
        let Some(root) = isolated_home(
            "apps::datadir::tests::chrome_extensions_clear_removes_registration_but_keeps_other_data",
        ) else {
            return;
        };
        let profile = test_profile(&root);
        seed_chrome_profile(&profile);
        let default_dir = profile.join("Default");
        clear_chrome(&[ClearCategory::Extensions]).unwrap();

        assert!(!default_dir.join("Extensions").exists());
        assert!(!default_dir.join("Local Extension Settings").exists());
        assert!(!default_dir.join("Secure Preferences").exists());
        assert!(!default_dir
            .join("IndexedDB")
            .join("chrome-extension_ext-id_0.indexeddb.leveldb")
            .exists());

        for kept in ["Cookies", "History", "Login Data", "Bookmarks", "Cache"] {
            assert!(default_dir.join(kept).exists(), "{kept} should be kept");
        }
        assert!(default_dir
            .join("IndexedDB")
            .join("https_example_com_0.indexeddb.leveldb")
            .exists());
        let prefs: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(default_dir.join("Preferences")).unwrap(),
        )
        .unwrap();
        assert!(prefs.get("extensions").is_none());
        assert_eq!(prefs["homepage"], "about:blank");
    }

    #[test]
    fn chrome_extensions_clear_falls_back_to_deleting_corrupt_preferences() {
        let Some(root) = isolated_home(
            "apps::datadir::tests::chrome_extensions_clear_falls_back_to_deleting_corrupt_preferences",
        ) else {
            return;
        };
        let profile = test_profile(&root);
        seed_chrome_profile(&profile);
        let default_dir = profile.join("Default");
        std::fs::write(default_dir.join("Preferences"), b"not json").unwrap();
        clear_chrome(&[ClearCategory::Extensions]).unwrap();
        assert!(!default_dir.join("Preferences").exists());
        assert!(!default_dir.join("Secure Preferences").exists());
        assert!(!default_dir.join("Extensions").exists());
    }

    #[test]
    fn chrome_full_reset_wipes_everything_and_recreates_layout() {
        let Some(root) = isolated_home(
            "apps::datadir::tests::chrome_full_reset_wipes_everything_and_recreates_layout",
        ) else {
            return;
        };
        let profile = test_profile(&root);
        seed_chrome_profile(&profile);
        clear(
            &chrome_app(),
            &ClearSelection {
                categories: Vec::new(),
                full_reset: true,
            },
        )
        .unwrap();
        let default_dir = profile.join("Default");
        assert!(!default_dir.join("Cookies").exists());
        assert!(!default_dir.join("Extensions").exists());
        assert!(!profile.join("FutureChromeData").exists());
        assert!(profile.is_dir(), "profile layout is recreated");
    }

    #[test]
    fn chrome_selective_clear_never_touches_sidecar_config_dirs() {
        let Some(root) = isolated_home(
            "apps::datadir::tests::chrome_selective_clear_never_touches_sidecar_config_dirs",
        ) else {
            return;
        };
        let profile = test_profile(&root);
        seed_chrome_profile(&profile);
        let sidecars = crate::paths::chrome_sidecar_config_dirs();
        assert_eq!(sidecars.len(), 2, "ivnc and host sidecars are both checked");
        for sidecar in &sidecars {
            touch(&sidecar.join("Preferences"));
            touch(&sidecar.join("Default").join("Bookmarks"));
        }

        // Every category selected: the sidecar roots still must survive,
        // because they can hold the host user's real Chrome profile.
        clear_chrome(&ClearCategory::ALL).unwrap();

        for sidecar in &sidecars {
            assert!(
                sidecar.join("Preferences").exists(),
                "{} must survive a selective clear",
                sidecar.display()
            );
            assert!(
                sidecar.join("Default").join("Bookmarks").exists(),
                "{} must survive a selective clear",
                sidecar.display()
            );
        }
        assert!(!test_profile(&root).join("Default").join("Cache").exists());
    }
}
