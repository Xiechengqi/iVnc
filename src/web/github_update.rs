//! GitHub release requests with ordered, independent proxy fallbacks.

use reqwest::{Client, Response, StatusCode};
use serde::Deserialize;
use std::{future::Future, path::Path, time::Duration};
use tokio::{io::AsyncWriteExt, time::Instant};

const RELEASE_API: &str = "https://api.github.com/repos/Xiechengqi/iVnc/releases/latest";
const MIN_BINARY_SIZE: u64 = 1024 * 1024;
const LOG_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Clone)]
struct Source {
    name: &'static str,
    url: String,
}

fn sources(url: &str) -> [Source; 3] {
    [
        Source {
            name: "GitHub",
            url: url.to_string(),
        },
        Source {
            name: "sina.dev",
            url: format!("https://sina.dev/{url}"),
        },
        Source {
            name: "httptunnel.top/gh",
            url: format!("https://httptunnel.top/gh/{url}"),
        },
    ]
}

#[derive(Clone, Copy)]
struct Timeouts {
    headers: Duration,
    json: Duration,
    idle: Duration,
    download: Duration,
}

const TIMEOUTS: Timeouts = Timeouts {
    headers: Duration::from_secs(8),
    json: Duration::from_secs(10),
    idle: Duration::from_secs(30),
    download: Duration::from_secs(300),
};

pub(super) fn client() -> Result<Client, reqwest::Error> {
    Client::builder()
        .connect_timeout(TIMEOUTS.headers)
        .timeout(TIMEOUTS.download)
        .user_agent("ivnc-updater")
        .build()
}

pub(super) fn binary_url(arch: &str) -> String {
    format!("https://github.com/Xiechengqi/iVnc/releases/download/latest/ivnc-linux-{arch}")
}

async fn get(client: &Client, source: &Source, timeouts: Timeouts) -> Result<Response, String> {
    let response = tokio::time::timeout(
        timeouts.headers,
        client.get(&source.url).timeout(timeouts.download).send(),
    )
    .await
    .map_err(|_| "等待响应头超时".to_string())?
    .map_err(|e| e.to_string())?;
    // No range request was made: a partial response is not a complete release.
    if response.status() != StatusCode::OK {
        return Err(format!("HTTP {}", response.status()));
    }
    Ok(response)
}

pub(super) async fn latest_version(client: &Client) -> Result<String, String> {
    latest_version_from(client, &sources(RELEASE_API), TIMEOUTS).await
}

async fn latest_version_from(
    client: &Client,
    sources: &[Source],
    timeouts: Timeouts,
) -> Result<String, String> {
    #[derive(Deserialize)]
    struct Release {
        tag_name: String,
    }

    let mut failures = Vec::new();
    for source in sources {
        let result = tokio::time::timeout(timeouts.json, async {
            let release = get(client, source, timeouts)
                .await?
                .json::<Release>()
                .await
                .map_err(|e| e.to_string())?;
            let version = release.tag_name.trim().trim_start_matches('v').to_string();
            if version.is_empty() {
                return Err("版本信息缺少有效 tag_name".to_string());
            }
            Ok(version)
        })
        .await
        .unwrap_or_else(|_| Err("读取版本信息超时".to_string()));
        match result {
            Ok(version) => return Ok(version),
            Err(error) => {
                log::warn!("Release check via {} failed: {}", source.name, error);
                failures.push(format!("{}: {}", source.name, error));
            }
        }
    }
    Err(failures.join("；"))
}

#[derive(Debug)]
pub(super) enum DownloadEvent {
    Attempt {
        source: &'static str,
    },
    Fallback {
        source: &'static str,
        next: &'static str,
        error: String,
    },
    Progress {
        source: &'static str,
        downloaded: u64,
        total: Option<u64>,
    },
}

enum Failure {
    Source(String),
    Local(String),
}

pub(super) async fn download<F, Fut>(
    client: &Client,
    url: &str,
    path: &Path,
    on_event: F,
) -> Result<u64, String>
where
    F: FnMut(DownloadEvent) -> Fut,
    Fut: Future<Output = ()>,
{
    download_from(client, &sources(url), path, TIMEOUTS, on_event).await
}

async fn download_from<F, Fut>(
    client: &Client,
    sources: &[Source],
    path: &Path,
    timeouts: Timeouts,
    mut on_event: F,
) -> Result<u64, String>
where
    F: FnMut(DownloadEvent) -> Fut,
    Fut: Future<Output = ()>,
{
    let mut failures = Vec::new();
    for (index, source) in sources.iter().enumerate() {
        on_event(DownloadEvent::Attempt {
            source: source.name,
        })
        .await;
        match download_once(client, source, path, timeouts, &mut on_event).await {
            Ok(size) => return Ok(size),
            Err(failure) => {
                // Drop the old stream/file before cleaning up. A new source
                // starts at byte zero, never appending to a partial response.
                match tokio::fs::remove_file(path).await {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(format!("清理临时文件失败: {e}")),
                }
                let error = match failure {
                    Failure::Local(error) => return Err(error),
                    Failure::Source(error) => error,
                };
                log::warn!("Upgrade download via {} failed: {}", source.name, error);
                failures.push(format!("{}: {}", source.name, error));
                if let Some(next) = sources.get(index + 1) {
                    on_event(DownloadEvent::Fallback {
                        source: source.name,
                        next: next.name,
                        error,
                    })
                    .await;
                }
            }
        }
    }
    Err(format!("所有下载来源均失败: {}", failures.join("；")))
}

fn valid_elf_header(header: &[u8]) -> bool {
    let machine = if cfg!(target_arch = "x86_64") {
        62u16
    } else {
        183u16
    };
    header.len() >= 20
        && header[..7] == [0x7f, b'E', b'L', b'F', 2, 1, 1]
        && matches!(u16::from_le_bytes([header[16], header[17]]), 2 | 3)
        && u16::from_le_bytes([header[18], header[19]]) == machine
}

async fn download_once<F, Fut>(
    client: &Client,
    source: &Source,
    path: &Path,
    timeouts: Timeouts,
    on_event: &mut F,
) -> Result<u64, Failure>
where
    F: FnMut(DownloadEvent) -> Fut,
    Fut: Future<Output = ()>,
{
    let mut response = get(client, source, timeouts)
        .await
        .map_err(Failure::Source)?;
    let total = response.content_length();
    let mut file = tokio::fs::File::create(path)
        .await
        .map_err(|e| Failure::Local(format!("创建临时文件失败: {e}")))?;
    let mut downloaded = 0u64;
    let mut header = Vec::with_capacity(20);
    let mut last_percent = 0u64;
    let mut last_bytes = 0u64;
    let mut last_log = Instant::now();
    loop {
        let chunk = tokio::time::timeout(timeouts.idle, response.chunk())
            .await
            .map_err(|_| Failure::Source("下载数据超时".to_string()))?
            .map_err(|e| Failure::Source(format!("下载数据失败: {e}")))?;
        let Some(chunk) = chunk else { break };
        header.extend_from_slice(&chunk[..chunk.len().min(20 - header.len())]);
        if header.len() == 20 && !valid_elf_header(&header) {
            return Err(Failure::Source(
                "响应不是当前架构的 Linux ELF 程序".to_string(),
            ));
        }
        file.write_all(&chunk)
            .await
            .map_err(|e| Failure::Local(format!("写入临时文件失败: {e}")))?;
        downloaded += chunk.len() as u64;
        let percent = total
            .filter(|size| *size > 0)
            .map(|size| downloaded.saturating_mul(100) / size)
            .unwrap_or(0);
        if percent >= last_percent + 5
            || downloaded - last_bytes >= 5 * 1024 * 1024
            || last_log.elapsed() >= LOG_INTERVAL
        {
            on_event(DownloadEvent::Progress {
                source: source.name,
                downloaded,
                total,
            })
            .await;
            last_percent = percent;
            last_bytes = downloaded;
            last_log = Instant::now();
        }
    }
    if downloaded < MIN_BINARY_SIZE || !valid_elf_header(&header) {
        return Err(Failure::Source(
            "下载文件过小或格式无效，可能损坏".to_string(),
        ));
    }
    if total.is_some_and(|size| size != downloaded) {
        return Err(Failure::Source(
            "下载文件大小与 Content-Length 不一致".to_string(),
        ));
    }
    file.shutdown()
        .await
        .map_err(|e| Failure::Local(format!("保存临时文件失败: {e}")))?;
    Ok(downloaded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, Bytes},
        extract::{Request, State},
        response::IntoResponse,
        Router,
    };
    use futures::{stream, StreamExt};
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct Reply {
        status: StatusCode,
        body: Vec<u8>,
        headers_delay: Duration,
        body_delay: Duration,
        broken: bool,
        length: Option<u64>,
    }

    impl Reply {
        fn body(body: impl Into<Vec<u8>>) -> Self {
            Self {
                status: StatusCode::OK,
                body: body.into(),
                headers_delay: Duration::ZERO,
                body_delay: Duration::ZERO,
                broken: false,
                length: None,
            }
        }
        fn status(status: StatusCode) -> Self {
            Self {
                status,
                ..Self::body(Vec::new())
            }
        }
    }

    struct Fixture {
        base: String,
        calls: Arc<Mutex<Vec<String>>>,
        server: tokio::task::JoinHandle<()>,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.server.abort();
        }
    }
    impl Fixture {
        fn sources(&self) -> [Source; 3] {
            ["GitHub", "sina.dev", "httptunnel.top/gh"].map(|name| Source {
                name,
                url: format!("{}/{name}", self.base),
            })
        }
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    async fn fixture(replies: [Reply; 3]) -> Fixture {
        type FixtureState = (Arc<[Reply; 3]>, Arc<Mutex<Vec<String>>>);
        async fn handler(
            State((replies, calls)): State<FixtureState>,
            request: Request,
        ) -> impl IntoResponse {
            assert_eq!(request.method(), "GET");
            assert_eq!(request.headers()["user-agent"], "ivnc-updater");
            let name = request.uri().path().trim_start_matches('/').to_string();
            calls.lock().unwrap().push(name.clone());
            let index = match name.as_str() {
                "GitHub" => 0,
                "sina.dev" => 1,
                "httptunnel.top/gh" => 2,
                _ => panic!("unexpected source: {name}"),
            };
            let reply = replies[index].clone();
            tokio::time::sleep(reply.headers_delay).await;
            // Split the body so stalled/failed responses have already written
            // a valid ELF prefix before the failure and retry.
            let midpoint = reply.body.len().min(4096);
            let first = reply.body[..midpoint].to_vec();
            let rest = reply.body[midpoint..].to_vec();
            let body = stream::once(async move { Ok::<_, std::io::Error>(Bytes::from(first)) })
                .chain(stream::once(async move {
                    tokio::time::sleep(reply.body_delay).await;
                    if reply.broken {
                        Err(std::io::Error::other("connection interrupted"))
                    } else {
                        Ok(Bytes::from(rest))
                    }
                }));
            let mut response = (reply.status, Body::from_stream(body)).into_response();
            if let Some(length) = reply.length {
                response
                    .headers_mut()
                    .insert("content-length", length.into());
            }
            response
        }
        let calls = Arc::new(Mutex::new(Vec::new()));
        let state = (Arc::new(replies), Arc::clone(&calls));
        let router = Router::new().fallback(handler).with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Fixture {
            base,
            calls,
            server,
        }
    }

    fn test_client() -> Client {
        Client::builder()
            .no_proxy()
            .user_agent("ivnc-updater")
            .build()
            .unwrap()
    }
    fn timeouts() -> Timeouts {
        Timeouts {
            headers: Duration::from_millis(100),
            json: Duration::from_millis(150),
            idle: Duration::from_millis(100),
            download: Duration::from_secs(2),
        }
    }
    fn unavailable() -> Reply {
        Reply::status(StatusCode::SERVICE_UNAVAILABLE)
    }
    fn release() -> Reply {
        Reply::body(br#"{"tag_name":"v1.2.3"}"#.to_vec())
    }
    fn binary(fill: u8) -> Vec<u8> {
        let mut body = vec![fill; MIN_BINARY_SIZE as usize + 4096];
        body[..7].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1]);
        body[16..18].copy_from_slice(&3u16.to_le_bytes());
        let machine = if cfg!(target_arch = "x86_64") {
            62u16
        } else {
            183u16
        };
        body[18..20].copy_from_slice(&machine.to_le_bytes());
        body
    }
    struct TempPath(std::path::PathBuf);
    impl TempPath {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!("ivnc-github-update-{}", uuid::Uuid::new_v4())))
        }
    }
    impl Drop for TempPath {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn proxy_urls_wrap_the_original_url_independently_in_the_requested_order() {
        for url in [
            RELEASE_API.to_string(),
            binary_url("amd64"),
            binary_url("arm64"),
        ] {
            let candidates = sources(&url);
            assert_eq!(candidates[0].name, "GitHub");
            assert_eq!(candidates[0].url, url);
            assert_eq!(candidates[1].name, "sina.dev");
            assert_eq!(candidates[1].url, format!("https://sina.dev/{url}"));
            assert_eq!(candidates[2].name, "httptunnel.top/gh");
            assert_eq!(
                candidates[2].url,
                format!("https://httptunnel.top/gh/{url}")
            );
        }
    }

    #[tokio::test]
    async fn successful_direct_release_check_never_contacts_proxies() {
        let server = fixture([release(), unavailable(), unavailable()]).await;
        assert_eq!(
            latest_version_from(&test_client(), &server.sources(), timeouts())
                .await
                .unwrap(),
            "1.2.3"
        );
        assert_eq!(server.calls(), ["GitHub"]);
    }

    #[tokio::test]
    async fn release_check_falls_back_on_http_errors_to_each_proxy_in_order() {
        for index in [1, 2] {
            let mut replies = [
                Reply::status(StatusCode::TOO_MANY_REQUESTS),
                unavailable(),
                unavailable(),
            ];
            replies[index] = release();
            let server = fixture(replies).await;
            assert_eq!(
                latest_version_from(&test_client(), &server.sources(), timeouts())
                    .await
                    .unwrap(),
                "1.2.3"
            );
            assert_eq!(
                server.calls(),
                ["GitHub", "sina.dev", "httptunnel.top/gh"][..=index]
            );
        }
    }

    #[tokio::test]
    async fn invalid_release_json_and_empty_version_are_source_failures() {
        for invalid in [
            "<html>proxy error</html>",
            r#"{"message":"rate limited"}"#,
            r#"{"tag_name":"v"}"#,
        ] {
            let server = fixture([Reply::body(invalid), Reply::body("{}"), release()]).await;
            assert_eq!(
                latest_version_from(&test_client(), &server.sources(), timeouts())
                    .await
                    .unwrap(),
                "1.2.3"
            );
            assert_eq!(server.calls(), ["GitHub", "sina.dev", "httptunnel.top/gh"]);
        }
    }

    #[tokio::test]
    async fn metadata_header_and_body_timeouts_both_advance_to_next_source() {
        let mut headers = release();
        headers.headers_delay = Duration::from_millis(400);
        let mut body = release();
        body.body_delay = Duration::from_millis(400);
        let server = fixture([headers, body, release()]).await;
        assert_eq!(
            latest_version_from(&test_client(), &server.sources(), timeouts())
                .await
                .unwrap(),
            "1.2.3"
        );
        assert_eq!(server.calls(), ["GitHub", "sina.dev", "httptunnel.top/gh"]);
    }

    #[tokio::test]
    async fn release_check_reports_all_three_failures() {
        let server = fixture([unavailable(), unavailable(), unavailable()]).await;
        let error = latest_version_from(&test_client(), &server.sources(), timeouts())
            .await
            .unwrap_err();
        for source in ["GitHub:", "sina.dev:", "httptunnel.top/gh:"] {
            assert!(error.contains(source), "{error}");
        }
        assert_eq!(server.calls().len(), 3);
    }

    #[tokio::test]
    async fn successful_binary_stops_at_the_first_available_source_and_logs_fallbacks() {
        for index in [0, 1, 2] {
            let expected = binary(42);
            let mut replies = [unavailable(), unavailable(), unavailable()];
            replies[index] = Reply {
                length: Some(expected.len() as u64),
                ..Reply::body(expected.clone())
            };
            let server = fixture(replies).await;
            let path = TempPath::new();
            let mut events = Vec::new();
            let size = download_from(
                &test_client(),
                &server.sources(),
                &path.0,
                timeouts(),
                |event| {
                    events.push(event);
                    async {}
                },
            )
            .await
            .unwrap();
            assert_eq!(size, expected.len() as u64);
            assert_eq!(tokio::fs::read(&path.0).await.unwrap(), expected);
            assert_eq!(
                server.calls(),
                ["GitHub", "sina.dev", "httptunnel.top/gh"][..=index]
            );
            assert_eq!(
                events
                    .iter()
                    .filter(|e| matches!(e, DownloadEvent::Fallback { .. }))
                    .count(),
                index
            );
            assert!(matches!(
                events.first(),
                Some(DownloadEvent::Attempt { source: "GitHub" })
            ));
            if index == 2 {
                assert!(
                    matches!(&events[1], DownloadEvent::Fallback { source: "GitHub", next: "sina.dev", error } if error.contains("503"))
                );
                assert!(matches!(
                    &events[3],
                    DownloadEvent::Fallback {
                        source: "sina.dev",
                        next: "httptunnel.top/gh",
                        ..
                    }
                ));
            }
        }
    }

    #[tokio::test]
    async fn interrupted_and_stalled_downloads_restart_at_zero_on_next_proxy() {
        let expected = binary(77);
        let broken = Reply {
            broken: true,
            body_delay: Duration::from_millis(20),
            length: Some(expected.len() as u64),
            ..Reply::body(binary(11))
        };
        let stalled = Reply {
            body_delay: Duration::from_millis(400),
            ..Reply::body(binary(22))
        };
        let server = fixture([broken, stalled, Reply::body(expected.clone())]).await;
        let path = TempPath::new();
        download_from(
            &test_client(),
            &server.sources(),
            &path.0,
            timeouts(),
            |_| async {},
        )
        .await
        .unwrap();
        assert_eq!(tokio::fs::read(&path.0).await.unwrap(), expected);
        assert_eq!(server.calls(), ["GitHub", "sina.dev", "httptunnel.top/gh"]);
    }

    #[tokio::test]
    async fn total_download_deadline_also_advances_to_the_next_source() {
        let slow = Reply {
            body_delay: Duration::from_millis(600),
            ..Reply::body(binary(11))
        };
        let server = fixture([slow, Reply::body(binary(77)), unavailable()]).await;
        let path = TempPath::new();
        let limits = Timeouts {
            download: Duration::from_millis(200),
            idle: Duration::from_secs(1),
            ..timeouts()
        };
        download_from(
            &test_client(),
            &server.sources(),
            &path.0,
            limits,
            |_| async {},
        )
        .await
        .unwrap();
        assert_eq!(tokio::fs::read(&path.0).await.unwrap(), binary(77));
        assert_eq!(server.calls(), ["GitHub", "sina.dev"]);
    }

    #[tokio::test]
    async fn connection_failure_and_header_timeout_also_fall_back_for_binary() {
        let slow = Reply {
            headers_delay: Duration::from_millis(400),
            ..Reply::body(binary(11))
        };
        let server = fixture([unavailable(), slow, Reply::body(binary(77))]).await;
        let dead = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = dead.local_addr().unwrap();
        drop(dead);
        let mut sources = server.sources();
        sources[0].url = format!("http://{address}/");
        let path = TempPath::new();
        download_from(&test_client(), &sources, &path.0, timeouts(), |_| async {})
            .await
            .unwrap();
        assert_eq!(tokio::fs::read(&path.0).await.unwrap(), binary(77));
        assert_eq!(server.calls(), ["sina.dev", "httptunnel.top/gh"]);
    }

    #[tokio::test]
    async fn invalid_payloads_are_retried_before_installation() {
        let mut wrong_machine = binary(1);
        wrong_machine[18..20].copy_from_slice(&0u16.to_le_bytes());
        let mut wrong_class = binary(1);
        wrong_class[4] = 1;
        let short = binary(1)[..4096].to_vec();
        for invalid in [
            vec![b'<'; MIN_BINARY_SIZE as usize + 128],
            wrong_machine,
            wrong_class,
            short,
        ] {
            let server =
                fixture([Reply::body(invalid), unavailable(), Reply::body(binary(77))]).await;
            let path = TempPath::new();
            download_from(
                &test_client(),
                &server.sources(),
                &path.0,
                timeouts(),
                |_| async {},
            )
            .await
            .unwrap();
            assert_eq!(tokio::fs::read(&path.0).await.unwrap(), binary(77));
            assert_eq!(server.calls().len(), 3);
        }
    }

    #[tokio::test]
    async fn partial_status_and_short_content_length_are_not_accepted() {
        let partial = Reply {
            status: StatusCode::PARTIAL_CONTENT,
            ..Reply::body(binary(11))
        };
        let truncated = Reply {
            length: Some(MIN_BINARY_SIZE * 2),
            ..Reply::body(binary(22))
        };
        let server = fixture([partial, truncated, Reply::body(binary(77))]).await;
        let path = TempPath::new();
        download_from(
            &test_client(),
            &server.sources(),
            &path.0,
            timeouts(),
            |_| async {},
        )
        .await
        .unwrap();
        assert_eq!(tokio::fs::read(&path.0).await.unwrap(), binary(77));
        assert_eq!(server.calls().len(), 3);
    }

    #[tokio::test]
    async fn all_failed_downloads_clean_temp_and_leave_existing_binary_untouched() {
        let broken = Reply {
            broken: true,
            body_delay: Duration::from_millis(20),
            ..Reply::body(binary(11))
        };
        let server = fixture([
            unavailable(),
            Reply::body("<html>proxy error</html>"),
            broken,
        ])
        .await;
        let path = TempPath::new();
        let install = TempPath::new();
        tokio::fs::write(&install.0, b"existing installed binary")
            .await
            .unwrap();
        let error = download_from(
            &test_client(),
            &server.sources(),
            &path.0,
            timeouts(),
            |_| async {},
        )
        .await
        .unwrap_err();
        assert!(error.starts_with("所有下载来源均失败:"));
        for source in ["GitHub:", "sina.dev:", "httptunnel.top/gh:"] {
            assert!(error.contains(source), "{error}");
        }
        assert!(!path.0.exists());
        assert_eq!(
            tokio::fs::read(&install.0).await.unwrap(),
            b"existing installed binary"
        );
        assert_eq!(server.calls().len(), 3);
    }

    #[tokio::test]
    async fn local_file_errors_do_not_trigger_useless_network_retries() {
        let server = fixture([Reply::body(binary(11)), unavailable(), unavailable()]).await;
        let missing_parent = TempPath::new();
        let path = missing_parent.0.join("ivnc.new");
        let error = download_from(
            &test_client(),
            &server.sources(),
            &path,
            timeouts(),
            |_| async {},
        )
        .await
        .unwrap_err();
        assert!(error.contains("创建临时文件失败"));
        assert_eq!(server.calls(), ["GitHub"]);
        assert!(!path.exists());
    }
}
