//! str0m Sans-I/O WebRTC session driver
//!
//! Each RtcSession wraps a str0m `Rtc` instance and drives it via
//! a tokio task that multiplexes TCP I/O, RTP broadcast, audio, and
//! text forwarding through a single event loop.

use super::data_channel::InputDataChannel;
use super::media_track::{rtp_util, VideoRtpClock};
use super::tcp_framing::{frame_packet, TcpFrameDecoder};
use super::video_format::{constrained_baseline, offered_h264_level, rtc_codec};
use super::WebRTCError;
use crate::clipboard::ClipboardReceiver;
use crate::config::VideoCodec;
use crate::file_upload::FileUploadHandler;
use crate::input::{InputEvent, InputEventData};
use crate::runtime_settings::RuntimeSettings;
use crate::web::SharedState;

use log::{debug, error, info, warn};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use str0m::change::SdpOffer;
use str0m::channel::{ChannelData, ChannelId};
use str0m::media::{MediaKind, Mid, Pt};
use str0m::net::{self, Protocol};
use str0m::rtp::SeqNo;
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc};

/// A single str0m WebRTC session bound to a TCP connection.
pub struct RtcSession {
    /// Unique session ID
    pub id: String,
    /// The str0m Sans-I/O instance
    pub rtc: Rtc,
    /// Video media line ID (set after SDP negotiation)
    pub video_mid: Option<Mid>,
    /// Audio media line ID (set after SDP negotiation)
    pub audio_mid: Option<Mid>,
    /// DataChannel ID for input
    pub dc_id: Option<ChannelId>,
    /// Negotiated video payload type (discovered from SDP, e.g. H264 PT)
    video_pt: Option<Pt>,
    /// Negotiated audio payload type (discovered from SDP)
    audio_pt: Option<Pt>,
    video_codec: VideoCodec,
    h264_level: Option<u8>,
    h264_ready: bool,
    video_limit_owner: Option<Arc<SharedState>>,
    /// Whether the session is connected
    pub connected: bool,
    /// RTP sequence counter for video (str0m RTP mode needs us to supply seq)
    video_seq: u64,
    video_clock: VideoRtpClock,
    /// RTP sequence counter for audio
    audio_seq: u64,
}

impl RtcSession {
    /// Create a new RtcSession with str0m configured for ICE-lite + RTP mode.
    pub fn new(id: String, video_codec: VideoCodec, offer_sdp: &str) -> Self {
        let now = Instant::now();
        let h264_level = offered_h264_level(offer_sdp);
        let mut builder = Rtc::builder()
            .set_ice_lite(true)
            .set_rtp_mode(true)
            .clear_codecs()
            .enable_opus(true);
        match video_codec {
            VideoCodec::H264 => builder.codec_config().add_h264(
                96.into(),
                Some(97.into()),
                true,
                0x42e000 | u32::from(h264_level.unwrap_or(31)),
            ),
            VideoCodec::VP8 => builder = builder.enable_vp8(true),
            VideoCodec::VP9 => builder = builder.enable_vp9(true),
            VideoCodec::AV1 => builder = builder.enable_av1(true),
        }
        let rtc = builder.build(now);

        Self {
            id,
            rtc,
            video_mid: None,
            audio_mid: None,
            dc_id: None,
            video_pt: None,
            audio_pt: None,
            video_codec,
            h264_level,
            h264_ready: false,
            video_limit_owner: None,
            connected: false,
            video_seq: 0,
            video_clock: VideoRtpClock::default(),
            audio_seq: 0,
        }
    }

    /// Add a TCP passive ICE candidate for the given listen address.
    pub fn add_local_tcp_candidate(&mut self, addr: SocketAddr) -> Result<(), WebRTCError> {
        use str0m::net::TcpType;
        let candidate = Candidate::builder()
            .tcp()
            .host(addr)
            .tcptype(TcpType::Passive)
            .build()
            .map_err(|e| WebRTCError::IceError(format!("Failed to build TCP candidate: {}", e)))?;
        self.rtc.add_local_candidate(candidate);
        Ok(())
    }

    /// Accept an SDP offer and return the SDP answer string.
    pub fn accept_offer(&mut self, offer_sdp: &str) -> Result<String, WebRTCError> {
        if self.video_codec == VideoCodec::H264 && self.h264_level.is_none() {
            return Err(WebRTCError::MediaError("IVNC_VIDEO_UNSUPPORTED: H.264 Constrained Baseline with packetization-mode=1 is required".into()));
        }
        let offer = SdpOffer::from_sdp_string(offer_sdp)
            .map_err(|e| WebRTCError::SdpError(format!("Failed to parse SDP offer: {}", e)))?;

        let answer = self
            .rtc
            .sdp_api()
            .accept_offer(offer)
            .map_err(|e| WebRTCError::SdpError(format!("Failed to accept offer: {}", e)))?;

        let answer = answer.to_sdp_string();
        // A rejected video m-line can coexist with a working DataChannel.
        // Do not report that as a usable desktop connection.
        if !answer
            .lines()
            .any(|line| line.starts_with("m=video ") && !line.starts_with("m=video 0 "))
        {
            return Err(WebRTCError::MediaError(format!(
                "IVNC_VIDEO_UNSUPPORTED: no compatible {} video format",
                self.video_codec.as_str()
            )));
        }
        Ok(answer)
    }

    pub fn register_video_limit(&mut self, state: Arc<SharedState>) {
        if self.video_codec == VideoCodec::H264 {
            if let Some(level) = self.h264_level {
                state.set_video_receive_level(&self.id, level);
                self.video_limit_owner = Some(state);
            }
        }
    }

    /// Write a video RTP packet from GStreamer into str0m.
    pub fn write_video_rtp(&mut self, rtp_data: &[u8]) -> Result<(), WebRTCError> {
        let mid = match self.video_mid {
            Some(mid) => mid,
            None => return Ok(()), // Not yet negotiated
        };

        if rtp_data.len() < 12 {
            return Ok(()); // Too small
        }

        // Use the negotiated PT from SDP, not the GStreamer-assigned one.
        // GStreamer uses PT 96 by default for H.264, but str0m may have
        // mapped PT 96 to VP8 — we must use the actual H.264 PT from SDP.
        let pt = match self.video_pt {
            Some(pt) => pt,
            None => return Ok(()),
        };
        let marker = rtp_util::is_marker_set(rtp_data);
        let source_timestamp = rtp_util::get_timestamp(rtp_data).unwrap_or(0);
        let source = rtp_util::get_ssrc(rtp_data).unwrap_or(0);
        let header_len = rtp_util::header_length(rtp_data).unwrap_or(12);
        let payload = if rtp_data.len() > header_len {
            rtp_data[header_len..].to_vec()
        } else {
            return Ok(());
        };

        if self.video_codec == VideoCodec::H264 {
            if self.video_clock.source != Some(source) {
                self.h264_ready = false;
            }
            // New sessions may subscribe while the common pipeline is being
            // resized for their receive limit. Wait for a compatible SPS instead
            // of feeding old, higher-level frames into the browser decoder.
            if payload[0] & 0x1f == 7 {
                self.h264_ready = payload.len() >= 4
                    && constrained_baseline(u32::from_be_bytes([
                        0, payload[1], payload[2], payload[3],
                    ]))
                    && self.h264_level.is_some_and(|level| payload[3] <= level);
                if !self.h264_ready {
                    warn!("Session {} waiting for compatible H.264 SPS", self.id);
                }
            }
            if !self.h264_ready {
                return Ok(());
            }
        }

        let timestamp = self
            .video_clock
            .map(source, source_timestamp, Instant::now());
        let seq = SeqNo::from(self.video_seq);
        self.video_seq += 1;

        if let Some(stream_tx) = self.rtc.direct_api().stream_tx_by_mid(mid, None) {
            let result = stream_tx.write_rtp(
                pt,
                seq,
                timestamp,
                Instant::now(),
                marker,
                str0m::rtp::ExtensionValues::default(),
                true, // nackable
                payload,
            );
            if self.video_seq == 1 {
                info!(
                    "Session {} first write_rtp pt={:?} ok={}",
                    self.id,
                    pt,
                    result.is_ok()
                );
            }
            result.map_err(|error| WebRTCError::MediaError(error.to_string()))?;
        } else if self.video_seq == 1 {
            warn!(
                "Session {} stream_tx_by_mid({:?}) returned None",
                self.id, mid
            );
        }

        Ok(())
    }

    /// Write an audio RTP packet (Opus) into str0m.
    pub fn write_audio_rtp(&mut self, opus_data: &[u8], timestamp: u32) -> Result<(), WebRTCError> {
        let mid = match self.audio_mid {
            Some(mid) => mid,
            None => return Ok(()),
        };
        let Some(pt) = self.audio_pt else {
            return Ok(());
        };

        let seq_no = SeqNo::from(self.audio_seq);
        self.audio_seq += 1;

        if let Some(stream_tx) = self.rtc.direct_api().stream_tx_by_mid(mid, None) {
            let _ = stream_tx.write_rtp(
                pt,
                seq_no,
                timestamp,
                Instant::now(),
                false, // continuous audio stream, no silence suppression
                str0m::rtp::ExtensionValues::default(),
                false, // not nackable for audio
                opus_data.to_vec(),
            );
        }

        Ok(())
    }

    /// Send a text message through the DataChannel.
    pub fn send_datachannel_text(&mut self, text: &str) -> Result<(), WebRTCError> {
        let dc_id = match self.dc_id {
            Some(id) => id,
            None => {
                return Err(WebRTCError::DataChannelError(
                    "DataChannel not open".to_string(),
                ))
            }
        };

        if let Some(mut channel) = self.rtc.channel(dc_id) {
            channel
                .write(false, text.as_bytes())
                .map_err(|e| WebRTCError::DataChannelError(format!("DC write failed: {}", e)))?;
        }

        Ok(())
    }
}

impl Drop for RtcSession {
    fn drop(&mut self) {
        if let Some(state) = self.video_limit_owner.take() {
            state.remove_video_receive_level(&self.id);
        }
    }
}

fn negotiated_payload(rtc: &Rtc, mid: Mid, codec: str0m::format::Codec) -> Option<Pt> {
    let media = rtc.media(mid)?;
    media.remote_pts().iter().copied().find(|pt| {
        rtc.codec_config()
            .params()
            .iter()
            .any(|p| p.pt() == *pt && p.spec().codec == codec)
    })
}

#[cfg(test)]
mod video_tests {
    use super::*;
    use str0m::media::Direction;

    fn offer_at_pt(profile: u32, packetization: bool, pt: u8) -> (String, Mid) {
        let mut builder = Rtc::builder().clear_codecs().enable_opus(true);
        builder
            .codec_config()
            .add_h264(pt.into(), Some((pt + 1).into()), packetization, profile);
        let mut rtc = builder.build(Instant::now());
        let mut changes = rtc.sdp_api();
        let mid = changes.add_media(MediaKind::Video, Direction::RecvOnly, None, None, None);
        changes.add_media(MediaKind::Audio, Direction::RecvOnly, None, None, None);
        let (offer, _) = changes.apply().unwrap();
        (offer.to_sdp_string(), mid)
    }

    fn offer(profile: u32, packetization: bool) -> (String, Mid) {
        offer_at_pt(profile, packetization, 96)
    }

    #[test]
    fn constrained_baseline_only_uses_the_offered_pt_and_level() {
        for (profile, pt) in [(0x42e01f, 96), (0x42e02a, 102), (0x42e034, 108)] {
            let (sdp, mid) = offer_at_pt(profile, true, pt);
            let mut session = RtcSession::new("video-test".into(), VideoCodec::H264, &sdp);
            session
                .add_local_tcp_candidate("127.0.0.1:9999".parse().unwrap())
                .unwrap();
            let answer = session.accept_offer(&sdp).unwrap();
            assert!(answer.contains("m=video 9 "));
            assert_eq!(
                negotiated_payload(&session.rtc, mid, str0m::format::Codec::H264),
                Some(pt.into())
            );
            assert_eq!(
                negotiated_payload(&session.rtc, mid, str0m::format::Codec::Vp8),
                None
            );
            assert!(answer.contains(&format!("profile-level-id={profile:06x}")));
            assert!(!answer.contains("VP8/90000"));
            assert!(!answer.contains("H265/90000"));
        }
    }

    #[test]
    fn incompatible_profile_packetization_and_codec_are_explicit_errors() {
        for (profile, mode, codec) in [
            (0x64001f, true, VideoCodec::H264),
            (0x42e01f, false, VideoCodec::H264),
            (0x42e01f, true, VideoCodec::VP8),
        ] {
            let (sdp, _) = offer(profile, mode);
            let mut session = RtcSession::new("rejected-video".into(), codec, &sdp);
            session
                .add_local_tcp_candidate("127.0.0.1:9999".parse().unwrap())
                .unwrap();
            assert!(session
                .accept_offer(&sdp)
                .unwrap_err()
                .to_string()
                .contains("IVNC_VIDEO_UNSUPPORTED"));
        }
    }

    #[test]
    fn common_output_restores_requested_size_when_a_limited_session_drops() {
        use crate::config::{ui::UiConfig, Config};
        let mut config = Config::default();
        config.display.width = 1920;
        config.display.height = 1080;
        config.encoding.target_fps = 60;
        config.webrtc.video_bitrate = 8000;
        let (input, _) = tokio::sync::mpsc::unbounded_channel();
        let state = Arc::new(SharedState::new(
            config.clone(),
            UiConfig::from_env(&config),
            input,
            Arc::new(RuntimeSettings::new(&config)),
            None,
            None,
        ));
        let (sdp, _) = offer(0x42e034, true);
        let mut high = RtcSession::new("high".into(), VideoCodec::H264, &sdp);
        high.register_video_limit(state.clone());
        assert_eq!(state.video_parameters(1920, 1080), (1920, 1080, 60, 8000));
        let (sdp, _) = offer(0x42e01f, true);
        let mut low = RtcSession::new("pending-low".into(), VideoCodec::H264, &sdp);
        low.register_video_limit(state.clone());
        let (w, h, fps, _) = state.video_parameters(1920, 1080);
        assert!(super::super::video_format::h264_level(31)
            .unwrap()
            .fits(w, h, fps));
        assert!(w < 1920 && h < 1080);
        assert_eq!(state.display_size(), (1920, 1080));
        drop(low); // Includes pending sessions that never finish connecting.
        assert_eq!(state.video_parameters(1920, 1080), (1920, 1080, 60, 8000));
        drop(high);
        assert_eq!(state.video_receive_level().id, 52);
    }
}

/// Drive a single RtcSession's event loop over a TCP connection.
///
/// This function runs as a tokio task for each connected peer.
/// It handles:
/// - TCP read → RFC 4571 decode → str0m handle_input
/// - str0m poll_output → RFC 4571 encode → TCP write
/// - RTP broadcast → str0m write_rtp (video)
/// - Audio broadcast → str0m write_rtp (audio)
/// - Text broadcast → DataChannel write
/// - DataChannel events → input_tx

pub async fn drive_session(
    mut session: RtcSession,
    mut tcp_stream: TcpStream,
    peer_addr: SocketAddr,
    local_addr: SocketAddr,
    shared_state: Arc<SharedState>,
    input_tx: mpsc::UnboundedSender<InputEventData>,
    upload_handler: Arc<Mutex<FileUploadHandler>>,
    clipboard: Arc<Mutex<ClipboardReceiver>>,
    runtime_settings: Arc<RuntimeSettings>,
    initial_buffer: Vec<u8>,
    mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    auth: crate::web::auth::AuthContext,
) {
    let session_id = session.id.clone();
    info!(
        "Session {} drive loop started (peer: {})",
        session_id, peer_addr
    );

    // Disable Nagle's algorithm for low-latency RTP delivery
    if let Err(e) = tcp_stream.set_nodelay(true) {
        warn!("Session {} failed to set TCP_NODELAY: {}", session_id, e);
    }

    let last_pong = Arc::new(AtomicU64::new(now_millis()));
    let (paste_reply_tx, mut paste_reply_rx) = mpsc::unbounded_channel();
    let paste = Mutex::new(crate::clipboard::paste::PasteReceiver::new(
        shared_state.config.input.enable_clipboard,
        input_tx.clone(),
        paste_reply_tx,
    ));
    let ctx = EventContext {
        input_tx: &input_tx,
        upload_handler: &upload_handler,
        clipboard: &clipboard,
        runtime_settings: &runtime_settings,
        shared_state: &shared_state,
        last_pong: &last_pong,
        paste: &paste,
    };

    let mut decoder = TcpFrameDecoder::new();
    if !initial_buffer.is_empty() {
        decoder.extend(&initial_buffer);
    }
    let mut buf = vec![0u8; 65535];

    // Use mpsc subscribers (reliable cross-thread wakeup, unlike broadcast)
    let mut rtp_rx = shared_state.subscribe_rtp_mpsc();
    let mut audio_rx = shared_state.subscribe_audio_mpsc();
    let mut text_rx = shared_state.subscribe_text_mpsc();

    // Audio RTP state
    let mut audio_timestamp: u32 = 0;
    let samples_per_frame: u32 = 960; // Opus 20ms @ 48kHz

    // Keepalive settings
    let mut ping_interval = tokio::time::interval(Duration::from_secs(15));
    let pong_timeout = Duration::from_secs(45);

    // Initial timeout — will be set by drain_outputs
    let mut next_timeout;

    // Drain initial poll_output to get the first timeout
    match drain_outputs(&mut session, &mut tcp_stream, &ctx).await {
        Ok(t) => next_timeout = t,
        Err(e) => {
            error!("Session {} initial drain failed: {}", session_id, e);
            return;
        }
    }

    let auth_cancelled = auth.cancelled();
    tokio::pin!(auth_cancelled);
    loop {
        let delay = next_timeout.saturating_duration_since(Instant::now());
        let mut fatal = false;

        tokio::select! {
            biased;

            _ = &mut auth_cancelled => {
                info!("Session {} authentication ended", session_id);
                break;
            }

            // Shutdown signal from disconnect handler
            _ = &mut shutdown_rx => {
                info!("Session {} received shutdown signal", session_id);
                break;
            }

            // TCP data from browser (highest priority — STUN consent checks,
            // DTLS, SCTP/DataChannel messages MUST be processed promptly or
            // the browser will declare the connection dead)
            result = tcp_stream.read(&mut buf) => {
                match result {
                    Ok(0) => {
                        info!("Session {} TCP connection closed", session_id);
                        break;
                    }
                    Ok(n) => {
                        decoder.extend(&buf[..n]);
                        loop {
                            match decoder.next_packet() {
                                Ok(Some(pkt)) => {
                                    let recv = net::Receive {
                                        proto: Protocol::Tcp,
                                        source: peer_addr,
                                        destination: local_addr,
                                        contents: match (&*pkt).try_into() {
                                            Ok(c) => c,
                                            Err(e) => {
                                                debug!("Session {} packet parse error: {}", session_id, e);
                                                continue;
                                            }
                                        },
                                    };
                                    if let Err(e) = session.rtc.handle_input(Input::Receive(Instant::now(), recv)) {
                                        warn!("Session {} handle_input error: {}", session_id, e);
                                        fatal = true;
                                        break;
                                    }
                                }
                                Ok(None) => break,
                                Err(e) => {
                                    warn!("Session {} invalid RFC 4571 frame: {:?}", session_id, e);
                                    fatal = true;
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Session {} TCP read error: {}", session_id, e);
                        break;
                    }
                }
            }

            // Video RTP from GStreamer → str0m
            result = rtp_rx.recv() => {
                match result {
                    Some(pkt) if session.connected => {
                        let _ = session.write_video_rtp(&pkt);
                        // Drain all pending RTP packets in one go
                        while let Ok(pkt) = rtp_rx.try_recv() {
                            let _ = session.write_video_rtp(&pkt);
                        }
                    }
                    Some(_) => {}
                    None => break,
                }
            }

            // Audio RTP → str0m
            result = audio_rx.recv() => {
                match result {
                    Some(pkt) if session.connected => {
                        let _ = session.write_audio_rtp(&pkt.data, audio_timestamp);
                        audio_timestamp = audio_timestamp.wrapping_add(samples_per_frame);
                        // Drain all pending audio packets in one go
                        while let Ok(pkt) = audio_rx.try_recv() {
                            let _ = session.write_audio_rtp(&pkt.data, audio_timestamp);
                            audio_timestamp = audio_timestamp.wrapping_add(samples_per_frame);
                        }
                    }
                    Some(_) => {}
                    None => break,
                }
            }

            // Timeout — str0m needs periodic timeout handling
            _ = tokio::time::sleep(delay) => {
                if let Err(e) = session.rtc.handle_input(Input::Timeout(Instant::now())) {
                    warn!("Session {} timeout error: {}", session_id, e);
                    break;
                }
            }

            // Text messages (cursor, clipboard, stats) → DataChannel
            result = text_rx.recv() => {
                match result {
                    Some(msg) => {
                        if session.connected {
                            let _ = session.send_datachannel_text(&msg);
                        }
                    }
                    None => break,
                }
            }

            // Clipboard capabilities/results belong only to the requesting connection.
            Some(reply) = paste_reply_rx.recv() => {
                let _ = session.send_datachannel_text(&reply);
            }

            // Keepalive ping
            _ = ping_interval.tick() => {
                if session.connected {
                    let _ = session.send_datachannel_text("ping");
                }
            }
        }

        // After any event, drain str0m outputs
        if fatal {
            break;
        }
        if session.connected {
            let last = last_pong.load(Ordering::Relaxed);
            let now = now_millis();
            if now.saturating_sub(last) > pong_timeout.as_millis() as u64 {
                warn!("Session {} pong timeout; closing connection", session_id);
                break;
            }
        }
        // str0m Sans-I/O requires a Timeout input to timestamp queued RTP
        // packets so the pacer can emit them.  Without this, write_rtp()
        // packets sit in the send queue with a sentinel timestamp and are
        // never transmitted.
        let _ = session.rtc.handle_input(Input::Timeout(Instant::now()));
        match drain_outputs(&mut session, &mut tcp_stream, &ctx).await {
            Ok(t) => next_timeout = t,
            Err(e) => {
                warn!("Session {} drain error: {}", session_id, e);
                break;
            }
        }
    }

    info!("Session {} drive loop ended", session_id);
    paste.lock().unwrap_or_else(|e| e.into_inner()).cancel_all();
    let _ = input_tx.send(InputEventData {
        event_type: InputEvent::KeyboardReset,
        ..Default::default()
    });
    shared_state.remove_connection(&session_id);
    shared_state.decrement_webrtc_sessions();
}

/// Drain all pending str0m outputs: transmit packets, handle events, get next timeout.
///
/// str0m's NullPacer emits one RTP packet per handle_timeout→poll_output cycle.
/// We loop: poll_output until Timeout, then re-trigger handle_input(Timeout) to
/// refresh the pacer's queue state, and poll again.  We stop when a cycle
/// produces no Transmit outputs (only Timeout).
async fn drain_outputs(
    session: &mut RtcSession,
    tcp_stream: &mut TcpStream,
    ctx: &EventContext<'_>,
) -> Result<Instant, WebRTCError> {
    let mut next_timeout;
    let mut cycles = 0u32;
    const MAX_DRAIN_CYCLES: u32 = 512;

    loop {
        let mut had_transmit = false;

        // Inner loop: drain poll_output until Timeout
        loop {
            match session.rtc.poll_output() {
                Ok(Output::Transmit(t)) => {
                    had_transmit = true;
                    let framed = frame_packet(&t.contents);
                    if let Err(e) =
                        tokio::time::timeout(Duration::from_secs(10), tcp_stream.write_all(&framed))
                            .await
                            .map_err(|_| {
                                std::io::Error::new(
                                    std::io::ErrorKind::TimedOut,
                                    "slow media client",
                                )
                            })
                            .and_then(|v| v)
                    {
                        return Err(WebRTCError::ConnectionFailed(format!("TCP write: {}", e)));
                    }
                }
                Ok(Output::Event(event)) => {
                    handle_event(session, event, ctx);
                }
                Ok(Output::Timeout(t)) => {
                    next_timeout = t;
                    break;
                }
                Err(e) => {
                    return Err(WebRTCError::ConnectionFailed(format!("poll_output: {}", e)));
                }
            }
        }

        cycles += 1;
        if !had_transmit || cycles >= MAX_DRAIN_CYCLES {
            break;
        }

        // Re-trigger timeout so the NullPacer refreshes its queue snapshot
        // and can emit the next queued packet.
        if let Err(e) = session.rtc.handle_input(Input::Timeout(Instant::now())) {
            warn!("Session {} drain re-trigger error: {}", session.id, e);
            break;
        }
    }

    Ok(next_timeout)
}

/// Context passed to event handlers so they can dispatch DataChannel messages.
struct EventContext<'a> {
    input_tx: &'a mpsc::UnboundedSender<InputEventData>,
    upload_handler: &'a Arc<Mutex<FileUploadHandler>>,
    clipboard: &'a Arc<Mutex<ClipboardReceiver>>,
    runtime_settings: &'a Arc<RuntimeSettings>,
    shared_state: &'a Arc<SharedState>,
    last_pong: &'a Arc<AtomicU64>,
    paste: &'a Mutex<crate::clipboard::paste::PasteReceiver>,
}

/// Handle a str0m event.
fn handle_event(session: &mut RtcSession, event: Event, ctx: &EventContext) {
    match event {
        Event::Connected => {
            session.connected = true;
            info!("Session {} WebRTC connected", session.id);
        }

        Event::MediaAdded(media) => match media.kind {
            MediaKind::Video => {
                session.video_mid = Some(media.mid);
                session.video_pt =
                    negotiated_payload(&session.rtc, media.mid, rtc_codec(session.video_codec));
                info!(
                    "Session {} video PT: {:?} ({:?}), receive level {:?}",
                    session.id, session.video_pt, session.video_codec, session.h264_level
                );
                info!("Session {} video mid: {:?}", session.id, media.mid);
            }
            MediaKind::Audio => {
                session.audio_mid = Some(media.mid);
                session.audio_pt =
                    negotiated_payload(&session.rtc, media.mid, str0m::format::Codec::Opus);
                info!("Session {} audio mid: {:?}", session.id, media.mid);
            }
        },

        Event::IceConnectionStateChange(state) => {
            info!("Session {} ICE state: {:?}", session.id, state);
            if state == IceConnectionState::Disconnected {
                session.connected = false;
            }
        }

        Event::ChannelOpen(id, label) => {
            // File upload's auxiliary channel must not steal clipboard replies.
            if label != "data" {
                session.dc_id = Some(id);
            }
            info!(
                "Session {} DataChannel '{}' opened (id={:?})",
                session.id, label, id
            );
            ctx.shared_state
                .datachannel_open_count
                .fetch_add(1, Ordering::Relaxed);
        }

        Event::ChannelData(data) => {
            handle_datachannel_data(session, data, ctx);
        }

        Event::ChannelClose(id) => {
            if session.dc_id == Some(id) {
                ctx.paste
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .cancel_all();
                let _ = ctx.input_tx.send(InputEventData {
                    event_type: InputEvent::KeyboardReset,
                    ..Default::default()
                });
                session.dc_id = None;
            }
            info!("Session {} DataChannel closed (id={:?})", session.id, id);
        }

        Event::KeyframeRequest(_) => {
            ctx.shared_state.request_keyframe();
        }

        _ => {
            debug!("Session {} unhandled event: {:?}", session.id, event);
        }
    }
}

/// Handle incoming DataChannel data — reuses the existing input parsing logic.
fn handle_datachannel_data(session: &mut RtcSession, data: ChannelData, ctx: &EventContext) {
    if data.binary {
        // Binary data → file upload handler
        ctx.upload_handler
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .handle_binary(&data.data);
        return;
    }

    let text = match std::str::from_utf8(&data.data) {
        Ok(t) => t,
        Err(e) => {
            debug!("Session {} DC invalid UTF-8: {}", session.id, e);
            return;
        }
    };

    if text.starts_with("pong") {
        ctx.last_pong.store(now_millis(), Ordering::Relaxed);
        return;
    }

    if text.starts_with("paste,") && session.dc_id != Some(data.id) {
        return;
    }
    if ctx
        .paste
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .allow_binary(ctx.shared_state.runtime_settings.binary_clipboard_enabled())
        .handle(text)
    {
        ctx.shared_state
            .record_connection_input_activity(&session.id);
        return;
    }

    // Try specialized handlers first
    if ctx
        .upload_handler
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .handle_control_message(text)
    {
        return;
    }
    if ctx
        .clipboard
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .handle_message(text)
    {
        return;
    }
    if ctx.shared_state.handle_command_message(text) {
        return;
    }
    if text.starts_with("SETTINGS,") {
        let payload = text.trim_start_matches("SETTINGS,");
        ctx.runtime_settings.apply_settings_json(payload);
        return;
    }
    if text.starts_with("_client_info,") {
        let payload = text.trim_start_matches("_client_info,");
        ctx.shared_state
            .update_connection_client_info(&session.id, payload);
        return;
    }
    if text.starts_with("_client_activity,") {
        let payload = text.trim_start_matches("_client_activity,");
        ctx.shared_state
            .update_connection_client_activity(&session.id, payload);
        return;
    }
    if ctx.runtime_settings.handle_simple_message(text) {
        return;
    }
    if text == "kr" {
        let _ = ctx.input_tx.send(InputEventData {
            event_type: InputEvent::KeyboardReset,
            ..Default::default()
        });
        return;
    }
    if text.starts_with("s,") || text.starts_with("SET_NATIVE_CURSOR_RENDERING,") {
        return;
    }
    if text == "_display,1" {
        let _ = session.send_datachannel_text(&ctx.shared_state.display_message());
        return;
    }
    if let Some(viewport) = text.strip_prefix("_viewport,") {
        ctx.shared_state.report_viewport(&session.id, viewport);
        return;
    }
    if text.starts_with("r,") {
        // Both old resize messages and new viewport reports are hints only.
        // An untrusted viewer can never resize the shared compositor.
        return;
    }
    if text.starts_with("_arg_fps,") {
        if let Ok(fps) = text.trim_start_matches("_arg_fps,").parse::<u32>() {
            ctx.runtime_settings.set_target_fps(fps);
        }
        return;
    }
    if text.starts_with("_f,") {
        if let Ok(fps) = text.trim_start_matches("_f,").parse::<u32>() {
            ctx.shared_state.update_client_fps(fps);
        }
        return;
    }
    if text.starts_with("_l,") {
        if let Ok(latency) = text.trim_start_matches("_l,").parse::<u64>() {
            ctx.shared_state.update_client_latency(latency);
        }
        return;
    }
    if text.starts_with("_stats_video,") {
        ctx.shared_state
            .update_webrtc_stats("video", text.trim_start_matches("_stats_video,"));
        return;
    }
    if text.starts_with("_stats_audio,") {
        ctx.shared_state
            .update_webrtc_stats("audio", text.trim_start_matches("_stats_audio,"));
        return;
    }
    if let Some(value) = text.strip_prefix("focus-key,") {
        if let Ok(window_key) = uuid::Uuid::parse_str(value) {
            let event = InputEventData {
                event_type: InputEvent::WindowFocusStable,
                window_key: Some(window_key),
                ..Default::default()
            };
            let _ = ctx.input_tx.send(event);
        }
        return;
    }
    if text.starts_with("focus,") {
        if let Ok(window_id) = text.trim_start_matches("focus,").parse::<u32>() {
            let mut event = InputEventData::default();
            event.event_type = InputEvent::WindowFocus;
            event.window_id = window_id;
            let _ = ctx.input_tx.send(event);
        }
        return;
    }
    if text.starts_with("close,") {
        if let Ok(window_id) = text.trim_start_matches("close,").parse::<u32>() {
            let mut event = InputEventData::default();
            event.event_type = InputEvent::WindowClose;
            event.window_id = window_id;
            let _ = ctx.input_tx.send(event);
        }
        return;
    }

    // Fall through to input event parsing (mouse, keyboard, etc.)
    match InputDataChannel::parse_display_input(text, &ctx.shared_state.display.snapshot()) {
        Ok(event) => {
            ctx.shared_state
                .record_connection_input_activity(&session.id);
            let _ = ctx.input_tx.send(event);
        }
        Err(e) => {
            debug!("Session {} DC parse error: {}", session.id, e);
        }
    }
}

fn now_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
