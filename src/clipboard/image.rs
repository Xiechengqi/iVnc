//! Bounded image decoding away from the compositor. Only one decoder runs,
//! with one waiting job; replacing selections cannot create unbounded threads.
use std::{
    io::Cursor,
    sync::{mpsc, OnceLock},
};

type Records = Vec<(String, Vec<u8>)>;
pub type Result = std::result::Result<Records, &'static str>;
pub type Receiver = mpsc::Receiver<Result>;
struct Job {
    records: Records,
    reply: mpsc::SyncSender<Result>,
}
const MAX_DIMENSION: u32 = 8192;
const MAX_PIXELS: u64 = 16 * 1024 * 1024;

pub fn convert(records: Records) -> std::result::Result<Receiver, &'static str> {
    static WORKER: OnceLock<mpsc::SyncSender<Job>> = OnceLock::new();
    let worker = WORKER.get_or_init(|| {
        let (tx, rx) = mpsc::sync_channel::<Job>(1);
        std::thread::Builder::new()
            .name("clipboard-image".into())
            .spawn(move || {
                for job in rx {
                    // A dropped receiver discards the result of a superseded selection.
                    let _ = job.reply.send(normalize(job.records));
                }
            })
            .expect("clipboard image worker");
        tx
    });
    let (reply, rx) = mpsc::sync_channel(1);
    worker
        .try_send(Job { records, reply })
        .map_err(|_| "busy")?;
    Ok(rx)
}

fn normalize(records: Records) -> Result {
    let mut result = Vec::new();
    let mut total = 0usize;
    for (mime, bytes) in records {
        let (mime, bytes) = if mime.starts_with("image/") {
            let format = match mime.as_str() {
                "image/png" => image::ImageFormat::Png,
                "image/jpeg" => image::ImageFormat::Jpeg,
                "image/webp" => image::ImageFormat::WebP,
                _ => return Err("format"),
            };
            let mut reader = image::ImageReader::with_format(Cursor::new(&bytes), format);
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(MAX_DIMENSION);
            limits.max_image_height = Some(MAX_DIMENSION);
            limits.max_alloc = Some(MAX_PIXELS * 4);
            reader.limits(limits);
            let (width, height) = image::ImageReader::with_format(Cursor::new(&bytes), format)
                .into_dimensions()
                .map_err(|_| "invalid_image")?;
            if width == 0
                || height == 0
                || width > MAX_DIMENSION
                || height > MAX_DIMENSION
                || u64::from(width) * u64::from(height) > MAX_PIXELS
            {
                return Err("image_limit");
            }
            let decoded = reader.decode().map_err(|error| {
                if matches!(error, image::ImageError::Limits(_)) {
                    "image_limit"
                } else {
                    "invalid_image"
                }
            })?;
            let png = if format == image::ImageFormat::Png {
                bytes
            } else {
                let mut output = Cursor::new(Vec::new());
                decoded
                    .write_to(&mut output, image::ImageFormat::Png)
                    .map_err(|_| "invalid_image")?;
                output.into_inner()
            };
            ("image/png".to_owned(), png)
        } else {
            if std::str::from_utf8(&bytes).is_err() {
                return Err("invalid_content");
            }
            (mime, bytes)
        };
        total = total.saturating_add(bytes.len());
        if total > super::MAX_CLIPBOARD_BYTES {
            return Err("limit");
        }
        result.push((mime, bytes));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn image_snapshots_decode_and_convert_without_losing_png_bytes() {
        let img = image::DynamicImage::new_rgb8(3, 2);
        for format in [
            image::ImageFormat::Png,
            image::ImageFormat::Jpeg,
            image::ImageFormat::WebP,
        ] {
            let mut encoded = Cursor::new(Vec::new());
            img.write_to(&mut encoded, format).unwrap();
            let bytes = encoded.into_inner();
            let result = normalize(vec![(format.to_mime_type().into(), bytes.clone())]).unwrap();
            assert_eq!(result[0].0, "image/png");
            assert_eq!(image::load_from_memory(&result[0].1).unwrap().width(), 3);
            if format == image::ImageFormat::Png {
                assert_eq!(result[0].1, bytes);
            }
        }
        assert_eq!(
            normalize(vec![("image/png".into(), b"\x89PNG\r\n\x1a\n".to_vec())]),
            Err("invalid_image")
        );
        let huge = image::DynamicImage::new_luma8(8193, 1);
        let mut encoded = Cursor::new(Vec::new());
        huge.write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        assert_eq!(
            normalize(vec![("image/png".into(), encoded.into_inner())]),
            Err("image_limit")
        );
    }
}
