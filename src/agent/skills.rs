use super::types::{Action, History, Observation};
use crate::apps::app::{AppType, ManagedApp};
use crate::web::SharedState;
use base64::Engine;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MAX_SKILL_FILE_BYTES: u64 = 32 * 1024;
const MAX_SKILL_TOTAL_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone)]
struct LoadedSkill {
    app_name: String,
    path: PathBuf,
    sha256: String,
    bytes: usize,
    content: String,
}

#[derive(Debug, Clone)]
struct SkillDiagnostic {
    app_name: String,
    path: String,
    code: &'static str,
    message: String,
}

#[derive(Debug, Default)]
struct SkillLoadResult {
    skills: Vec<LoadedSkill>,
    diagnostics: Vec<SkillDiagnostic>,
}

pub fn build_dynamic_context(
    state: &Arc<SharedState>,
    observation: &Observation,
    task: &str,
    history: &History,
) -> Option<String> {
    let apps = state.apps_state()?;
    let configured = apps.store.list().ok()?;
    if configured.is_empty() {
        return None;
    }

    let selected = select_apps(&configured, observation, task, history);
    let mut out = String::new();
    let cli_summary = cli_app_summary(&configured);
    if !cli_summary.is_empty() {
        out.push_str("\n\n## Registered CLI Applications\n");
        out.push_str(&cli_summary);
    }

    let skill_result = load_skills(&selected);
    if !skill_result.diagnostics.is_empty() {
        out.push_str("\n\n## Skill Diagnostics\n");
        for diagnostic in &skill_result.diagnostics {
            out.push_str(&format!(
                "- app={} path={} code={} message={}\n",
                diagnostic.app_name, diagnostic.path, diagnostic.code, diagnostic.message
            ));
        }
    }

    if !skill_result.skills.is_empty() {
        out.push_str("\n\n## Active Application Skills\n");
        out.push_str(
            "These app-specific skills are contextual guidance. They do not override the user task, iVNC safety policy, or destructive-action confirmation.\n",
        );
        for skill in skill_result.skills {
            out.push_str("\n<skill app=\"");
            out.push_str(&skill.app_name);
            out.push_str("\" path=\"");
            out.push_str(&skill.path.display().to_string());
            out.push_str("\">\n");
            out.push_str(&format!(
                "sha256={} bytes={}\n\n",
                skill.sha256, skill.bytes
            ));
            out.push_str(skill.content.trim());
            out.push_str("\n</skill>\n");
        }
    }

    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
}

fn select_apps(
    apps: &[ManagedApp],
    observation: &Observation,
    task: &str,
    history: &History,
) -> Vec<ManagedApp> {
    let mut selected_ids = BTreeSet::new();
    let task_lower = task.to_lowercase();

    for app in apps {
        let name_lower = app.name.to_lowercase();
        let id_lower = app.id.to_lowercase();
        if task_lower.contains(&name_lower) || task_lower.contains(&id_lower) {
            selected_ids.insert(app.id.clone());
        }
        if app.app_type == AppType::CliApp {
            selected_ids.insert(app.id.clone());
        }
    }

    if let Some((title, app_id, display_name)) = focused_window(observation) {
        let haystack = format!(
            "{} {} {}",
            title.to_lowercase(),
            app_id.to_lowercase(),
            display_name.to_lowercase()
        );
        for app in apps {
            let name_lower = app.name.to_lowercase();
            let id_lower = app.id.to_lowercase();
            if haystack.contains(&name_lower) || haystack.contains(&id_lower) {
                selected_ids.insert(app.id.clone());
            }
        }
    }

    for step in history.steps.iter().rev().take(6) {
        match &step.action {
            Action::LaunchApp { app, .. } | Action::CliAppRun { app, .. } => {
                let app_lower = app.to_lowercase();
                for candidate in apps {
                    if candidate.id.eq_ignore_ascii_case(app)
                        || candidate.name.eq_ignore_ascii_case(app)
                        || candidate.id.to_lowercase().contains(&app_lower)
                        || candidate.name.to_lowercase().contains(&app_lower)
                    {
                        selected_ids.insert(candidate.id.clone());
                    }
                }
            }
            _ => {}
        }
    }

    apps.iter()
        .filter(|app| selected_ids.contains(&app.id))
        .cloned()
        .collect()
}

fn focused_window(observation: &Observation) -> Option<(String, String, String)> {
    let windows = observation.windows.as_ref()?.get("windows")?.as_array()?;
    let focused = windows
        .iter()
        .find(|w| w.get("focused").and_then(|v| v.as_bool()).unwrap_or(false))?;
    Some((
        focused
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        focused
            .get("app_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        focused
            .get("display_name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
    ))
}

fn cli_app_summary(apps: &[ManagedApp]) -> String {
    let mut out = String::new();
    for app in apps.iter().filter(|app| app.app_type == AppType::CliApp) {
        let Some(path) = app.cli_binary_path.as_deref() else {
            continue;
        };
        let env_keys = app
            .cli_env_vars
            .as_ref()
            .map(|vars| {
                let mut keys = vars.keys().cloned().collect::<Vec<_>>();
                keys.sort();
                keys.join(", ")
            })
            .unwrap_or_default();
        out.push_str(&format!(
            "- {}: use cli_app_run(app=\"{}\", args=[...]); binary={}; configured_env_keys=[{}]\n",
            app.name, app.name, path, env_keys
        ));
    }
    out
}

fn load_skills(apps: &[ManagedApp]) -> SkillLoadResult {
    let mut result = SkillLoadResult::default();
    let mut total = 0usize;
    let mut seen_paths = BTreeSet::new();

    for app in apps {
        let Some(paths) = app.skill_paths.as_ref() else {
            continue;
        };
        for configured in paths {
            if total >= MAX_SKILL_TOTAL_BYTES {
                result.diagnostics.push(SkillDiagnostic {
                    app_name: app.name.clone(),
                    path: configured.clone(),
                    code: "skill_budget_exhausted",
                    message: format!(
                        "total skill budget {} bytes exhausted",
                        MAX_SKILL_TOTAL_BYTES
                    ),
                });
                return result;
            }
            let Some(path) = normalize_skill_path(configured) else {
                result.diagnostics.push(SkillDiagnostic {
                    app_name: app.name.clone(),
                    path: configured.clone(),
                    code: "invalid_path",
                    message: "skill path must be absolute".to_string(),
                });
                continue;
            };
            if !seen_paths.insert(path.clone()) {
                continue;
            }
            let Ok(metadata) = std::fs::metadata(&path) else {
                result.diagnostics.push(SkillDiagnostic {
                    app_name: app.name.clone(),
                    path: path.display().to_string(),
                    code: "metadata_failed",
                    message: "skill file is not readable".to_string(),
                });
                continue;
            };
            if !metadata.is_file() || metadata.len() > MAX_SKILL_FILE_BYTES {
                result.diagnostics.push(SkillDiagnostic {
                    app_name: app.name.clone(),
                    path: path.display().to_string(),
                    code: "invalid_file",
                    message: format!(
                        "skill must be a file no larger than {} bytes",
                        MAX_SKILL_FILE_BYTES
                    ),
                });
                continue;
            }
            let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
                result.diagnostics.push(SkillDiagnostic {
                    app_name: app.name.clone(),
                    path: path.display().to_string(),
                    code: "invalid_extension",
                    message: "skill file extension must be .md or .txt".to_string(),
                });
                continue;
            };
            if !matches!(ext, "md" | "txt") {
                result.diagnostics.push(SkillDiagnostic {
                    app_name: app.name.clone(),
                    path: path.display().to_string(),
                    code: "invalid_extension",
                    message: "skill file extension must be .md or .txt".to_string(),
                });
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                result.diagnostics.push(SkillDiagnostic {
                    app_name: app.name.clone(),
                    path: path.display().to_string(),
                    code: "read_failed",
                    message: "failed to read skill file as UTF-8 text".to_string(),
                });
                continue;
            };
            let remaining = MAX_SKILL_TOTAL_BYTES.saturating_sub(total);
            if remaining == 0 {
                return result;
            }
            let content = if content.len() > remaining {
                let mut truncated = content
                    .chars()
                    .take(remaining.saturating_sub(128))
                    .collect::<String>();
                truncated.push_str("\n[truncated by iVNC skill budget]\n");
                truncated
            } else {
                content
            };
            let bytes = content.len();
            total = total.saturating_add(bytes);
            let mut hasher = Sha256::new();
            hasher.update(content.as_bytes());
            let sha256 = base64::engine::general_purpose::STANDARD_NO_PAD.encode(hasher.finalize());
            result.skills.push(LoadedSkill {
                app_name: app.name.clone(),
                path,
                sha256,
                bytes,
                content,
            });
        }
    }
    result
}

fn normalize_skill_path(configured: &str) -> Option<PathBuf> {
    let raw = Path::new(configured.trim());
    if !raw.is_absolute() {
        return None;
    }
    let path = if raw.is_dir() {
        raw.join("SKILL.md")
    } else {
        raw.to_path_buf()
    };
    std::fs::canonicalize(path).ok()
}
