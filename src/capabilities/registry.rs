use super::cli_describe::{describe_cli_app, raw_cli_tool};
use super::types::{
    CapabilityApp, CapabilityDiagnostic, CapabilitySkill, CapabilitySnapshot, CapabilityTool,
    SkillSource,
};
use crate::apps::api::AppsState;
use crate::apps::app::{AppStatus, AppType, ManagedApp};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MAX_SKILL_FILE_BYTES: u64 = 32 * 1024;

pub async fn build_snapshot(apps: &Arc<AppsState>) -> CapabilitySnapshot {
    let configured = match apps.store.list() {
        Ok(apps) => apps,
        Err(err) => {
            return CapabilitySnapshot {
                apps: Vec::new(),
                tools: Vec::new(),
                skills: Vec::new(),
                diagnostics: vec![CapabilityDiagnostic {
                    app_id: String::new(),
                    code: "app_store_list_failed".to_string(),
                    message: err,
                }],
            };
        }
    };

    let mut out_apps = Vec::new();
    let mut out_tools = Vec::new();
    let mut out_skills = Vec::new();
    let mut diagnostics = Vec::new();

    for app in configured {
        let status = match apps.app_status(&app) {
            AppStatus::Running => "running",
            AppStatus::Stopped => "stopped",
            AppStatus::Crashed => "crashed",
        }
        .to_string();
        let mut capabilities = vec![app.app_type.as_str().to_string()];
        let mut app_tools: Vec<CapabilityTool> = Vec::new();
        let mut app_skills = local_skills(&app);
        let mut app_diagnostics = Vec::new();

        if app.app_type == AppType::CliApp {
            if let Some(raw) = raw_cli_tool(&app) {
                app_tools.push(raw);
            }
            let described = describe_cli_app(&app).await;
            if !described.tools.is_empty() {
                capabilities.push("cli_describe".to_string());
                app_tools.extend(described.tools);
            }
            if !described.skills.is_empty() {
                capabilities.push("manifest_skills".to_string());
                app_skills.extend(described.skills);
            }
            app_diagnostics.extend(described.diagnostics);
        }
        if !app_skills.is_empty() && !capabilities.iter().any(|c| c == "skills") {
            capabilities.push("skills".to_string());
        }

        diagnostics.extend(app_diagnostics.clone());
        let app_summary = CapabilityApp {
            id: app.id.clone(),
            name: app.name.clone(),
            app_type: app.app_type.as_str().to_string(),
            status,
            capabilities,
            tool_count: app_tools.len(),
            skill_count: app_skills.len(),
            diagnostics: app_diagnostics,
        };
        out_tools.extend(app_tools);
        out_skills.extend(app_skills);
        out_apps.push(app_summary);
    }

    CapabilitySnapshot {
        apps: out_apps,
        tools: out_tools,
        skills: out_skills,
        diagnostics,
    }
}

fn local_skills(app: &ManagedApp) -> Vec<CapabilitySkill> {
    app.skill_paths
        .as_ref()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(idx, path)| CapabilitySkill {
            id: super::types::tool_id_for(&app.id, &format!("skill_{}", idx + 1)),
            app_id: app.id.clone(),
            name: std::path::Path::new(path)
                .file_name()
                .and_then(|v| v.to_str())
                .unwrap_or("skill")
                .to_string(),
            summary: format!("Local skill for {}", app.name),
            content: None,
            source: SkillSource::LocalPath { path: path.clone() },
        })
        .collect()
}

pub async fn read_skill_content(configured_path: &str) -> Result<String, String> {
    let path = normalize_skill_path(configured_path)?;
    let metadata = tokio::fs::metadata(&path)
        .await
        .map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_SKILL_FILE_BYTES {
        return Err(format!(
            "skill must be a file no larger than {} bytes",
            MAX_SKILL_FILE_BYTES
        ));
    }
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return Err("skill file extension must be .md or .txt".to_string());
    };
    if !matches!(ext, "md" | "txt") {
        return Err("skill file extension must be .md or .txt".to_string());
    }
    tokio::fs::read_to_string(path)
        .await
        .map_err(|e| e.to_string())
}

fn normalize_skill_path(configured_path: &str) -> Result<PathBuf, String> {
    let raw = Path::new(configured_path.trim());
    if !raw.is_absolute() {
        return Err("skill path must be absolute".to_string());
    }
    let path = if raw.is_dir() {
        raw.join("SKILL.md")
    } else {
        raw.to_path_buf()
    };
    std::fs::canonicalize(path).map_err(|e| e.to_string())
}
