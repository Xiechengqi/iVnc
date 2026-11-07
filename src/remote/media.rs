//! Routes RFC4571 ICE-TCP connections using the authenticated SDP's ICE ufrag.
//! Media is forwarded without decoding. DTLS/SRTP remains between viewer and worker.
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU8, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::Notify,
};

/// Concurrent ICE-TCP connections per session; browsers may try one per local interface.
const MAX_RELAYS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A's browser view, or B's local view of its own application.
    View,
    /// A remote signaling connection received by B.
    Signal,
    /// A remote file or configuration request received by B.
    Data,
}

/// Why access ended. Clients stop reconnecting automatically unless it was `Closed`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum StopReason {
    Closed = 1,
    Revoked = 2,
    Disconnected = 3,
    Stopped = 4,
}
impl StopReason {
    fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Closed),
            2 => Some(Self::Revoked),
            3 => Some(Self::Disconnected),
            4 => Some(Self::Stopped),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::Revoked => "revoked",
            Self::Disconnected => "disconnected",
            Self::Stopped => "stopped",
        }
    }
}

/// Remote identity attached to B-side access.
#[derive(Clone, Debug, Default)]
pub struct Origin {
    pub grant: String,
    pub label: String,
    pub peer: String,
}

pub struct Lease {
    pub id: String,
    pub app: String,
    pub remote: bool,
    pub owner: bool,
    pub generation: String,
    pub kind: Kind,
    pub origin: Option<Origin>,
    relays: AtomicUsize,
    connected: AtomicUsize,
    stopped: AtomicU8,
    notify: Notify,
    pub created: Instant,
}
impl Lease {
    pub fn new(app: String, remote: bool, owner: bool, generation: String) -> Arc<Self> {
        Self::build(app, remote, owner, generation, Kind::View, None)
    }
    pub fn remote(app: String, kind: Kind, generation: String, origin: Origin) -> Arc<Self> {
        Self::build(app, true, false, generation, kind, Some(origin))
    }
    fn build(
        app: String,
        remote: bool,
        owner: bool,
        generation: String,
        kind: Kind,
        origin: Option<Origin>,
    ) -> Arc<Self> {
        Arc::new(Self {
            id: super::token(),
            app,
            remote,
            owner,
            generation,
            kind,
            origin,
            relays: AtomicUsize::new(0),
            connected: AtomicUsize::new(0),
            stopped: AtomicU8::new(0),
            notify: Notify::new(),
            created: Instant::now(),
        })
    }
    pub fn stop(&self) {
        self.stop_with(StopReason::Closed);
    }
    /// The first reason wins; later stops do not change it.
    pub fn stop_with(&self, reason: StopReason) {
        let _ = self
            .stopped
            .compare_exchange(0, reason as u8, Ordering::SeqCst, Ordering::SeqCst);
        self.notify.notify_waiters();
    }
    pub fn stop_reason(&self) -> Option<StopReason> {
        StopReason::from_u8(self.stopped.load(Ordering::SeqCst))
    }
    pub fn active(&self) -> bool {
        self.stopped.load(Ordering::SeqCst) == 0
    }
    pub fn connected(&self) -> bool {
        self.active() && self.connected.load(Ordering::SeqCst) > 0
    }
    pub async fn cancelled(&self) {
        let notified = self.notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if self.active() {
            notified.await;
        }
    }
}

struct RelayGuard<'a> {
    lease: &'a Lease,
    connected: bool,
}
impl Drop for RelayGuard<'_> {
    fn drop(&mut self) {
        if self.connected {
            self.lease.connected.fetch_sub(1, Ordering::SeqCst);
        }
        self.lease.relays.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Clone)]
struct Route {
    destination: String,
    lease: Arc<Lease>,
}
#[derive(Default)]
pub struct MediaRouter {
    leases: Mutex<HashMap<String, Arc<Lease>>>,
    routes: Mutex<HashMap<String, Route>>,
}
impl MediaRouter {
    pub fn track(&self, lease: &Arc<Lease>) -> Result<(), String> {
        let mut leases = self.leases.lock().unwrap();
        leases.retain(|_, l| l.active());
        if leases.len() >= 128 {
            return Err("Too many application sessions".into());
        }
        leases.insert(lease.id.clone(), lease.clone());
        Ok(())
    }
    /// Remote viewers connected to a local application. File requests are not sessions.
    pub fn sessions(&self, app: &str) -> Vec<serde_json::Value> {
        let mut sessions: Vec<_> = self
            .leases
            .lock()
            .unwrap()
            .values()
            .filter(|l| l.app == app && l.kind == Kind::Signal && l.active())
            .map(|l| {
                let origin = l.origin.clone().unwrap_or_default();
                serde_json::json!({
                    "id": l.id,
                    "age_seconds": l.created.elapsed().as_secs(),
                    "generation": l.generation,
                    "grant": origin.grant,
                    "client": origin.label,
                    "peer": origin.peer,
                    "media": l.connected(),
                })
            })
            .collect();
        sessions.sort_by_key(|s| s["age_seconds"].as_u64());
        sessions
    }
    pub fn disconnect(&self, app: &str, id: &str) -> bool {
        let leases = self.leases.lock().unwrap();
        if let Some(lease) = leases.get(id).filter(|l| l.app == app && l.remote) {
            lease.stop_with(StopReason::Disconnected);
            true
        } else {
            false
        }
    }
    /// Ends every remote session that used a revoked credential.
    pub fn revoke_grant(&self, app: &str, grant: &str) {
        for lease in self.leases.lock().unwrap().values() {
            if lease.app == app && lease.origin.as_ref().is_some_and(|o| o.grant == grant) {
                lease.stop_with(StopReason::Revoked);
            }
        }
    }

    pub fn register_sdp(
        &self,
        sdp: &str,
        destination: &str,
        lease: &Arc<Lease>,
    ) -> Result<(), String> {
        let mut routes = self.routes.lock().unwrap();
        routes.retain(|_, r| r.lease.active());
        if routes.len() >= 256 {
            return Err("Too many application media sessions".into());
        }
        for line in sdp.lines() {
            if let Some(ufrag) = line.strip_prefix("a=ice-ufrag:") {
                let ufrag = ufrag.trim();
                if ufrag.is_empty() || ufrag.len() > 256 {
                    return Err("Invalid ICE credentials".into());
                }
                if let Some(old) = routes.get(ufrag) {
                    if !Arc::ptr_eq(&old.lease, lease) {
                        return Err("Duplicate ICE credentials".into());
                    }
                }
                routes.insert(
                    ufrag.into(),
                    Route {
                        destination: destination.into(),
                        lease: lease.clone(),
                    },
                );
            }
        }
        Ok(())
    }
    pub fn revoke(&self, app: &str, remote_only: bool, reason: StopReason) {
        for lease in self.leases.lock().unwrap().values() {
            if lease.app == app && (!remote_only || lease.remote) {
                lease.stop_with(reason);
            }
        }
        let mut routes = self.routes.lock().unwrap();
        routes.retain(|_, r| {
            if r.lease.app == app && (!remote_only || r.lease.remote) {
                r.lease.stop_with(reason);
                false
            } else {
                r.lease.active()
            }
        });
    }
    pub async fn try_relay(
        &self,
        mut stream: TcpStream,
        first: Vec<u8>,
    ) -> Result<Option<(TcpStream, Vec<u8>)>, String> {
        let username = stun_username(first.get(2..).unwrap_or_default());
        let route = {
            let routes = self.routes.lock().unwrap();
            username.and_then(|name| {
                name.split_once(':').and_then(|(receiver, _)| {
                    routes.get(receiver).filter(|r| r.lease.active()).cloned()
                })
            })
        };
        let Some(route) = route else {
            return Ok(Some((stream, first)));
        };
        if route.lease.relays.fetch_add(1, Ordering::SeqCst) >= MAX_RELAYS {
            route.lease.relays.fetch_sub(1, Ordering::SeqCst);
            return Err("Too many media connections for this application view".into());
        }
        let mut guard = RelayGuard {
            lease: &route.lease,
            connected: false,
        };
        let _ = stream.set_nodelay(true);
        tokio::select! {
            result = async {
                let mut upstream = tokio::time::timeout(
                    Duration::from_secs(10),
                    TcpStream::connect(&route.destination),
                )
                .await
                .map_err(|_| "Media upstream timeout")?
                .map_err(|e| e.to_string())?;
                let _ = upstream.set_nodelay(true);
                upstream.write_all(&first).await.map_err(|e| e.to_string())?;
                route.lease.connected.fetch_add(1, Ordering::SeqCst);
                guard.connected = true;
                tokio::io::copy_bidirectional(&mut stream, &mut upstream)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(None)
            } => result,
            _ = route.lease.cancelled() => Ok(None),
        }
    }
}
pub async fn first_frame(stream: &mut TcpStream) -> Result<Vec<u8>, String> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let size = stream.read_u16().await.map_err(|e| e.to_string())? as usize;
        if !(20..=4096).contains(&size) {
            return Err("Invalid first ICE frame size".into());
        }
        let mut frame = vec![0; size + 2];
        frame[..2].copy_from_slice(&(size as u16).to_be_bytes());
        stream
            .read_exact(&mut frame[2..])
            .await
            .map_err(|e| e.to_string())?;
        Ok(frame)
    })
    .await
    .map_err(|_| "Initial ICE frame timed out")?
}
fn stun_username(packet: &[u8]) -> Option<&str> {
    if packet.len() < 20 || packet[0] & 0xc0 != 0 || packet[4..8] != [0x21, 0x12, 0xa4, 0x42] {
        return None;
    }
    let end = 20 + u16::from_be_bytes([packet[2], packet[3]]) as usize;
    if end > packet.len() {
        return None;
    }
    let mut offset = 20;
    while offset + 4 <= end {
        let kind = u16::from_be_bytes([packet[offset], packet[offset + 1]]);
        let size = u16::from_be_bytes([packet[offset + 2], packet[offset + 3]]) as usize;
        offset += 4;
        if offset + size > end {
            return None;
        }
        if kind == 6 {
            return std::str::from_utf8(&packet[offset..offset + size]).ok();
        }
        offset += (size + 3) & !3;
    }
    None
}
#[cfg(test)]
mod tests {
    use super::*;

    async fn tcp_pair() -> (TcpStream, TcpStream) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (server, _) = listener.accept().await.unwrap();
        (client, server)
    }

    #[tokio::test]
    async fn media_failure_and_disconnect_release_connection_for_retry() {
        let router = MediaRouter::default();
        let lease = Lease::new("app".into(), true, false, "generation".into());
        // RFC4571 length, STUN header/transaction ID, then USERNAME app:cli.
        let mut frame = vec![0, 32, 0, 1, 0, 12, 0x21, 0x12, 0xa4, 0x42];
        frame.extend([0; 12]);
        frame.extend([0, 6, 0, 7]);
        frame.extend(b"app:cli\0");

        router
            .register_sdp("a=ice-ufrag:app", "127.0.0.1:0", &lease)
            .unwrap();
        let (_browser, incoming) = tcp_pair().await;
        assert!(router.try_relay(incoming, frame.clone()).await.is_err());
        assert!(!lease.connected());

        let upstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        router
            .register_sdp(
                "a=ice-ufrag:app",
                &upstream.local_addr().unwrap().to_string(),
                &lease,
            )
            .unwrap();
        for revoke in [false, true] {
            let (mut browser, incoming) = tcp_pair().await;
            let forwarding = router.try_relay(incoming, frame.clone());
            let exchange = async {
                let (mut worker, _) = upstream.accept().await.unwrap();
                let mut received = vec![0; frame.len()];
                worker.read_exact(&mut received).await.unwrap();
                assert_eq!(received, frame);
                assert!(lease.connected());
                browser.write_all(b"ping").await.unwrap();
                let mut payload = [0; 4];
                worker.read_exact(&mut payload).await.unwrap();
                assert_eq!(&payload, b"ping");
                worker.write_all(b"pong").await.unwrap();
                browser.read_exact(&mut payload).await.unwrap();
                assert_eq!(&payload, b"pong");
                if revoke {
                    lease.stop();
                    assert_eq!(browser.read(&mut payload).await.unwrap(), 0);
                    assert_eq!(worker.read(&mut payload).await.unwrap(), 0);
                }
                drop(worker);
                drop(browser);
            };
            let (result, ()) = tokio::time::timeout(Duration::from_secs(3), async {
                tokio::join!(forwarding, exchange)
            })
            .await
            .unwrap();
            assert!(result.unwrap().is_none());
            assert!(!lease.connected());
        }
    }

    #[tokio::test]
    async fn sessions_allow_parallel_media_and_report_why_they_ended() {
        let router = MediaRouter::default();
        let origin = Origin {
            grant: "grant".into(),
            label: "A".into(),
            peer: "192.0.2.1".into(),
        };
        let signal = Lease::remote("app".into(), Kind::Signal, "g".into(), origin.clone());
        let data = Lease::remote("app".into(), Kind::Data, "g".into(), origin);
        router.track(&signal).unwrap();
        router.track(&data).unwrap();
        // File requests are not listed as sessions.
        let sessions = router.sessions("app");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0]["grant"], "grant");
        assert_eq!(sessions[0]["peer"], "192.0.2.1");

        let upstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        router
            .register_sdp(
                "a=ice-ufrag:app",
                &upstream.local_addr().unwrap().to_string(),
                &signal,
            )
            .unwrap();
        let mut frame = vec![0, 32, 0, 1, 0, 12, 0x21, 0x12, 0xa4, 0x42];
        frame.extend([0; 12]);
        frame.extend([0, 6, 0, 7]);
        frame.extend(b"app:cli\0");
        let mut relays = Vec::new();
        let mut browsers = Vec::new();
        let expected = frame.len();
        for _ in 0..2 {
            let (browser, incoming) = tcp_pair().await;
            browsers.push(browser);
            let router = &router;
            let frame = frame.clone();
            relays.push(async move { router.try_relay(incoming, frame).await });
        }
        let accept = async {
            let mut workers = Vec::new();
            for _ in 0..2 {
                let mut worker = upstream.accept().await.unwrap().0;
                let mut received = vec![0; expected];
                worker.read_exact(&mut received).await.unwrap();
                workers.push(worker);
            }
            assert!(signal.connected());
            router.revoke_grant("app", "grant");
            workers
        };
        let (results, _) = tokio::time::timeout(Duration::from_secs(3), async {
            tokio::join!(futures::future::join_all(relays), accept)
        })
        .await
        .unwrap();
        assert!(results.iter().all(|r| matches!(r, Ok(None))));
        assert_eq!(signal.stop_reason(), Some(StopReason::Revoked));
        assert_eq!(data.stop_reason(), Some(StopReason::Revoked));
        assert!(!signal.connected());
        assert!(router.sessions("app").is_empty());
    }

    #[test]
    fn fragmented_or_truncated_stun_cannot_choose_an_application() {
        let mut p = vec![0; 20];
        p[0] = 0;
        p[1] = 1;
        p[4..8].copy_from_slice(&[0x21, 0x12, 0xa4, 0x42]);
        p.extend([0, 6, 0, 7]);
        p.extend(b"app:cli");
        p.push(0);
        p[3] = 12;
        assert_eq!(stun_username(&p), Some("app:cli"));
        for n in 0..p.len() {
            assert_eq!(stun_username(&p[..n]), None);
        }
        p[7] = 0;
        assert_eq!(stun_username(&p), None);
    }
    #[tokio::test]
    async fn revocation_closes_media_and_never_reassigns_credentials() {
        let router = MediaRouter::default();
        let a = Lease::new("app".into(), true, false, "generation".into());
        router
            .register_sdp("a=ice-ufrag:test\r\n", "127.0.0.1:1", &a)
            .unwrap();
        let b = Lease::new("other".into(), true, false, "generation".into());
        assert!(router
            .register_sdp("a=ice-ufrag:test", "127.0.0.1:2", &b)
            .is_err());
        router.revoke("app", true, StopReason::Revoked);
        assert_eq!(a.stop_reason(), Some(StopReason::Revoked));
        assert!(!a.active());
        a.cancelled().await;
    }
}
