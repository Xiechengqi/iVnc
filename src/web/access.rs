//! Durable, fail-closed IP login policy and explicitly trusted proxy resolution.

use axum::http::HeaderMap;
use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    io::{Read, Write},
    net::IpAddr,
    path::Path,
};

const MAX_RULES: usize = 256;
const MAX_FILE_BYTES: u64 = 32 * 1024;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AccessConfig {
    pub enabled: bool,
    pub ip_allowlist: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ValidationError {
    pub error: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (line {:?})", self.error, self.line)
    }
}

pub fn normalize_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
        _ => ip,
    }
}

fn parse_network(value: &str) -> Result<IpNet, ()> {
    if let Ok(ip) = value.parse::<IpAddr>() {
        return Ok(IpNet::from(normalize_ip(ip)));
    }
    let net: IpNet = value.parse().map_err(|_| ())?;
    if let IpNet::V6(v6) = net {
        if let Some(v4) = v6.addr().to_ipv4_mapped() {
            if v6.prefix_len() >= 96 {
                return IpNet::new(IpAddr::V4(v4), v6.prefix_len() - 96)
                    .map(|n| n.trunc())
                    .map_err(|_| ());
            }
        }
    }
    Ok(net.trunc())
}

#[derive(Default)]
pub struct AccessPolicy {
    pub config: AccessConfig,
    networks: Vec<IpNet>,
}

impl AccessPolicy {
    pub fn parse(config: AccessConfig) -> Result<Self, ValidationError> {
        let mut seen = HashSet::new();
        let mut networks = Vec::new();
        // Keep input indices until after validation so blank lines do not shift errors.
        for (index, text) in config.ip_allowlist.iter().enumerate() {
            let value = text.trim();
            if value.is_empty() {
                continue;
            }
            let network = parse_network(value).map_err(|_| ValidationError {
                error: "invalid_ip_rule",
                line: Some(index + 1),
            })?;
            if seen.insert(network) {
                networks.push(network);
            }
            if networks.len() > MAX_RULES {
                return Err(ValidationError {
                    error: "too_many_ip_rules",
                    line: Some(index + 1),
                });
            }
        }
        if config.enabled && networks.is_empty() {
            return Err(ValidationError {
                error: "ip_rules_required",
                line: None,
            });
        }
        Ok(Self {
            config: AccessConfig {
                enabled: config.enabled,
                ip_allowlist: networks
                    .iter()
                    .map(|n| {
                        if n.prefix_len() == if n.addr().is_ipv4() { 32 } else { 128 } {
                            n.addr().to_string()
                        } else {
                            n.to_string()
                        }
                    })
                    .collect(),
            },
            networks,
        })
    }

    pub fn covers(&self, ip: IpAddr) -> bool {
        let ip = normalize_ip(ip);
        self.networks.iter().any(|n| n.contains(&ip))
    }

    pub fn matches(&self, ip: Option<IpAddr>) -> bool {
        self.config.enabled && ip.is_some_and(|ip| self.covers(ip))
    }

    pub fn load(path: &Path) -> Self {
        let result = (|| -> Result<Self, String> {
            let file = match std::fs::File::open(path) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(Self::default());
                }
                Err(error) => return Err(error.to_string()),
            };
            let mut bytes = Vec::new();
            file.take(MAX_FILE_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if bytes.len() as u64 > MAX_FILE_BYTES {
                return Err("access settings file is too large".into());
            }
            let config = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            Self::parse(config).map_err(|e| e.to_string())
        })();
        result.unwrap_or_else(|error| {
            log::warn!("IP login disabled: cannot load {}: {error}", path.display());
            Self::default()
        })
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        use std::os::unix::fs::OpenOptionsExt;
        let parent = path.parent().ok_or("Invalid access settings path")?;
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let temp = parent.join(format!(".access-{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| {
            let bytes = serde_json::to_vec_pretty(&self.config).map_err(|e| e.to_string())?;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temp)
                .map_err(|e| e.to_string())?;
            file.write_all(&bytes).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            std::fs::rename(&temp, path).map_err(|e| e.to_string())
        })();
        let _ = std::fs::remove_file(temp);
        result
    }
}

pub struct TrustedProxies {
    // An invalid server configuration must never fall back to trusting a proxy's IP.
    networks: Option<Vec<IpNet>>,
}

impl TrustedProxies {
    pub fn new(values: &[String]) -> Self {
        let networks: Result<Vec<IpNet>, ()> =
            values.iter().map(|v| parse_network(v.trim())).collect();
        if networks.is_err() {
            log::warn!("Invalid http.trusted_proxy_cidrs; IP login is disabled");
        }
        Self {
            networks: networks.ok(),
        }
    }

    pub fn client_ip(&self, peer: IpAddr, headers: &HeaderMap) -> Option<IpAddr> {
        let networks = self.networks.as_ref()?;
        let trusted = |ip: &IpAddr| networks.iter().any(|net| net.contains(ip));
        let mut current = normalize_ip(peer);
        if !trusted(&current) {
            return Some(current);
        }
        let mut values = headers.get_all("x-forwarded-for").iter();
        let value = values.next()?.to_str().ok()?;
        if values.next().is_some() || value.len() > 2048 {
            return None;
        }
        let hops: Vec<_> = value.split(',').collect();
        if hops.len() > 32 {
            return None;
        }
        // Validate the entire value before selecting the first untrusted hop.
        let hops: Option<Vec<_>> = hops
            .into_iter()
            .map(|hop| hop.trim().parse::<IpAddr>().ok().map(normalize_ip))
            .collect();
        for hop in hops?.into_iter().rev() {
            if !trusted(&current) {
                break;
            }
            current = hop;
        }
        Some(current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(lines: &[&str]) -> AccessPolicy {
        AccessPolicy::parse(AccessConfig {
            enabled: true,
            ip_allowlist: lines.iter().map(|s| s.to_string()).collect(),
        })
        .unwrap()
    }

    #[test]
    fn rules_normalize_and_match_network_boundaries_and_mapped_ipv4() {
        let p = policy(&[
            " 192.168.1.19/24 ",
            "192.168.1.0/24",
            "",
            "::ffff:10.0.0.19/120",
            "2001:db8::19/64",
            "2001:db9::1",
        ]);
        assert_eq!(p.config.ip_allowlist.len(), 4);
        assert_eq!(p.config.ip_allowlist[0], "192.168.1.0/24");
        for ip in [
            "192.168.1.0",
            "192.168.1.255",
            "::ffff:192.168.1.20",
            "10.0.0.255",
            "2001:db8::ffff",
            "2001:db9::1",
        ] {
            assert!(p.matches(Some(ip.parse().unwrap())), "{ip}");
        }
        for ip in [
            "192.168.0.255",
            "192.168.2.0",
            "10.0.1.0",
            "2001:db8:0:1::1",
            "2001:db9::2",
        ] {
            assert!(!p.matches(Some(ip.parse().unwrap())), "{ip}");
        }
        assert!(!p.matches(None));
    }

    #[test]
    fn invalid_rules_reject_the_entire_policy_and_preserve_line_numbers() {
        for rule in [
            "192.168.1.0/33",
            "2001:db8::/129",
            "localhost",
            "192.168.1.*",
            "192.168.1.1:8008",
        ] {
            let error = AccessPolicy::parse(AccessConfig {
                enabled: true,
                ip_allowlist: vec!["127.0.0.1".into(), "".into(), rule.into()],
            })
            .err()
            .unwrap();
            assert_eq!(error.error, "invalid_ip_rule");
            assert_eq!(error.line, Some(3));
        }
        assert!(AccessPolicy::parse(AccessConfig {
            enabled: true,
            ..Default::default()
        })
        .is_err());
        assert!(!AccessPolicy::default().matches(Some("127.0.0.1".parse().unwrap())));
    }

    #[test]
    fn only_explicitly_trusted_proxies_can_supply_a_valid_bounded_chain() {
        let proxies = TrustedProxies::new(&["127.0.0.1".into(), "10.0.0.0/24".into()]);
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "192.168.1.50, 10.0.0.2".parse().unwrap());
        assert_eq!(
            proxies.client_ip("127.0.0.1".parse().unwrap(), &headers),
            Some("192.168.1.50".parse().unwrap())
        );
        // An untrusted intermediate hop terminates trust, including forged left entries.
        headers.insert(
            "x-forwarded-for",
            "192.168.1.50, 172.16.0.2".parse().unwrap(),
        );
        assert_eq!(
            proxies.client_ip("127.0.0.1".parse().unwrap(), &headers),
            Some("172.16.0.2".parse().unwrap())
        );
        assert_eq!(
            proxies.client_ip("192.0.2.1".parse().unwrap(), &headers),
            Some("192.0.2.1".parse().unwrap())
        );
        assert_eq!(
            proxies.client_ip("127.0.0.1".parse().unwrap(), &HeaderMap::new()),
            None
        );
        for value in ["", "garbage", "192.168.1.50,", "192.168.1.50:80"] {
            headers.insert("x-forwarded-for", value.parse().unwrap());
            assert_eq!(
                proxies.client_ip("127.0.0.1".parse().unwrap(), &headers),
                None
            );
        }
        headers.insert("x-forwarded-for", "192.168.1.50".parse().unwrap());
        headers.append("x-forwarded-for", "10.0.0.2".parse().unwrap());
        assert_eq!(
            proxies.client_ip("127.0.0.1".parse().unwrap(), &headers),
            None
        );
        let invalid = TrustedProxies::new(&["invalid".into()]);
        assert_eq!(
            invalid.client_ip("192.0.2.1".parse().unwrap(), &headers),
            None
        );
    }

    #[test]
    fn persistence_is_private_atomic_and_corruption_disables_ip_login() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("ivnc-access-{}", uuid::Uuid::new_v4()));
        let path = dir.join("access.json");
        assert!(!AccessPolicy::load(&path).config.enabled);
        let p = policy(&["192.168.1.0/24"]);
        p.save(&path).unwrap();
        assert_eq!(AccessPolicy::load(&path).config, p.config);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let blocked = dir.join("blocked");
        std::fs::create_dir(&blocked).unwrap();
        assert!(p.save(&blocked).is_err());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        std::fs::write(&path, r#"{"enabled":true,"ip_allowlist":["bad"]}"#).unwrap();
        assert!(!AccessPolicy::load(&path).config.enabled);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
