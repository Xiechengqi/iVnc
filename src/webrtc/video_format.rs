//! The common H.264 stream must fit every receiver's negotiated level.
//! Limits are in macroblocks, per H.264 Annex A (8-bit 4:2:0).

use crate::config::VideoCodec;

pub const MAX_H264_LEVEL: u8 = 52;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct H264Level {
    pub id: u8,
    pub name: &'static str,
    pub max_fs: u32,
    pub max_mbps: u32,
    pub max_kbps: u32,
}

const LEVELS: &[H264Level] = &[
    H264Level {
        id: 10,
        name: "1",
        max_fs: 99,
        max_mbps: 1485,
        max_kbps: 64,
    },
    H264Level {
        id: 11,
        name: "1.1",
        max_fs: 396,
        max_mbps: 3000,
        max_kbps: 192,
    },
    H264Level {
        id: 12,
        name: "1.2",
        max_fs: 396,
        max_mbps: 6000,
        max_kbps: 384,
    },
    H264Level {
        id: 13,
        name: "1.3",
        max_fs: 396,
        max_mbps: 11880,
        max_kbps: 768,
    },
    H264Level {
        id: 20,
        name: "2",
        max_fs: 396,
        max_mbps: 11880,
        max_kbps: 2000,
    },
    H264Level {
        id: 21,
        name: "2.1",
        max_fs: 792,
        max_mbps: 19800,
        max_kbps: 4000,
    },
    H264Level {
        id: 22,
        name: "2.2",
        max_fs: 1620,
        max_mbps: 20250,
        max_kbps: 4000,
    },
    H264Level {
        id: 30,
        name: "3",
        max_fs: 1620,
        max_mbps: 40500,
        max_kbps: 10000,
    },
    H264Level {
        id: 31,
        name: "3.1",
        max_fs: 3600,
        max_mbps: 108000,
        max_kbps: 14000,
    },
    H264Level {
        id: 32,
        name: "3.2",
        max_fs: 5120,
        max_mbps: 216000,
        max_kbps: 20000,
    },
    H264Level {
        id: 40,
        name: "4",
        max_fs: 8192,
        max_mbps: 245760,
        max_kbps: 20000,
    },
    H264Level {
        id: 41,
        name: "4.1",
        max_fs: 8192,
        max_mbps: 245760,
        max_kbps: 50000,
    },
    H264Level {
        id: 42,
        name: "4.2",
        max_fs: 8704,
        max_mbps: 522240,
        max_kbps: 50000,
    },
    H264Level {
        id: 50,
        name: "5",
        max_fs: 22080,
        max_mbps: 589824,
        max_kbps: 135000,
    },
    H264Level {
        id: 51,
        name: "5.1",
        max_fs: 36864,
        max_mbps: 983040,
        max_kbps: 240000,
    },
    H264Level {
        id: 52,
        name: "5.2",
        max_fs: 36864,
        max_mbps: 2073600,
        max_kbps: 240000,
    },
];

pub fn h264_level(id: u8) -> Option<H264Level> {
    LEVELS.iter().find(|level| level.id == id).copied()
}

pub fn macroblocks(width: u32, height: u32) -> u32 {
    width.div_ceil(16).saturating_mul(height.div_ceil(16))
}

impl H264Level {
    pub fn fits(&self, width: u32, height: u32, fps: u32) -> bool {
        let fs = macroblocks(width, height);
        fs <= self.max_fs
            && fs.saturating_mul(fps) <= self.max_mbps
            && width.div_ceil(16).saturating_pow(2) <= 8 * self.max_fs
            && height.div_ceil(16).saturating_pow(2) <= 8 * self.max_fs
    }

    /// Keep the requested aspect ratio and resolution as far as possible, then
    /// cap FPS. This favors readable desktop text over unnecessary frame rate.
    pub fn fit(&self, width: u32, height: u32, fps: u32) -> (u32, u32, u32) {
        let (mut w, mut h) = (width.max(2) & !1, height.max(2) & !1);
        while !self.fits(w, h, 1) {
            let scale = (self.max_fs as f64 / macroblocks(w, h) as f64)
                .sqrt()
                .min(0.99);
            w = ((w as f64 * scale) as u32).max(2) & !1;
            h = ((h as f64 * scale) as u32).max(2) & !1;
        }
        (w, h, fps.max(1).min(self.max_mbps / macroblocks(w, h)))
    }
}

pub fn required_h264_level(width: u32, height: u32, fps: u32, kbps: u32) -> Option<H264Level> {
    LEVELS
        .iter()
        .find(|level| level.fits(width, height, fps) && kbps <= level.max_kbps)
        .copied()
}

/// RFC 6184's equivalent representations of Constrained Baseline.
pub fn constrained_baseline(profile: u32) -> bool {
    let idc = (profile >> 16) as u8;
    let flags = (profile >> 8) as u8;
    matches!(idc, 0x42) && flags & 0x4f == 0x40
        || idc == 0x4d && flags & 0x8f == 0x80
        || idc == 0x58 && flags & 0xcf == 0xc0
}

/// Inspect only the first active, receiving video m-line. str0m retains local
/// capabilities after negotiation, so its global codec list is not an offer.
pub fn offered_h264_level(sdp: &str) -> Option<u8> {
    let section = sdp.split("m=").skip(1).find(|section| {
        let mut fields = section.lines().next().unwrap_or("").split_whitespace();
        fields.next() == Some("video") && fields.next() != Some("0")
    })?;
    if section
        .lines()
        .any(|line| matches!(line.trim(), "a=inactive" | "a=sendonly"))
    {
        return None;
    }
    let pts: Vec<&str> = section.lines().next()?.split_whitespace().skip(3).collect();
    section
        .lines()
        .filter_map(|line| {
            let (pt, format) = line.trim().strip_prefix("a=fmtp:")?.split_once(' ')?;
            if !pts.contains(&pt)
                || !section.lines().any(|line| {
                    line.trim()
                        .strip_prefix(&format!("a=rtpmap:{pt} "))
                        .is_some_and(|value| value.eq_ignore_ascii_case("H264/90000"))
                })
            {
                return None;
            }
            let mut mode = None;
            let mut profile = Some(0x42e01f);
            for field in format.split(';') {
                if let Some((key, value)) = field.trim().split_once('=') {
                    match key {
                        "packetization-mode" => mode = value.parse::<u8>().ok(),
                        "profile-level-id" => {
                            profile = (value.len() == 6)
                                .then(|| u32::from_str_radix(value, 16).ok())
                                .flatten();
                        }
                        _ => {}
                    }
                }
            }
            let profile = profile?;
            let id = profile as u8;
            // Level 1b has a distinct constraint flag and is deliberately unsupported.
            (mode == Some(1)
                && constrained_baseline(profile)
                && !(id == 11 && profile & 0x1000 != 0))
                .then(|| h264_level(id).map(|level| level.id))
                .flatten()
        })
        .max()
}

pub fn rtc_codec(codec: VideoCodec) -> str0m::format::Codec {
    match codec {
        VideoCodec::H264 => str0m::format::Codec::H264,
        VideoCodec::VP8 => str0m::format::Codec::Vp8,
        VideoCodec::VP9 => str0m::format::Codec::Vp9,
        VideoCodec::AV1 => str0m::format::Codec::Av1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_cover_desktop_sizes_and_fit_portrait_and_ultrawide() {
        assert_eq!(required_h264_level(1920, 1080, 60, 8000).unwrap().id, 42);
        assert_eq!(required_h264_level(3840, 2160, 60, 16000).unwrap().id, 52);
        let level = h264_level(31).unwrap();
        assert_eq!(level.fit(1280, 720, 60), (1280, 720, 30));
        for (w, h) in [(1920, 1080), (1080, 1920), (8192, 100), (100, 8192)] {
            let (w, h, fps) = level.fit(w, h, 60);
            assert!(level.fits(w, h, fps));
            assert_eq!(w % 2 + h % 2, 0);
        }
    }

    #[test]
    fn offer_uses_receiving_video_payloads_and_supported_profiles_only() {
        let offer = "v=0\r\nm=video 9 UDP/TLS/RTP/SAVPF 96 98\r\na=recvonly\r\na=rtpmap:96 H264/90000\r\na=fmtp:96 packetization-mode=1;profile-level-id=42e01f\r\na=rtpmap:98 H264/90000\r\na=fmtp:98 packetization-mode=1;profile-level-id=42c02a\r\n";
        assert_eq!(offered_h264_level(offer), Some(42));
        assert_eq!(
            offered_h264_level(&offer.replace("recvonly", "sendonly")),
            None
        );
        assert_eq!(
            offered_h264_level(
                &offer
                    .replace("42e01f", "f4001f")
                    .replace("42c02a", "64002a")
            ),
            None
        );
        assert_eq!(
            offered_h264_level(&offer.replace("packetization-mode=1", "packetization-mode=0")),
            None
        );
        assert_eq!(
            offered_h264_level(&offer.replace("video 9", "video 0")),
            None
        );
        assert_eq!(
            offered_h264_level(
                &offer
                    .replace("42e01f", "invalid")
                    .replace("42c02a", "invalid")
            ),
            None
        );
        assert_eq!(
            offered_h264_level(
                &offer
                    .replace("42e01f", "4d801f")
                    .replace("42c02a", "58c02a")
            ),
            Some(42)
        );
    }
}
