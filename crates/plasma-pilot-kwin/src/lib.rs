use std::process::Command;

use libplasma_pilot::{MonitorInfo, PilotError};

pub const BACKEND_NAME: &str = "kwin";

pub type Result<T> = std::result::Result<T, PilotError>;

pub fn list_monitors() -> Result<Vec<MonitorInfo>> {
    let output = Command::new("qdbus6")
        .args(["org.kde.KWin", "/KWin", "org.kde.KWin.supportInformation"])
        .output()
        .map_err(|err| PilotError::BackendUnavailable(format!("run qdbus6: {err}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(PilotError::BackendUnavailable(format!(
            "qdbus6 KWin supportInformation exited with status {}: {stderr}",
            output.status
        )));
    }

    let support_info = String::from_utf8(output.stdout).map_err(|err| {
        PilotError::BackendUnavailable(format!("KWin supportInformation was not UTF-8: {err}"))
    })?;
    parse_support_info_monitors(&support_info)
}

pub fn parse_support_info_monitors(support_info: &str) -> Result<Vec<MonitorInfo>> {
    let mut monitors = Vec::new();
    let mut current = MonitorBuilder::default();
    let mut in_screen = false;

    for raw_line in support_info.lines() {
        let line = raw_line.trim();
        if line.starts_with("Screen ") && line.ends_with(':') {
            if in_screen {
                monitors.push(current.build()?);
                current = MonitorBuilder::default();
            }
            current.id = Some(
                line.trim_end_matches(':')
                    .trim_start_matches("Screen ")
                    .to_string(),
            );
            in_screen = true;
            continue;
        }

        if !in_screen {
            continue;
        }

        if line.is_empty() {
            continue;
        }

        if line.ends_with("======") {
            monitors.push(current.build()?);
            current = MonitorBuilder::default();
            in_screen = false;
            continue;
        }

        if let Some(name) = line.strip_prefix("Name:") {
            let name = name.trim().to_string();
            current.name = Some(name.clone());
            current.id = Some(name);
        } else if let Some(geometry) = line.strip_prefix("Geometry:") {
            let geometry = parse_geometry(geometry.trim())?;
            current.logical_origin_x = Some(geometry.0);
            current.logical_origin_y = Some(geometry.1);
            current.logical_width = Some(geometry.2);
            current.logical_height = Some(geometry.3);
        } else if let Some(scale) = line.strip_prefix("Scale:") {
            let scale = scale.trim().parse::<f64>().map_err(|err| {
                PilotError::InvalidRequest(format!("parse KWin monitor scale '{scale}': {err}"))
            })?;
            current.scale_factor = Some(scale);
        }
    }

    if in_screen {
        monitors.push(current.build()?);
    }

    Ok(monitors)
}

fn parse_geometry(geometry: &str) -> Result<(i32, i32, u32, u32)> {
    let Some((origin, size)) = geometry.split_once(',') else {
        return Err(PilotError::InvalidRequest(format!(
            "invalid KWin geometry '{geometry}'"
        )));
    };
    let Some((origin_y, size)) = size.split_once(',') else {
        return Err(PilotError::InvalidRequest(format!(
            "invalid KWin geometry '{geometry}'"
        )));
    };
    let Some((width, height)) = size.split_once('x') else {
        return Err(PilotError::InvalidRequest(format!(
            "invalid KWin geometry '{geometry}'"
        )));
    };

    let origin_x = origin.trim().parse::<i32>().map_err(|err| {
        PilotError::InvalidRequest(format!("parse KWin geometry x '{origin}': {err}"))
    })?;
    let origin_y = origin_y.trim().parse::<i32>().map_err(|err| {
        PilotError::InvalidRequest(format!("parse KWin geometry y '{origin_y}': {err}"))
    })?;
    let width = width.trim().parse::<u32>().map_err(|err| {
        PilotError::InvalidRequest(format!("parse KWin geometry width '{width}': {err}"))
    })?;
    let height = height.trim().parse::<u32>().map_err(|err| {
        PilotError::InvalidRequest(format!("parse KWin geometry height '{height}': {err}"))
    })?;

    Ok((origin_x, origin_y, width, height))
}

#[derive(Debug, Default)]
struct MonitorBuilder {
    id: Option<String>,
    name: Option<String>,
    logical_origin_x: Option<i32>,
    logical_origin_y: Option<i32>,
    logical_width: Option<u32>,
    logical_height: Option<u32>,
    scale_factor: Option<f64>,
}

impl MonitorBuilder {
    fn build(self) -> Result<MonitorInfo> {
        let id = self.id.unwrap_or_else(|| "unknown".to_string());
        let scale_factor = self.scale_factor.unwrap_or(1.0);
        let logical_width = self.logical_width.ok_or_else(|| {
            PilotError::InvalidRequest(format!("KWin monitor {id} missing logical width"))
        })?;
        let logical_height = self.logical_height.ok_or_else(|| {
            PilotError::InvalidRequest(format!("KWin monitor {id} missing logical height"))
        })?;

        Ok(MonitorInfo {
            id,
            name: self.name,
            physical_width: scaled_dimension(logical_width, scale_factor),
            physical_height: scaled_dimension(logical_height, scale_factor),
            logical_width,
            logical_height,
            scale_factor,
            logical_origin_x: self.logical_origin_x.unwrap_or(0),
            logical_origin_y: self.logical_origin_y.unwrap_or(0),
            transform: None,
        })
    }
}

fn scaled_dimension(value: u32, scale: f64) -> u32 {
    (f64::from(value) * scale).round().max(1.0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kwin_support_info_screen() {
        let support_info = r#"
Screens
 =======
Number of Screens: 1

Screen 0:
---------
Name: HDMI-A-2
Enabled: 1
Geometry: 0,0,5120x2880
Physical size: 1872x1053mm
Scale: 1.5
Refresh Rate: 59940
Adaptive Sync: incapable

Compositing
 ===========
"#;

        let monitors = parse_support_info_monitors(support_info).expect("monitors parse");
        assert_eq!(monitors.len(), 1);
        assert_eq!(monitors[0].id, "HDMI-A-2");
        assert_eq!(monitors[0].logical_width, 5120);
        assert_eq!(monitors[0].logical_height, 2880);
        assert_eq!(monitors[0].physical_width, 7680);
        assert_eq!(monitors[0].physical_height, 4320);
        assert_eq!(monitors[0].scale_factor, 1.5);
    }
}
