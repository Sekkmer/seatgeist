use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use image::{GenericImageView, Rgba, imageops::FilterType};
use libseatgeist::{ScreenshotInfo, ScreenshotTileRequest, ScreenshotTransform};

use crate::config::RedactRegion;

pub(crate) fn temporary_capture_path(output: &Path) -> PathBuf {
    let file_name = output
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("screenshot.png");
    let temp_name = format!(".seatgeist-full-{}-{file_name}", std::process::id());
    output.with_file_name(temp_name)
}

pub(crate) fn write_preview_or_copy(
    source: &Path,
    output: &Path,
    source_width: u32,
    source_height: u32,
    max_edge: u32,
) -> Result<(u32, u32)> {
    if max_edge == 0 {
        bail!("max_edge must be greater than zero");
    }

    let largest_edge = source_width.max(source_height);
    if largest_edge <= max_edge {
        fs::copy(source, output)
            .with_context(|| format!("copy screenshot preview to {}", output.display()))?;
        return Ok((source_width, source_height));
    }

    let scale = f64::from(max_edge) / f64::from(largest_edge);
    let output_width = scaled_dimension(source_width, scale);
    let output_height = scaled_dimension(source_height, scale);
    let image =
        image::open(source).with_context(|| format!("open screenshot {}", source.display()))?;
    let resized = image.resize(output_width, output_height, FilterType::Lanczos3);
    resized
        .save(output)
        .with_context(|| format!("write screenshot preview {}", output.display()))?;
    Ok((output_width, output_height))
}

pub(crate) fn write_tile_preview(
    source: &Path,
    request: &ScreenshotTileRequest,
    max_edge: u32,
) -> Result<(u32, u32)> {
    if max_edge == 0 {
        bail!("max_edge must be greater than zero");
    }

    let image =
        image::open(source).with_context(|| format!("open screenshot {}", source.display()))?;
    let cropped = image.crop_imm(request.x, request.y, request.width, request.height);
    let largest_edge = request.width.max(request.height);
    let output_image = if largest_edge > max_edge {
        let scale = f64::from(max_edge) / f64::from(largest_edge);
        let output_width = scaled_dimension(request.width, scale);
        let output_height = scaled_dimension(request.height, scale);
        cropped.resize(output_width, output_height, FilterType::Lanczos3)
    } else {
        cropped
    };

    let (output_width, output_height) = output_image.dimensions();
    output_image
        .save(&request.output)
        .with_context(|| format!("write screenshot tile {}", request.output.display()))?;
    Ok((output_width, output_height))
}

pub(crate) fn apply_screenshot_redactions(
    info: &ScreenshotInfo,
    redactions: &[RedactRegion],
) -> Result<()> {
    if redactions.is_empty() {
        return Ok(());
    }

    let mut image = image::open(&info.path)
        .with_context(|| format!("open screenshot for redaction {}", info.path.display()))?
        .to_rgba8();
    let (width, height) = image.dimensions();
    let mut changed = false;
    for redaction in redactions {
        let Some(rect) = output_redaction_rect(redaction, &info.transform, width, height) else {
            continue;
        };
        changed = true;
        for y in rect.y..rect.y + rect.height {
            for x in rect.x..rect.x + rect.width {
                image.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
    }

    if changed {
        image
            .save(&info.path)
            .with_context(|| format!("write redacted screenshot {}", info.path.display()))?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OutputRedactionRect {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

pub(crate) fn output_redaction_rect(
    redaction: &RedactRegion,
    transform: &ScreenshotTransform,
    output_width: u32,
    output_height: u32,
) -> Option<OutputRedactionRect> {
    if transform.scale_x <= 0.0 || transform.scale_y <= 0.0 {
        return None;
    }

    let source_left = f64::from(transform.source_origin_x);
    let source_top = f64::from(transform.source_origin_y);
    let source_right = source_left + f64::from(output_width) / transform.scale_x;
    let source_bottom = source_top + f64::from(output_height) / transform.scale_y;

    let redact_left = f64::from(redaction.x);
    let redact_top = f64::from(redaction.y);
    let redact_right = redact_left + f64::from(redaction.width);
    let redact_bottom = redact_top + f64::from(redaction.height);

    let left = redact_left.max(source_left);
    let top = redact_top.max(source_top);
    let right = redact_right.min(source_right);
    let bottom = redact_bottom.min(source_bottom);
    if right <= left || bottom <= top {
        return None;
    }

    let output_left = ((left - source_left) * transform.scale_x)
        .floor()
        .clamp(0.0, f64::from(output_width)) as u32;
    let output_top = ((top - source_top) * transform.scale_y)
        .floor()
        .clamp(0.0, f64::from(output_height)) as u32;
    let output_right = ((right - source_left) * transform.scale_x)
        .ceil()
        .clamp(0.0, f64::from(output_width)) as u32;
    let output_bottom = ((bottom - source_top) * transform.scale_y)
        .ceil()
        .clamp(0.0, f64::from(output_height)) as u32;
    if output_right <= output_left || output_bottom <= output_top {
        return None;
    }

    Some(OutputRedactionRect {
        x: output_left,
        y: output_top,
        width: output_right - output_left,
        height: output_bottom - output_top,
    })
}

pub(crate) fn scaled_dimension(value: u32, scale: f64) -> u32 {
    (f64::from(value) * scale).round().max(1.0) as u32
}

pub(crate) fn validate_tile_request(request: &ScreenshotTileRequest) -> Result<()> {
    if request.width == 0 || request.height == 0 {
        bail!("tile width and height must be greater than zero");
    }
    if request.max_edge == Some(0) {
        bail!("max_edge must be greater than zero");
    }
    Ok(())
}

pub(crate) fn validate_tile_bounds(
    request: &ScreenshotTileRequest,
    source_width: u32,
    source_height: u32,
) -> Result<()> {
    let Some(end_x) = request.x.checked_add(request.width) else {
        bail!("tile x + width overflows u32");
    };
    let Some(end_y) = request.y.checked_add(request.height) else {
        bail!("tile y + height overflows u32");
    };

    if end_x > source_width || end_y > source_height {
        bail!(
            "tile {}x{} at {},{} is outside source screenshot {}x{}",
            request.width,
            request.height,
            request.x,
            request.y,
            source_width,
            source_height
        );
    }

    Ok(())
}

pub(crate) fn prepare_screenshot_output(output: &Path) -> Result<()> {
    if output.extension().and_then(|ext| ext.to_str()) != Some("png") {
        bail!(
            "screenshot output must be a .png path: {}",
            output.display()
        );
    }

    if let Ok(metadata) = fs::symlink_metadata(output) {
        if metadata.file_type().is_symlink() {
            bail!(
                "refusing to write screenshot through symlink {}",
                output.display()
            );
        }
        if metadata.is_dir() {
            bail!("screenshot output is a directory: {}", output.display());
        }
    }

    let parent = output
        .parent()
        .ok_or_else(|| anyhow::anyhow!("screenshot output has no parent: {}", output.display()))?;
    if !parent.as_os_str().is_empty() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create screenshot output dir {}", parent.display()))?;
    }
    Ok(())
}

pub(crate) fn read_png_dimensions_with_retry(path: &Path) -> Result<(u32, u32)> {
    let mut last_error = None;
    for _ in 0..10 {
        match read_png_dimensions(path) {
            Ok(dimensions) => return Ok(dimensions),
            Err(err) => {
                last_error = Some(err);
                thread::sleep(Duration::from_millis(50));
            }
        }
    }

    match last_error {
        Some(err) => Err(err),
        None => bail!("could not read screenshot dimensions"),
    }
}

fn read_png_dimensions(path: &Path) -> Result<(u32, u32)> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    if bytes.len() < 24 || &bytes[0..8] != b"\x89PNG\r\n\x1a\n" {
        bail!("screenshot is not a valid PNG: {}", path.display());
    }

    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    Ok((width, height))
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::symlink, path::PathBuf};

    use image::{Rgba, RgbaImage};
    use libseatgeist::{CoordinateSpace, ScreenshotInfo, ScreenshotTransform};
    use uuid::Uuid;

    use super::*;

    fn temp_png(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "seatgeist-screenshot-image-{label}-{}.png",
            Uuid::new_v4()
        ))
    }

    fn physical_transform(origin_x: u32, origin_y: u32, scale: f64) -> ScreenshotTransform {
        ScreenshotTransform {
            source_coordinate_space: CoordinateSpace::PhysicalPixel,
            output_coordinate_space: CoordinateSpace::PhysicalPixel,
            source_origin_x: origin_x,
            source_origin_y: origin_y,
            scale_x: scale,
            scale_y: scale,
        }
    }

    #[test]
    fn redaction_rect_maps_source_region_to_preview_output() {
        let rect = output_redaction_rect(
            &RedactRegion {
                x: 400,
                y: 200,
                width: 200,
                height: 100,
            },
            &physical_transform(0, 0, 0.5),
            800,
            450,
        )
        .expect("redaction overlaps preview");

        assert_eq!(
            rect,
            OutputRedactionRect {
                x: 200,
                y: 100,
                width: 100,
                height: 50,
            }
        );
    }

    #[test]
    fn redaction_rect_maps_source_region_to_tile_output() {
        let rect = output_redaction_rect(
            &RedactRegion {
                x: 150,
                y: 250,
                width: 100,
                height: 100,
            },
            &physical_transform(100, 200, 0.5),
            200,
            100,
        )
        .expect("redaction overlaps tile");

        assert_eq!(
            rect,
            OutputRedactionRect {
                x: 25,
                y: 25,
                width: 50,
                height: 50,
            }
        );
    }

    #[test]
    fn redaction_rect_ignores_non_overlapping_region() {
        let rect = output_redaction_rect(
            &RedactRegion {
                x: 1000,
                y: 1000,
                width: 100,
                height: 100,
            },
            &physical_transform(0, 0, 1.0),
            100,
            100,
        );

        assert_eq!(rect, None);
    }

    #[test]
    fn screenshot_redaction_blacks_output_pixels() {
        let path = temp_png("redacted");
        RgbaImage::from_pixel(4, 4, Rgba([255, 255, 255, 255]))
            .save(&path)
            .expect("fixture image is written");
        let info = ScreenshotInfo {
            path: path.clone(),
            backend: "test".to_string(),
            occlusion_possible: false,
            source_width: 4,
            source_height: 4,
            output_width: 4,
            output_height: 4,
            transform: physical_transform(0, 0, 1.0),
            coordinate_space: CoordinateSpace::PhysicalPixel,
            monitors: Vec::new(),
        };

        apply_screenshot_redactions(
            &info,
            &[RedactRegion {
                x: 1,
                y: 1,
                width: 2,
                height: 2,
            }],
        )
        .expect("redaction succeeds");

        let redacted = image::open(&path).expect("redacted image opens").to_rgba8();
        assert_eq!(*redacted.get_pixel(0, 0), Rgba([255, 255, 255, 255]));
        assert_eq!(*redacted.get_pixel(1, 1), Rgba([0, 0, 0, 255]));
        assert_eq!(*redacted.get_pixel(2, 2), Rgba([0, 0, 0, 255]));
        fs::remove_file(path).ok();
    }

    #[test]
    fn preview_and_tile_outputs_are_bounded() {
        let source = temp_png("source");
        let preview = temp_png("preview");
        let tile = temp_png("tile");
        RgbaImage::from_pixel(8, 4, Rgba([12, 34, 56, 255]))
            .save(&source)
            .expect("source fixture is written");

        assert_eq!(
            write_preview_or_copy(&source, &preview, 8, 4, 4).expect("preview is written"),
            (4, 2)
        );
        assert_eq!(
            read_png_dimensions(&preview).expect("preview dimensions are readable"),
            (4, 2)
        );

        let request = ScreenshotTileRequest {
            output: tile.clone(),
            x: 2,
            y: 0,
            width: 4,
            height: 4,
            max_edge: Some(2),
            portal_interactive: false,
        };
        validate_tile_request(&request).expect("tile request is valid");
        validate_tile_bounds(&request, 8, 4).expect("tile is in bounds");
        assert_eq!(
            write_tile_preview(&source, &request, 2).expect("tile is written"),
            (2, 2)
        );

        for path in [source, preview, tile] {
            fs::remove_file(path).ok();
        }
    }

    #[test]
    fn output_preparation_rejects_non_png_and_symlink() {
        let non_png = temp_png("wrong-extension").with_extension("jpg");
        let error = prepare_screenshot_output(&non_png).expect_err("non-PNG path is rejected");
        assert!(error.to_string().contains("must be a .png path"));

        let target = temp_png("target");
        let link = temp_png("link");
        fs::write(&target, b"not used").expect("target fixture is written");
        symlink(&target, &link).expect("symlink fixture is created");
        let error = prepare_screenshot_output(&link).expect_err("symlink output is rejected");
        assert!(
            error
                .to_string()
                .contains("refusing to write screenshot through symlink")
        );

        fs::remove_file(link).ok();
        fs::remove_file(target).ok();
    }

    #[test]
    fn tile_bounds_reject_outside_and_overflowing_requests() {
        let mut request = ScreenshotTileRequest {
            output: temp_png("bounds"),
            x: 7,
            y: 0,
            width: 2,
            height: 1,
            max_edge: None,
            portal_interactive: false,
        };
        assert!(validate_tile_bounds(&request, 8, 4).is_err());

        request.x = u32::MAX;
        assert!(
            validate_tile_bounds(&request, u32::MAX, 4)
                .expect_err("overflowing tile is rejected")
                .to_string()
                .contains("overflows")
        );
    }
}
