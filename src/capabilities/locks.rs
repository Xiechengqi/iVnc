use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const LEASE_TTL: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Serialize)]
pub struct ResourceBusy {
    pub resource: String,
    pub holder: String,
    pub retry_after_ms: u64,
}

#[derive(Debug)]
struct LockLease {
    holder: String,
    expires_at: Instant,
}

#[derive(Debug)]
pub struct ResourceGuard {
    resources: Vec<String>,
    holder: String,
}

impl Drop for ResourceGuard {
    fn drop(&mut self) {
        let mut locks = locks().lock().unwrap();
        for resource in &self.resources {
            if locks
                .get(resource)
                .map(|lease| lease.holder == self.holder)
                .unwrap_or(false)
            {
                locks.remove(resource);
            }
        }
    }
}

static LOCKS: OnceLock<Mutex<HashMap<String, LockLease>>> = OnceLock::new();

fn locks() -> &'static Mutex<HashMap<String, LockLease>> {
    LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn acquire(resources: &[String], holder: &str) -> Result<ResourceGuard, ResourceBusy> {
    if resources.is_empty() {
        return Ok(ResourceGuard {
            resources: Vec::new(),
            holder: holder.to_string(),
        });
    }
    let now = Instant::now();
    let mut locks = locks().lock().unwrap();
    locks.retain(|_, lease| lease.expires_at > now);
    for resource in resources {
        if let Some(lease) = locks.get(resource) {
            if lease.holder != holder {
                return Err(ResourceBusy {
                    resource: resource.clone(),
                    holder: lease.holder.clone(),
                    retry_after_ms: lease
                        .expires_at
                        .saturating_duration_since(now)
                        .as_millis()
                        .min(30_000) as u64,
                });
            }
        }
    }
    let expires_at = now + LEASE_TTL;
    for resource in resources {
        locks.insert(
            resource.clone(),
            LockLease {
                holder: holder.to_string(),
                expires_at,
            },
        );
    }
    Ok(ResourceGuard {
        resources: resources.to_vec(),
        holder: holder.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusive_resource_rejects_second_holder() {
        let resource = format!("test-resource-{}", uuid::Uuid::new_v4());
        let _guard = acquire(&[resource.clone()], "holder-a").unwrap();
        let busy = acquire(&[resource.clone()], "holder-b").unwrap_err();
        assert_eq!(busy.resource, resource);
        assert_eq!(busy.holder, "holder-a");
    }

    #[test]
    fn dropping_guard_releases_resource() {
        let resource = format!("test-resource-{}", uuid::Uuid::new_v4());
        {
            let _guard = acquire(&[resource.clone()], "holder-a").unwrap();
        }
        assert!(acquire(&[resource], "holder-b").is_ok());
    }
}
