//! Cron-style scheduler for agent runs.
//!
//! The scheduler periodically reads `console.json` (so CRUD changes hot-reload
//! within one tick), computes the next fire time for each enabled task via the
//! `cron` crate, and calls `crate::agent::launch::launch_agent_run` when a task
//! is due. Runs that overlap with another active run are *skipped* (cron
//! convention — never queued, never stacked). Missed firings during downtime
//! are *not* backfilled: on startup or after a restart, `next_fire_ms` is the
//! first scheduled time strictly after `now`.

use crate::agent::launch::{launch_agent_run, LaunchError, LaunchRequest};
use crate::agent::types::{now_ms, RunSource};
use crate::console_config;
use crate::web::SharedState;
use chrono::TimeZone;
use cron::Schedule;
use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use log::{info, warn};

/// Per-task runtime state. Lives in `SharedState::schedule_state`, never on
/// disk — it is rebuilt from `cron.upcoming(...)` on startup and on edits.
#[derive(Debug, Clone, Default)]
pub struct ScheduleRuntime {
    pub next_fire_ms: u64,
    pub last_run_ms: Option<u64>,
    pub last_run_id: Option<String>,
    pub last_outcome: Option<String>,
    pub last_skip_reason: Option<String>,
}

fn next_fire_after_ms(sched: &Schedule, after_ms: u64) -> u64 {
    let after = chrono::Local
        .timestamp_millis_opt(after_ms as i64)
        .single()
        .unwrap_or_else(chrono::Local::now);
    sched
        .after(&after)
        .next()
        .map(|dt| dt.timestamp_millis() as u64)
        .unwrap_or(u64::MAX)
}

/// Normalize a user-facing cron string for the `cron` crate.
///
/// We document 5-field POSIX cron (`min hour DoM Mon DoW`), but the `cron`
/// crate expects 6 or 7 fields with a leading seconds field. Five fields are
/// prepended with `0 ` (fire on second 0); 6+ fields are passed through so
/// users with a `cron`-crate native string still work.
pub fn normalize_cron(expr: &str) -> String {
    let trimmed = expr.trim();
    let fields = trimmed.split_whitespace().count();
    if fields == 5 {
        format!("0 {}", trimmed)
    } else {
        trimmed.to_string()
    }
}

/// One scheduler tick. Public so tests can drive it deterministically.
pub async fn tick_once(state: Arc<SharedState>) {
    let cfg = console_config::load();
    let now = now_ms();
    let mut known: HashSet<String> = HashSet::new();

    for st in cfg.schedules.iter() {
        known.insert(st.id.clone());
        if !st.enabled {
            continue;
        }
        let Ok(sched) = Schedule::from_str(&normalize_cron(&st.cron)) else {
            warn!("scheduler: invalid cron expression for schedule {} ({}), skipping", st.id, st.cron);
            continue;
        };

        let fire_now = {
            let mut map = state.schedule_state.lock().unwrap();
            let rt = map.entry(st.id.clone()).or_insert_with(|| ScheduleRuntime {
                next_fire_ms: next_fire_after_ms(&sched, now),
                ..ScheduleRuntime::default()
            });
            if now >= rt.next_fire_ms {
                rt.next_fire_ms = next_fire_after_ms(&sched, now);
                true
            } else {
                false
            }
        };

        if fire_now {
            fire(&state, st).await;
        }
    }

    // Drop runtime entries for deleted tasks so stale `last_*` don't linger.
    let mut map = state.schedule_state.lock().unwrap();
    map.retain(|id, _| known.contains(id));
}

async fn fire(state: &Arc<SharedState>, st: &console_config::ScheduledTask) {
    info!("scheduler: firing schedule {} ({})", st.id, st.name);
    let req = LaunchRequest {
        task: st.task.clone(),
        provider: st.provider.clone(),
        model: st.model.clone(),
        budget: Some(st.budget.clone()),
        options: st.options.clone(),
        replay_actions: None,
        source: Some(RunSource::Schedule {
            id: st.id.clone(),
            name: Some(st.name.clone()),
        }),
    };
    match launch_agent_run(state.clone(), req, false).await {
        Ok(report) => {
            let mut map = state.schedule_state.lock().unwrap();
            if let Some(rt) = map.get_mut(&st.id) {
                rt.last_run_ms = Some(now_ms());
                rt.last_run_id = Some(report.run_id);
                rt.last_outcome = Some("started".into());
                rt.last_skip_reason = None;
            }
        }
        Err(LaunchError::AlreadyActive(active_id)) => {
            let mut map = state.schedule_state.lock().unwrap();
            if let Some(rt) = map.get_mut(&st.id) {
                rt.last_run_ms = Some(now_ms());
                rt.last_outcome = Some("skipped".into());
                rt.last_skip_reason = Some(format!("active_run={}", active_id));
            }
        }
        Err(e) => {
            let msg = match e {
                LaunchError::ProviderUnavailable(name) => {
                    format!("provider_unavailable: {}", name)
                }
                LaunchError::InvalidRequest(m) => format!("invalid_request: {}", m),
                LaunchError::Internal(m) => format!("internal: {}", m),
                LaunchError::AlreadyActive(_) => unreachable!(),
            };
            warn!("scheduler: launch failed for schedule {}: {}", st.id, msg);
            let mut map = state.schedule_state.lock().unwrap();
            if let Some(rt) = map.get_mut(&st.id) {
                rt.last_run_ms = Some(now_ms());
                rt.last_outcome = Some("failed".into());
                rt.last_skip_reason = Some(msg);
            }
        }
    }
}

/// Reset the cached runtime for one task so the next tick recomputes
/// `next_fire_ms`. Call this from the PUT/DELETE handlers.
pub fn invalidate(state: &Arc<SharedState>, schedule_id: &str) {
    let mut map = state.schedule_state.lock().unwrap();
    map.remove(schedule_id);
}

/// Spawned at startup; runs until the process exits.
pub async fn run_loop(state: Arc<SharedState>) {
    let mut tick = tokio::time::interval(Duration::from_secs(30));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    info!("scheduler: loop started (30s tick)");
    loop {
        tick.tick().await;
        tick_once(state.clone()).await;
    }
}
