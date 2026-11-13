//! GStreamer video encoding pipeline
//!
//! Provides a complete pipeline for:
//! - Receiving raw frames from compositor via appsrc
//! - Video encoding (H.264, VP8, VP9)

#![allow(dead_code)]
//! - RTP packetization for WebRTC

use super::{encoder::EncoderSelection, GstError};
use crate::config::{HardwareEncoder, VideoCodec, WebRTCConfig};
use crate::webrtc::video_format::required_h264_level;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use log::{debug, info, warn};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Pipeline state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineState {
    /// Pipeline is stopped
    Stopped,
    /// Pipeline is starting
    Starting,
    /// Pipeline is running
    Running,
    /// Pipeline is paused
    Paused,
    /// Pipeline encountered an error
    Error,
}

/// Pipeline configuration
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Compositor input dimensions, independent of encoded output.
    pub source_width: u32,
    pub source_height: u32,
    /// Encoded frame width
    pub width: u32,
    /// Frame height
    pub height: u32,
    /// Target framerate
    pub framerate: u32,
    /// Video codec
    pub codec: VideoCodec,
    /// Target bitrate in kbps
    pub bitrate: u32,
    /// Hardware encoder preference
    pub hardware_encoder: HardwareEncoder,
    /// Keyframe interval in frames
    pub keyframe_interval: u32,
    /// Pipeline latency in ms
    pub latency_ms: u32,
}

impl From<&WebRTCConfig> for PipelineConfig {
    fn from(config: &WebRTCConfig) -> Self {
        Self {
            source_width: 1920,
            source_height: 1080,
            width: 1920,
            height: 1080,
            framerate: 30,
            codec: config.video_codec,
            bitrate: config.video_bitrate,
            hardware_encoder: config.hardware_encoder,
            keyframe_interval: config.keyframe_interval,
            latency_ms: config.pipeline_latency_ms,
        }
    }
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            source_width: 1920,
            source_height: 1080,
            width: 1920,
            height: 1080,
            framerate: 30,
            codec: VideoCodec::H264,
            bitrate: 4000,
            hardware_encoder: HardwareEncoder::Auto,
            keyframe_interval: 60,
            latency_ms: 50,
        }
    }
}

/// RTP packet callback type
pub type RtpCallback = Box<dyn Fn(&[u8], u32, u64) + Send + Sync>;

/// Video pipeline for GStreamer-based encoding
pub struct VideoPipeline {
    pipeline: gst::Pipeline,
    appsrc: gst_app::AppSrc,
    appsink: gst_app::AppSink,
    config: PipelineConfig,
    state: Arc<AtomicBool>,
    frame_count: Arc<AtomicU64>,
    encoder_element: String,
    reported_caps: AtomicBool,
}

impl VideoPipeline {
    /// Create a new video pipeline with appsrc for compositor frame input
    pub fn new(config: PipelineConfig) -> Result<Self, GstError> {
        gst::init().map_err(|e| GstError::InitFailed(e.to_string()))?;

        let pipeline = gst::Pipeline::new();

        // Create appsrc for receiving raw frames from compositor
        let caps_str = format!(
            "video/x-raw,format=BGRx,width={},height={},framerate={}/1",
            config.source_width, config.source_height, config.framerate
        );
        let caps = caps_str
            .parse::<gst::Caps>()
            .map_err(|e| GstError::PipelineFailed(format!("Invalid caps: {}", e)))?;

        let appsrc = gst_app::AppSrc::builder()
            .name("framesrc")
            .caps(&caps)
            .format(gst::Format::Time)
            .is_live(true)
            .do_timestamp(true)
            .build();

        // videoconvert: BGRx -> I420 for encoder
        let convert = gst::ElementFactory::make("videoconvert")
            .build()
            .map_err(|e| {
                GstError::PipelineFailed(format!("Failed to create videoconvert: {}", e))
            })?;

        let scale = gst::ElementFactory::make("videoscale")
            .property("add-borders", false)
            .build()
            .map_err(|e| GstError::PipelineFailed(format!("Failed to create videoscale: {e}")))?;
        let encoder_selection = EncoderSelection::select(config.codec, config.hardware_encoder);
        let (mut encoder, mut encoder_name) =
            encoder_selection.create_encoder(config.bitrate, config.keyframe_interval)?;
        let h264_caps = if config.codec == VideoCodec::H264 {
            let level = required_h264_level(
                config.width,
                config.height,
                config.framerate,
                config.bitrate,
            )
            .ok_or_else(|| {
                GstError::PipelineFailed(
                    "H.264 output exceeds the supported level 5.2 envelope".into(),
                )
            })?;
            let caps = gst::Caps::builder("video/x-h264")
                .field("profile", "constrained-baseline")
                .field("level", level.name)
                .build();
            if encoder
                .static_pad("src")
                .is_none_or(|pad| pad.query_caps(Some(&caps)).is_empty())
            {
                warn!(
                    "Encoder {} cannot produce compatible H.264; using software encoding",
                    encoder_name
                );
                (encoder, encoder_name) =
                    EncoderSelection::select(VideoCodec::H264, HardwareEncoder::Software)
                        .create_encoder(config.bitrate, config.keyframe_interval)?;
            }
            Some(caps)
        } else {
            None
        };
        info!(
            "Using encoder: {} for codec {:?}",
            encoder_name, config.codec
        );

        let payloader = Self::create_payloader(config.codec)?;

        let appsink = gst_app::AppSink::builder()
            .name("rtpsink")
            .sync(false)
            .max_buffers(0)
            .drop(false)
            .build();

        pipeline
            .add_many([
                appsrc.upcast_ref(),
                &convert,
                &scale,
                &encoder,
                &payloader,
                appsink.upcast_ref(),
            ])
            .map_err(|e| GstError::PipelineFailed(format!("Failed to add elements: {}", e)))?;

        // Link: appsrc -> convert -> encoder -> payloader -> appsink
        appsrc
            .upcast_ref::<gst::Element>()
            .link(&convert)
            .map_err(|e| GstError::LinkFailed(format!("appsrc->convert: {}", e)))?;
        // An unconstrained videoconvert can select Y444, causing x264 to emit
        // High 4:4:4 even when SDP negotiated Constrained Baseline. Hardware
        // encoders may prefer NV12; both choices here are 8-bit 4:2:0.
        let raw_caps: gst::Caps = format!(
            "video/x-raw,format=(string){{I420,NV12}},width={},height={}",
            config.width, config.height
        )
        .parse()
        .unwrap();
        convert
            .link(&scale)
            .map_err(|e| GstError::LinkFailed(format!("convert->scale: {e}")))?;
        scale
            .link_filtered(&encoder, &raw_caps)
            .map_err(|e| GstError::LinkFailed(format!("convert->encoder: {}", e)))?;
        let linked = if let Some(caps) = h264_caps.as_ref() {
            encoder.link_filtered(&payloader, caps)
        } else {
            encoder.link(&payloader)
        };
        linked.map_err(|e| GstError::LinkFailed(format!("encoder->payloader: {}", e)))?;
        payloader
            .link(appsink.upcast_ref::<gst::Element>())
            .map_err(|e| GstError::LinkFailed(format!("payloader->appsink: {}", e)))?;

        pipeline.set_latency(gst::ClockTime::from_mseconds(config.latency_ms as u64));

        Ok(Self {
            pipeline,
            appsrc,
            appsink,
            config,
            state: Arc::new(AtomicBool::new(false)),
            frame_count: Arc::new(AtomicU64::new(0)),
            encoder_element: encoder_name,
            reported_caps: AtomicBool::new(false),
        })
    }

    /// Create RTP payloader for the specified codec
    fn create_payloader(codec: VideoCodec) -> Result<gst::Element, GstError> {
        let (element_name, pt) = match codec {
            VideoCodec::H264 => ("rtph264pay", 96),
            VideoCodec::VP8 => ("rtpvp8pay", 97),
            VideoCodec::VP9 => ("rtpvp9pay", 98),
            VideoCodec::AV1 => ("rtpav1pay", 99),
        };

        let mut builder = gst::ElementFactory::make(element_name).property("pt", pt as u32);

        // For H264, ensure SPS/PPS are sent regularly for browser decoders.
        if matches!(codec, VideoCodec::H264) {
            // New and resumed Safari decoders need the parameter sets with
            // every IDR, including after pipeline rebuilds and keyframe requests.
            builder = builder.property("config-interval", -1i32);
        }

        if matches!(codec, VideoCodec::H264) {
            // Keep SPS/PPS as individual NAL units, including for the session's
            // compatibility gate before it starts forwarding a new source.
            builder = builder.property_from_str("aggregate-mode", "none");
        }
        builder.build().map_err(|e| {
            GstError::PipelineFailed(format!("Failed to create {}: {}", element_name, e))
        })
    }

    /// Start the pipeline
    pub fn start(&self) -> Result<(), GstError> {
        info!(
            "Starting GStreamer pipeline with encoder: {}",
            self.encoder_element
        );

        self.pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| GstError::StateChangeFailed(format!("Failed to start pipeline: {}", e)))?;

        self.state.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Stop the pipeline
    pub fn stop(&self) -> Result<(), GstError> {
        info!("Stopping GStreamer pipeline");

        self.state.store(false, Ordering::SeqCst);

        self.pipeline
            .set_state(gst::State::Null)
            .map_err(|e| GstError::StateChangeFailed(format!("Failed to stop pipeline: {}", e)))?;

        Ok(())
    }

    /// Push a raw frame (XRGB8888 / BGRx) into the pipeline via appsrc
    pub fn push_frame(&self, data: &[u8]) -> Result<(), GstError> {
        let mut buffer = gst::Buffer::with_size(data.len())
            .map_err(|e| GstError::PipelineFailed(format!("Buffer alloc failed: {}", e)))?;
        {
            let buffer_ref = buffer.get_mut().unwrap();
            let mut map = buffer_ref
                .map_writable()
                .map_err(|e| GstError::PipelineFailed(format!("Buffer map failed: {}", e)))?;
            map.copy_from_slice(data);
        }
        self.appsrc
            .push_buffer(buffer)
            .map_err(|e| GstError::PipelineFailed(format!("appsrc push failed: {:?}", e)))?;
        self.frame_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Pause the pipeline
    pub fn pause(&self) -> Result<(), GstError> {
        self.pipeline
            .set_state(gst::State::Paused)
            .map_err(|e| GstError::StateChangeFailed(format!("Failed to pause pipeline: {}", e)))?;
        Ok(())
    }

    /// Resume the pipeline
    pub fn resume(&self) -> Result<(), GstError> {
        self.pipeline.set_state(gst::State::Playing).map_err(|e| {
            GstError::StateChangeFailed(format!("Failed to resume pipeline: {}", e))
        })?;
        Ok(())
    }

    /// Check if pipeline is running
    pub fn is_running(&self) -> bool {
        self.state.load(Ordering::SeqCst)
    }

    /// Get the current state
    pub fn state(&self) -> PipelineState {
        let (_, current, _) = self.pipeline.state(gst::ClockTime::from_mseconds(0));
        match current {
            gst::State::Null => PipelineState::Stopped,
            gst::State::Ready => PipelineState::Starting,
            gst::State::Paused => PipelineState::Paused,
            gst::State::Playing => PipelineState::Running,
            _ => PipelineState::Error,
        }
    }

    /// Get the appsink for pulling RTP packets
    pub fn appsink(&self) -> &gst_app::AppSink {
        &self.appsink
    }

    /// Pull a sample from the pipeline (blocking with timeout)
    pub fn pull_sample(&self, _timeout_ms: u64) -> Option<gst::Sample> {
        self.appsink.pull_sample().ok()
    }

    /// Pull a sample (non-blocking)
    pub fn try_pull_sample(&self) -> Option<gst::Sample> {
        let sample = self.appsink.try_pull_sample(gst::ClockTime::ZERO)?;
        if !self.reported_caps.swap(true, Ordering::Relaxed) {
            info!(
                "Encoded video caps ({}): {:?}",
                self.encoder_element,
                sample.caps()
            );
        }
        Some(sample)
    }

    /// Pull a sample with timeout (blocks up to timeout_ms)
    pub fn try_pull_sample_timeout(&self, timeout_ms: u64) -> Option<gst::Sample> {
        self.appsink
            .try_pull_sample(gst::ClockTime::from_mseconds(timeout_ms))
    }

    /// Request a keyframe (IDR)
    pub fn request_keyframe(&self) {
        if let Some(encoder) = self.pipeline.by_name("encoder") {
            // send_event() on an element sends upstream events upstream through sink pads
            let event = gst_video::UpstreamForceKeyUnitEvent::builder()
                .all_headers(true)
                .build();
            if !encoder.send_event(event) {
                warn!("Failed to send force-keyunit event to encoder");
            } else {
                info!("Sent force-keyunit event to encoder for IDR frame");
            }
        } else {
            warn!("No encoder element found for keyframe request");
        }
    }

    /// Update bitrate dynamically
    pub fn set_bitrate(&self, bitrate_kbps: u32) {
        if let Some(encoder) = self.pipeline.by_name("encoder") {
            // Try setting bitrate property (different encoders use different properties)
            if encoder.has_property("bitrate", None) {
                // x264enc uses kbps
                let _ = encoder.set_property("bitrate", bitrate_kbps);
                debug!("Updated encoder bitrate to {} kbps", bitrate_kbps);
            } else if encoder.has_property("target-bitrate", None) {
                // vp8enc/vp9enc use bps
                let _ = encoder.set_property("target-bitrate", bitrate_kbps * 1000);
                debug!(
                    "Updated encoder target-bitrate to {} bps",
                    bitrate_kbps * 1000
                );
            }
        }
    }

    /// Update keyframe interval dynamically (best-effort)
    pub fn set_keyframe_interval(&self, interval: u32) {
        if let Some(encoder) = self.pipeline.by_name("encoder") {
            if encoder.has_property("key-int-max", None) {
                let _ = encoder.set_property("key-int-max", interval as i32);
            } else if encoder.has_property("gop-size", None) {
                let _ = encoder.set_property("gop-size", interval);
            } else if encoder.has_property("keyframe-max-dist", None) {
                let _ = encoder.set_property("keyframe-max-dist", interval as i32);
            } else if encoder.has_property("keyframe-period", None) {
                let _ = encoder.set_property("keyframe-period", interval as i32);
            } else if encoder.has_property("iframeinterval", None) {
                let _ = encoder.set_property("iframeinterval", interval as i32);
            }
            debug!("Updated encoder keyframe interval to {} frames", interval);
        }
    }

    /// Get frame count
    pub fn frame_count(&self) -> u64 {
        self.frame_count.load(Ordering::Relaxed)
    }

    /// Get pipeline configuration
    pub fn config(&self) -> &PipelineConfig {
        &self.config
    }

    /// Get the name of the encoder being used
    pub fn encoder_name(&self) -> &str {
        &self.encoder_element
    }
}

impl Drop for VideoPipeline {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_config_default() {
        let config = PipelineConfig::default();
        assert_eq!(config.framerate, 30);
        assert_eq!(config.bitrate, 4000);
        assert_eq!(config.codec, VideoCodec::H264);
    }

    #[test]
    fn software_h264_emits_420_constrained_baseline_with_parameter_sets() {
        gst::init().unwrap();
        if gst::ElementFactory::find("x264enc").is_none() {
            eprintln!("x264enc unavailable; skipping encoder integration check");
            return;
        }
        let pipeline = VideoPipeline::new(PipelineConfig {
            source_width: 640,
            source_height: 480,
            width: 320,
            height: 240,
            framerate: 30,
            bitrate: 1000,
            hardware_encoder: HardwareEncoder::Software,
            keyframe_interval: 1,
            ..Default::default()
        })
        .unwrap();
        pipeline.start().unwrap();
        let frame = vec![128; 640 * 480 * 4];
        for _ in 0..2 {
            let (mut sps, mut pps, mut idr) = (false, false, false);
            pipeline.push_frame(&frame).unwrap();
            let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
            let mut marker = false;
            while std::time::Instant::now() < until && !marker {
                if let Some(sample) = pipeline.try_pull_sample_timeout(100) {
                    let caps = sample.caps().unwrap().structure(0).unwrap();
                    assert_eq!(caps.get::<&str>("encoding-name").unwrap(), "H264");
                    let data = sample.buffer().unwrap().map_readable().unwrap();
                    let payload = crate::webrtc::media_track::rtp_util::get_payload(&data).unwrap();
                    if payload[0] & 31 == 7 {
                        assert!(crate::webrtc::video_format::constrained_baseline(
                            u32::from_be_bytes([0, payload[1], payload[2], payload[3]])
                        ));
                        assert!(payload[3] <= 30, "SPS must respect the selected level");
                        sps = true;
                    }
                    pps |= payload[0] & 31 == 8;
                    idr |= payload[0] & 31 == 5
                        || (payload[0] & 31 == 28 && payload.len() > 1 && payload[1] & 31 == 5);
                    marker = crate::webrtc::media_track::rtp_util::is_marker_set(&data);
                }
            }
            assert!(marker, "encoder did not produce a complete frame");
            assert!(sps && pps && idr, "every IDR frame must carry SPS/PPS");
        }
        let encoder = pipeline.pipeline.by_name("encoder").unwrap();
        let caps = encoder.static_pad("sink").unwrap().current_caps().unwrap();
        assert_eq!(caps.structure(0).unwrap().get::<i32>("width").unwrap(), 320);
        assert_eq!(
            caps.structure(0).unwrap().get::<i32>("height").unwrap(),
            240
        );
        assert!(matches!(
            caps.structure(0).unwrap().get::<&str>("format").unwrap(),
            "I420" | "NV12"
        ));
    }
}
