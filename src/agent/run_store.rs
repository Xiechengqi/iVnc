use super::types::{FinishReason, RunReport};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct RunStore {
    inner: Arc<Mutex<HashMap<String, RunReport>>>,
}

impl RunStore {
    pub fn insert(&self, report: RunReport) {
        self.inner
            .lock()
            .unwrap()
            .insert(report.run_id.clone(), report);
    }

    pub fn update(&self, report: RunReport) {
        self.insert(report);
    }

    pub fn get(&self, run_id: &str) -> Option<RunReport> {
        self.inner.lock().unwrap().get(run_id).cloned()
    }

    pub fn list(&self) -> Vec<RunReport> {
        let mut runs: Vec<_> = self.inner.lock().unwrap().values().cloned().collect();
        runs.sort_by(|a, b| b.wall_ms.cmp(&a.wall_ms));
        runs
    }

    pub fn running_run_id(&self) -> Option<String> {
        self.inner
            .lock()
            .unwrap()
            .values()
            .find(|report| matches!(report.finish_reason, FinishReason::Running))
            .map(|report| report.run_id.clone())
    }

    pub fn mark_interrupted(&self, run_id: Option<&str>) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(run_id) = run_id {
            if let Some(report) = inner.get_mut(run_id) {
                if matches!(report.finish_reason, FinishReason::Running) {
                    report.finish_reason = FinishReason::Interrupted;
                }
            }
        } else {
            for report in inner.values_mut() {
                if matches!(report.finish_reason, FinishReason::Running) {
                    report.finish_reason = FinishReason::Interrupted;
                }
            }
        }
    }

    pub fn is_terminal(&self, run_id: &str) -> bool {
        self.inner
            .lock()
            .unwrap()
            .get(run_id)
            .map(|r| !matches!(r.finish_reason, FinishReason::Running))
            .unwrap_or(false)
    }
}
