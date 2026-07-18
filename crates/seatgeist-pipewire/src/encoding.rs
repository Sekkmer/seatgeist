use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba, imageops::FilterType};
use sha2::{Digest, Sha256};

use crate::{
    EncodedFrame, MAX_SOURCE_BYTES, MAX_SOURCE_EDGE, PipeWireCaptureError, RawPixelFormat,
    RawVideoFrame, Result,
};

pub fn encode_bounded_png(frame: &RawVideoFrame, max_edge: u32) -> Result<EncodedFrame> {
    if max_edge == 0 {
        return Err(PipeWireCaptureError::InvalidFrame(
            "max_edge must be greater than zero".to_string(),
        ));
    }
    validate_frame(frame)?;
    let rgba = convert_to_rgba(frame)?;
    let revision = frame_revision(frame, &rgba);
    let source = ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(frame.width, frame.height, rgba)
        .ok_or_else(|| {
            PipeWireCaptureError::InvalidFrame(
                "converted RGBA length does not match dimensions".to_string(),
            )
        })?;
    let image = DynamicImage::ImageRgba8(source);
    let bounded = if frame.width > max_edge || frame.height > max_edge {
        image.resize(max_edge, max_edge, FilterType::Triangle)
    } else {
        image
    };
    let output_width = bounded.width();
    let output_height = bounded.height();
    let mut png = std::io::Cursor::new(Vec::new());
    bounded
        .write_to(&mut png, ImageFormat::Png)
        .map_err(|err| PipeWireCaptureError::Png(err.to_string()))?;
    Ok(EncodedFrame {
        png: png.into_inner(),
        source_width: frame.width,
        source_height: frame.height,
        output_width,
        output_height,
        revision,
        sequence: frame.sequence,
        damage_present: frame.damage_present,
    })
}

fn validate_frame(frame: &RawVideoFrame) -> Result<()> {
    if frame.width == 0 || frame.height == 0 {
        return Err(PipeWireCaptureError::InvalidFrame(
            "width and height must be greater than zero".to_string(),
        ));
    }
    if frame.width > MAX_SOURCE_EDGE || frame.height > MAX_SOURCE_EDGE {
        return Err(PipeWireCaptureError::InvalidFrame(format!(
            "source dimensions exceed {MAX_SOURCE_EDGE} pixels per edge"
        )));
    }
    if frame.data.len() > MAX_SOURCE_BYTES {
        return Err(PipeWireCaptureError::InvalidFrame(format!(
            "source buffer exceeds {MAX_SOURCE_BYTES} bytes"
        )));
    }
    let row_bytes = usize::try_from(frame.width)
        .ok()
        .and_then(|width| width.checked_mul(frame.format.bytes_per_pixel()))
        .ok_or_else(|| PipeWireCaptureError::InvalidFrame("row byte size overflow".to_string()))?;
    let stride = usize::try_from(frame.stride.unsigned_abs())
        .map_err(|_| PipeWireCaptureError::InvalidFrame("stride exceeds usize".to_string()))?;
    if stride < row_bytes {
        return Err(PipeWireCaptureError::InvalidFrame(format!(
            "stride {stride} is smaller than row size {row_bytes}"
        )));
    }
    let required = stride
        .checked_mul(usize::try_from(frame.height).unwrap_or(usize::MAX))
        .ok_or_else(|| PipeWireCaptureError::InvalidFrame("buffer size overflow".to_string()))?;
    if frame.data.len() < required {
        return Err(PipeWireCaptureError::InvalidFrame(format!(
            "buffer has {} bytes but {required} are required",
            frame.data.len()
        )));
    }
    Ok(())
}

fn convert_to_rgba(frame: &RawVideoFrame) -> Result<Vec<u8>> {
    let width = usize::try_from(frame.width)
        .map_err(|_| PipeWireCaptureError::InvalidFrame("width exceeds usize".to_string()))?;
    let height = usize::try_from(frame.height)
        .map_err(|_| PipeWireCaptureError::InvalidFrame("height exceeds usize".to_string()))?;
    let stride = usize::try_from(frame.stride.unsigned_abs())
        .map_err(|_| PipeWireCaptureError::InvalidFrame("stride exceeds usize".to_string()))?;
    let bpp = frame.format.bytes_per_pixel();
    let mut rgba = Vec::with_capacity(
        width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| PipeWireCaptureError::InvalidFrame("RGBA size overflow".to_string()))?,
    );
    for output_y in 0..height {
        let source_y = if frame.stride < 0 {
            height - output_y - 1
        } else {
            output_y
        };
        let row_start = source_y * stride;
        let row = &frame.data[row_start..row_start + width * bpp];
        for pixel in row.chunks_exact(bpp) {
            let (red, green, blue, alpha) = match frame.format {
                RawPixelFormat::Bgrx => (pixel[2], pixel[1], pixel[0], 255),
                RawPixelFormat::Bgra => (pixel[2], pixel[1], pixel[0], pixel[3]),
                RawPixelFormat::Rgbx => (pixel[0], pixel[1], pixel[2], 255),
                RawPixelFormat::Rgba => (pixel[0], pixel[1], pixel[2], pixel[3]),
                RawPixelFormat::Bgr => (pixel[2], pixel[1], pixel[0], 255),
                RawPixelFormat::Rgb => (pixel[0], pixel[1], pixel[2], 255),
            };
            rgba.extend_from_slice(&[red, green, blue, alpha]);
        }
    }
    Ok(rgba)
}

fn frame_revision(frame: &RawVideoFrame, bounded_rgba: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(frame.width.to_le_bytes());
    hasher.update(frame.height.to_le_bytes());
    hasher.update(bounded_rgba);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_bgrx_with_padding_to_bounded_png() -> Result<()> {
        let frame = RawVideoFrame {
            width: 2,
            height: 1,
            stride: 12,
            format: RawPixelFormat::Bgrx,
            sequence: 7,
            damage_present: true,
            data: vec![0, 0, 255, 0, 0, 255, 0, 0, 9, 9, 9, 9],
        };
        let encoded = encode_bounded_png(&frame, 1)?;
        assert_eq!((encoded.source_width, encoded.source_height), (2, 1));
        assert_eq!((encoded.output_width, encoded.output_height), (1, 1));
        assert_eq!(encoded.sequence, 7);
        assert!(encoded.damage_present);
        let decoded = image::load_from_memory(&encoded.png)
            .map_err(|err| PipeWireCaptureError::Png(err.to_string()))?;
        assert_eq!((decoded.width(), decoded.height()), (1, 1));
        Ok(())
    }

    #[test]
    fn negative_stride_flips_bottom_up_frames() -> Result<()> {
        let frame = RawVideoFrame {
            width: 1,
            height: 2,
            stride: -4,
            format: RawPixelFormat::Bgra,
            sequence: 1,
            damage_present: false,
            data: vec![255, 0, 0, 255, 0, 0, 255, 255],
        };
        let rgba = convert_to_rgba(&frame)?;
        assert_eq!(&rgba[..4], &[255, 0, 0, 255]);
        assert_eq!(&rgba[4..], &[0, 0, 255, 255]);
        Ok(())
    }

    #[test]
    fn rejects_truncated_or_impossible_frames() {
        let mut frame = RawVideoFrame {
            width: 2,
            height: 2,
            stride: 8,
            format: RawPixelFormat::Rgba,
            sequence: 1,
            damage_present: false,
            data: vec![0; 8],
        };
        assert!(encode_bounded_png(&frame, 800).is_err());
        frame.data.resize(16, 0);
        frame.stride = 4;
        assert!(encode_bounded_png(&frame, 800).is_err());
        assert!(encode_bounded_png(&frame, 0).is_err());
    }

    #[test]
    fn revisions_ignore_sequence_but_change_with_pixels() -> Result<()> {
        let mut frame = RawVideoFrame {
            width: 1,
            height: 1,
            stride: 4,
            format: RawPixelFormat::Rgba,
            sequence: 1,
            damage_present: false,
            data: vec![1, 2, 3, 255],
        };
        let first = encode_bounded_png(&frame, 800)?;
        frame.sequence = 2;
        let same = encode_bounded_png(&frame, 800)?;
        assert_eq!(first.revision, same.revision);
        frame.data[0] = 9;
        let changed = encode_bounded_png(&frame, 800)?;
        assert_ne!(first.revision, changed.revision);
        Ok(())
    }
}
