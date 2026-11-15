use super::TEXT_LIMIT;
use image::{ImageDecoder, ImageReader};
use std::{
    io::{self, Read},
    path::Path,
};

pub(super) const SOURCE_LIMIT: u64 = 32 * 1024 * 1024;
pub(super) const PIXEL_LIMIT: u64 = 16 * 1024 * 1024;

pub(super) fn image(path: &Path, edge: u32) -> Result<image::RgbaImage, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > SOURCE_LIMIT {
        return Err("画像が内蔵プレビューの読み込み上限（32 MiB）を超えています".into());
    }
    // A growing file cannot make a codec's eager read allocate without limit.
    let mut bytes = Vec::new();
    file.take(SOURCE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > SOURCE_LIMIT {
        return Err("画像の読み込み上限を超えました".into());
    }
    decode_bytes(bytes, edge)
}

fn decode_bytes(bytes: Vec<u8>, edge: u32) -> Result<image::RgbaImage, String> {
    let mut reader = ImageReader::new(io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let (width, height) = decoder.dimensions();
    if width == 0
        || height == 0
        || u64::from(width) * u64::from(height) > PIXEL_LIMIT
        || decoder.total_bytes() > 64 * 1024 * 1024
    {
        return Err("画像が内蔵プレビューの画素数上限を超えています".into());
    }
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut decoded = image::DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    decoded.apply_orientation(orientation);
    Ok(decoded
        .thumbnail(edge.clamp(1, 1600), edge.clamp(1, 1600))
        .into_rgba8())
}

pub(super) fn text(reader: impl Read) -> io::Result<Option<(String, bool)>> {
    let mut bytes = Vec::new();
    reader
        .take((TEXT_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)?;
    let truncated = bytes.len() > TEXT_LIMIT;
    bytes.truncate(TEXT_LIMIT);
    if bytes.contains(&0) {
        return Ok(None);
    }
    let text = match std::str::from_utf8(&bytes) {
        Ok(text) => text,
        Err(error) if truncated && error.error_len().is_none() => {
            std::str::from_utf8(&bytes[..error.valid_up_to()]).expect("valid UTF-8 prefix")
        }
        Err(_) => return Ok(None),
    };
    Ok(Some((
        text.strip_prefix('\u{feff}').unwrap_or(text).to_owned(),
        truncated,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_is_bounded_and_binary_is_not_decoded_as_text() {
        let (text, truncated) = text(&vec![b'a'; TEXT_LIMIT + 32][..]).unwrap().unwrap();
        assert_eq!(text.len(), TEXT_LIMIT);
        assert!(truncated);
        assert!(super::text(&b"a\0b"[..]).unwrap().is_none());
        assert!(super::text(&[0xff, 0xfe][..]).unwrap().is_none());
        assert_eq!(
            super::text("日本語".as_bytes()).unwrap().unwrap(),
            ("日本語".into(), false)
        );
    }
    #[test]
    fn truncated_text_keeps_valid_utf8_at_boundary() {
        let mut bytes = vec![b'a'; TEXT_LIMIT - 1];
        bytes.extend_from_slice("日本語".as_bytes());
        let (text, truncated) = text(&bytes[..]).unwrap().unwrap();
        assert_eq!(text.len(), TEXT_LIMIT - 1);
        assert!(truncated);
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba([255, 0, 0, 255]));
        let mut bytes = io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    #[test]
    fn image_thumbnail_preserves_aspect_ratio_and_color() {
        let image = decode_bytes(png(64, 32), 16).unwrap();
        assert_eq!(image.dimensions(), (16, 8));
        assert_eq!(image.get_pixel(0, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn image_dimension_budget_rejects_before_full_decode() {
        assert!(decode_bytes(png(8193, 1), 16).is_err());
        assert!(decode_bytes(b"not an image".to_vec(), 16).is_err());
    }
}
