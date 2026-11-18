//! Keep embedded Miao sessions separate for each published iVnc host and port.
//!
//! Miao v2 names its session after its config path. Containers commonly use the
//! same path, but browser cookies are shared across ports on the same host.

use axum::http::{header, HeaderMap, HeaderValue};
use sha2::{Digest, Sha256};

const SESSION_PREFIX: &str = "miao_session_";
const SCOPED_PREFIX: &str = "ivnc_miao_";

pub(super) struct MiaoCookieScope {
    prefix: String,
}

impl MiaoCookieScope {
    pub fn new(headers: &HeaderMap) -> Self {
        // Host includes the browser-visible port, unlike MIAO_PORT or the
        // container's listening port. Match iVnc's management-cookie isolation.
        let host = headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        let hash = format!("{:x}", Sha256::digest(host.as_bytes()));
        Self {
            prefix: format!("{SCOPED_PREFIX}{}_", &hash[..16]),
        }
    }

    pub fn prepare_upstream(&self, headers: &mut HeaderMap) {
        let pairs: Vec<_> = headers
            .get_all(header::COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(';'))
            .filter_map(|pair| pair.trim().split_once('='))
            .filter_map(|(name, value)| {
                if let Some(original) = name.strip_prefix(&self.prefix) {
                    return is_session(original).then_some((original, value));
                }
                // Do not fall back to legacy cookies: they may belong to a
                // different container. The passwordless panel creates a fresh
                // scoped session automatically, without deleting shared data.
                if name.starts_with(SCOPED_PREFIX) || is_session(name) {
                    return None;
                }
                Some((name, value))
            })
            .collect();
        let retained = pairs
            .iter()
            // Multiple same-name cookies (e.g. from different paths) are
            // ambiguous. Let Miao reject them and bootstrap a clean session.
            .filter(|(name, _)| {
                !is_session(name) || pairs.iter().filter(|(other, _)| other == name).count() == 1
            })
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ");
        headers.remove(header::COOKIE);
        if !retained.is_empty() {
            if let Ok(value) = retained.parse() {
                headers.insert(header::COOKIE, value);
            }
        }
    }

    pub fn downstream_cookie(&self, value: &HeaderValue) -> Option<HeaderValue> {
        let cookie = value.to_str().ok()?;
        let (name, _) = cookie.split_once('=')?;
        if is_session(name) {
            // Rename only; preserve HttpOnly, SameSite, Secure, Path, lifetime
            // and deletion attributes exactly as issued by Miao.
            format!("{}{cookie}", self.prefix).parse().ok()
        } else if name.starts_with(SCOPED_PREFIX) {
            None
        } else {
            Some(value.clone())
        }
    }
}

fn is_session(name: &str) -> bool {
    name.strip_prefix(SESSION_PREFIX)
        .is_some_and(|suffix| !suffix.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope_for(host: &str) -> MiaoCookieScope {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, host.parse().unwrap());
        MiaoCookieScope::new(&headers)
    }

    fn browser_cookie(scope: &MiaoCookieScope, cookie: &str) -> String {
        scope
            .downstream_cookie(&cookie.parse().unwrap())
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn published_ports_isolate_identical_upstream_cookie_names() {
        let a = scope_for("192.168.1.4:17000");
        let b = scope_for("192.168.1.4:16000");
        let ac = browser_cookie(&a, "miao_session_ae798df1b773=one; Path=/");
        let bc = browser_cookie(&b, "miao_session_ae798df1b773=two; Path=/");
        assert_ne!(ac.split_once('=').unwrap().0, bc.split_once('=').unwrap().0);
        // The browser sends both instances' cookies to either port.
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, format!("{ac}; {bc}").parse().unwrap());
        let mut other = headers.clone();
        a.prepare_upstream(&mut headers);
        b.prepare_upstream(&mut other);
        assert_eq!(headers[header::COOKIE], "miao_session_ae798df1b773=one");
        assert_eq!(other[header::COOKIE], "miao_session_ae798df1b773=two");
    }

    #[test]
    fn session_attributes_and_deletion_are_preserved() {
        let scope = scope_for("panel.test:17000");
        for tail in [
            "token; Path=/; HttpOnly; SameSite=Strict; Max-Age=86400",
            "token; Path=/; HttpOnly; SameSite=Strict; Max-Age=86400; Secure",
            "deleted; Path=/; HttpOnly; Max-Age=0",
        ] {
            let original = format!("miao_session_abc={tail}");
            let mapped = scope.downstream_cookie(&original.parse().unwrap()).unwrap();
            assert_eq!(
                mapped.to_str().unwrap(),
                format!("{}{original}", scope.prefix)
            );
        }
    }

    #[test]
    fn legacy_and_foreign_cookies_cannot_authenticate_upstream() {
        let scope = scope_for("panel.test:17000");
        let other = scope_for("panel.test:16000");
        let foreign = browser_cookie(&other, "miao_session_abc=foreign");
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            format!("miao_session_abc=legacy; {foreign}")
                .parse()
                .unwrap(),
        );
        scope.prepare_upstream(&mut headers);
        assert!(!headers.contains_key(header::COOKIE));
    }

    #[test]
    fn reads_all_cookie_headers_and_drops_ambiguous_sessions() {
        let scope = scope_for("panel.test:17000");
        let cookie = browser_cookie(&scope, "miao_session_abc=token");
        let mut headers = HeaderMap::new();
        headers.append(
            header::COOKIE,
            format!("{cookie}; theme=dark").parse().unwrap(),
        );
        headers.append(header::COOKIE, cookie.parse().unwrap());
        scope.prepare_upstream(&mut headers);
        assert_eq!(headers[header::COOKIE], "theme=dark");
    }

    #[test]
    fn unrelated_cookies_remain_unchanged_and_cannot_spoof_scoped_names() {
        let scope = scope_for("panel.test:17000");
        let unrelated = HeaderValue::from_static("theme=dark; Path=/");
        assert_eq!(scope.downstream_cookie(&unrelated), Some(unrelated));
        assert!(scope
            .downstream_cookie(&HeaderValue::from_static("ivnc_miao_foreign=value; Path=/"))
            .is_none());
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            format!("{}theme=bad; theme=dark", scope.prefix)
                .parse()
                .unwrap(),
        );
        scope.prepare_upstream(&mut headers);
        assert_eq!(headers[header::COOKIE], "theme=dark");
    }

    #[test]
    fn host_case_is_normalized_and_ipv6_ports_remain_distinct() {
        assert_eq!(
            scope_for("Panel.test:17000").prefix,
            scope_for("panel.test:17000").prefix
        );
        assert_ne!(
            scope_for("[::1]:17000").prefix,
            scope_for("[::1]:16000").prefix
        );
    }
}
