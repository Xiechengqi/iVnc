use super::cli_describe::{describe_cli_app, raw_cli_tool};
use super::types::{CapabilityApp, CapabilityDiagnostic, CapabilitySnapshot, CapabilityTool};
use crate::apps::api::AppsState;
use crate::apps::app::{AppStatus, AppType};
use std::sync::Arc;

pub async fn build_snapshot(apps: &Arc<AppsState>) -> CapabilitySnapshot {
    let configured = match apps.store.list() {
        Ok(apps) => apps,
        Err(err) => {
            return CapabilitySnapshot {
                apps: Vec::new(),
                tools: Vec::new(),
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
            app_diagnostics.extend(described.diagnostics);
        }
        diagnostics.extend(app_diagnostics.clone());
        let app_summary = CapabilityApp {
            id: app.id.clone(),
            name: app.name.clone(),
            app_type: app.app_type.as_str().to_string(),
            status,
            capabilities,
            tool_count: app_tools.len(),
            diagnostics: app_diagnostics,
        };
        out_tools.extend(app_tools);
        out_apps.push(app_summary);
    }

    CapabilitySnapshot {
        apps: out_apps,
        tools: out_tools,
        diagnostics,
    }
}
