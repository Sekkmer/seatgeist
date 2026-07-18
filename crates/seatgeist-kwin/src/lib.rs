use std::{collections::HashSet, process::Command};

use libseatgeist::{CoordinateSpace, MonitorInfo, SeatgeistError, WindowGeometry, WindowInfo};

pub const BACKEND_NAME: &str = "kwin";

pub type Result<T> = std::result::Result<T, SeatgeistError>;

pub fn list_monitors() -> Result<Vec<MonitorInfo>> {
    let output = Command::new("qdbus6")
        .args(["org.kde.KWin", "/KWin", "org.kde.KWin.supportInformation"])
        .output()
        .map_err(|err| SeatgeistError::BackendUnavailable(format!("run qdbus6: {err}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SeatgeistError::BackendUnavailable(format!(
            "qdbus6 KWin supportInformation exited with status {}: {stderr}",
            output.status
        )));
    }

    let support_info = String::from_utf8(output.stdout).map_err(|err| {
        SeatgeistError::BackendUnavailable(format!("KWin supportInformation was not UTF-8: {err}"))
    })?;
    parse_support_info_monitors(&support_info)
}

pub fn list_windows() -> Result<Vec<WindowInfo>> {
    let output = Command::new("qdbus6")
        .args([
            "--literal",
            "org.kde.KWin",
            "/WindowsRunner",
            "org.kde.krunner1.Match",
            "",
        ])
        .output()
        .map_err(|err| SeatgeistError::BackendUnavailable(format!("run qdbus6: {err}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SeatgeistError::BackendUnavailable(format!(
            "qdbus6 KWin WindowsRunner Match exited with status {}: {stderr}",
            output.status
        )));
    }

    let literal = String::from_utf8(output.stdout).map_err(|err| {
        SeatgeistError::BackendUnavailable(format!("WindowsRunner output was not UTF-8: {err}"))
    })?;
    let mut windows = Vec::new();
    for match_entry in parse_windows_runner_matches(&literal) {
        windows.push(enrich_window(match_entry));
    }
    Ok(windows)
}

pub fn active_window() -> Result<Option<WindowInfo>> {
    Err(SeatgeistError::BackendUnavailable(
        "active window requires the Seatgeist KWin script bridge".to_string(),
    ))
}

pub fn focus_window(window_id: &str) -> Result<()> {
    let match_id = runner_match_id(window_id)?;
    let output = Command::new("qdbus6")
        .args([
            "org.kde.KWin",
            "/WindowsRunner",
            "org.kde.krunner1.Run",
            &match_id,
            "",
        ])
        .output()
        .map_err(|err| SeatgeistError::BackendUnavailable(format!("run qdbus6: {err}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SeatgeistError::BackendUnavailable(format!(
            "qdbus6 KWin WindowsRunner Run exited with status {}: {stderr}",
            output.status
        )));
    }

    Ok(())
}

fn enrich_window(match_entry: RunnerWindowMatch) -> WindowInfo {
    let info = get_window_info(&match_entry.kwin_uuid).unwrap_or_default();
    WindowInfo {
        id: match_entry.kwin_uuid,
        app_id: info
            .desktop_file
            .or(info.resource_class)
            .or(Some(match_entry.icon_name)),
        title: info.caption.unwrap_or(match_entry.title),
        pid: info.pid,
        monitor_id: None,
        geometry: info.geometry,
    }
}

fn get_window_info(window_id: &str) -> Result<KwinWindowInfo> {
    let output = Command::new("qdbus6")
        .args([
            "--literal",
            "org.kde.KWin",
            "/KWin",
            "org.kde.KWin.getWindowInfo",
            window_id,
        ])
        .output()
        .map_err(|err| SeatgeistError::BackendUnavailable(format!("run qdbus6: {err}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SeatgeistError::BackendUnavailable(format!(
            "qdbus6 KWin getWindowInfo exited with status {}: {stderr}",
            output.status
        )));
    }

    let literal = String::from_utf8(output.stdout).map_err(|err| {
        SeatgeistError::BackendUnavailable(format!("getWindowInfo output was not UTF-8: {err}"))
    })?;
    Ok(parse_get_window_info(&literal))
}

pub fn parse_windows_runner_matches(literal: &str) -> Vec<RunnerWindowMatch> {
    let mut matches = Vec::new();
    let mut seen_window_ids = HashSet::new();
    let mut rest = literal;
    while let Some(index) = rest.find("(sssida{sv})") {
        rest = &rest[index + "(sssida{sv})".len()..];
        let (Some((runner_id, after_id)), Some((title, after_title)), Some((icon, after_icon))) = (
            parse_quoted(rest),
            parse_quoted_after_first(rest),
            parse_third_quoted(rest),
        ) else {
            break;
        };
        let _ = after_id;
        let _ = after_title;
        rest = after_icon;
        let kwin_uuid = normalize_runner_window_id(&runner_id);
        if !seen_window_ids.insert(kwin_uuid.clone()) {
            continue;
        }
        matches.push(RunnerWindowMatch {
            kwin_uuid,
            title,
            icon_name: icon,
        });
    }
    matches
}

fn parse_quoted_after_first(input: &str) -> Option<(String, &str)> {
    let (_, rest) = parse_quoted(input)?;
    parse_quoted(rest)
}

fn parse_third_quoted(input: &str) -> Option<(String, &str)> {
    let (_, rest) = parse_quoted(input)?;
    let (_, rest) = parse_quoted(rest)?;
    parse_quoted(rest)
}

fn parse_quoted(input: &str) -> Option<(String, &str)> {
    let start = input.find('"')?;
    let mut value = String::new();
    let mut escaped = false;
    for (offset, character) in input[start + 1..].char_indices() {
        if escaped {
            value.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if character == '"' {
            let rest = &input[start + 1 + offset + character.len_utf8()..];
            return Some((value, rest));
        }
        value.push(character);
    }
    None
}

fn normalize_runner_window_id(runner_id: &str) -> String {
    runner_id
        .strip_prefix("0_")
        .unwrap_or(runner_id)
        .to_string()
}

fn runner_match_id(window_id: &str) -> Result<String> {
    let trimmed = window_id.trim();
    if trimmed.is_empty() {
        return Err(SeatgeistError::InvalidRequest(
            "window id must not be empty".to_string(),
        ));
    }
    if trimmed.starts_with("0_") {
        return Ok(trimmed.to_string());
    }
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        return Ok(format!("0_{trimmed}"));
    }
    Ok(format!("0_{{{trimmed}}}"))
}

pub fn parse_get_window_info(literal: &str) -> KwinWindowInfo {
    let caption = parse_variant_string(literal, "caption");
    let desktop_file = parse_variant_string(literal, "desktopFile");
    let resource_class = parse_variant_string(literal, "resourceClass");
    let pid = parse_variant_u32(literal, "pid");
    let geometry = match (
        parse_variant_f64(literal, "x"),
        parse_variant_f64(literal, "y"),
        parse_variant_f64(literal, "width"),
        parse_variant_f64(literal, "height"),
    ) {
        (Some(x), Some(y), Some(width), Some(height)) => Some(WindowGeometry {
            x: x.round() as i32,
            y: y.round() as i32,
            width: width.round().max(1.0) as u32,
            height: height.round().max(1.0) as u32,
            space: CoordinateSpace::LogicalPixel,
        }),
        _ => None,
    };

    KwinWindowInfo {
        caption,
        desktop_file,
        resource_class,
        pid,
        geometry,
    }
}

fn parse_variant_string(literal: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\" = [Variant(QString): ");
    let start = literal.find(&needle)?;
    parse_quoted(&literal[start + needle.len()..]).map(|(value, _)| value)
}

fn parse_variant_f64(literal: &str, key: &str) -> Option<f64> {
    let needle = format!("\"{key}\" = [Variant(double): ");
    let start = literal.find(&needle)? + needle.len();
    let rest = &literal[start..];
    let end = rest.find(']')?;
    rest[..end].trim().parse().ok()
}

fn parse_variant_u32(literal: &str, key: &str) -> Option<u32> {
    ["int", "uint", "qlonglong", "qulonglong"]
        .into_iter()
        .find_map(|variant_type| {
            let needle = format!("\"{key}\" = [Variant({variant_type}): ");
            let start = literal.find(&needle)? + needle.len();
            let rest = &literal[start..];
            let end = rest.find(']')?;
            rest[..end].trim().parse().ok().filter(|value| *value > 0)
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerWindowMatch {
    pub kwin_uuid: String,
    pub title: String,
    pub icon_name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KwinWindowInfo {
    pub caption: Option<String>,
    pub desktop_file: Option<String>,
    pub resource_class: Option<String>,
    pub pid: Option<u32>,
    pub geometry: Option<WindowGeometry>,
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
                SeatgeistError::InvalidRequest(format!("parse KWin monitor scale '{scale}': {err}"))
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
        return Err(SeatgeistError::InvalidRequest(format!(
            "invalid KWin geometry '{geometry}'"
        )));
    };
    let Some((origin_y, size)) = size.split_once(',') else {
        return Err(SeatgeistError::InvalidRequest(format!(
            "invalid KWin geometry '{geometry}'"
        )));
    };
    let Some((width, height)) = size.split_once('x') else {
        return Err(SeatgeistError::InvalidRequest(format!(
            "invalid KWin geometry '{geometry}'"
        )));
    };

    let origin_x = origin.trim().parse::<i32>().map_err(|err| {
        SeatgeistError::InvalidRequest(format!("parse KWin geometry x '{origin}': {err}"))
    })?;
    let origin_y = origin_y.trim().parse::<i32>().map_err(|err| {
        SeatgeistError::InvalidRequest(format!("parse KWin geometry y '{origin_y}': {err}"))
    })?;
    let width = width.trim().parse::<u32>().map_err(|err| {
        SeatgeistError::InvalidRequest(format!("parse KWin geometry width '{width}': {err}"))
    })?;
    let height = height.trim().parse::<u32>().map_err(|err| {
        SeatgeistError::InvalidRequest(format!("parse KWin geometry height '{height}': {err}"))
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
            SeatgeistError::InvalidRequest(format!("KWin monitor {id} missing logical width"))
        })?;
        let logical_height = self.logical_height.ok_or_else(|| {
            SeatgeistError::InvalidRequest(format!("KWin monitor {id} missing logical height"))
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

    #[test]
    fn parses_windows_runner_matches() {
        let literal = r#"[Argument: a(sssida{sv}) {[Argument: (sssida{sv}) "0_{bfa1f612-bb46-49de-b54e-8b89cd7e86b5}", "oxidentp : MainThread — Konsole", "utilities-terminal", 30, 0.7, [Argument: a{sv} {"subtext" = [Variant(QString): "Activate running window on Desktop 1"]}]], [Argument: (sssida{sv}) "0_{88ee4ade-8664-447b-8fe8-ca8a6e86e259}", "outfit.txt — Kate", "kate", 30, 0.5, [Argument: a{sv} {"subtext" = [Variant(QString): "Activate running window on Desktop 1"]}]]}]"#;

        let windows = parse_windows_runner_matches(literal);
        assert_eq!(windows.len(), 2);
        assert_eq!(
            windows[0].kwin_uuid,
            "{bfa1f612-bb46-49de-b54e-8b89cd7e86b5}"
        );
        assert_eq!(windows[0].title, "oxidentp : MainThread — Konsole");
        assert_eq!(windows[0].icon_name, "utilities-terminal");
        assert_eq!(windows[1].title, "outfit.txt — Kate");
    }

    #[test]
    fn deduplicates_windows_runner_matches_by_stable_id() {
        let literal = r#"[Argument: a(sssida{sv}) {[Argument: (sssida{sv}) "0_{same}", "Best title", "firefox", 100, 0.8, [Argument: a{sv} {}]], [Argument: (sssida{sv}) "0_{same}", "Lower score title", "firefox", 30, 0.5, [Argument: a{sv} {}]]}]"#;

        let windows = parse_windows_runner_matches(literal);
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].kwin_uuid, "{same}");
        assert_eq!(windows[0].title, "Best title");
    }

    #[test]
    fn parses_get_window_info_geometry() {
        let literal = r#"[Argument: a{sv} {"caption" = [Variant(QString): "oxidentp : MainThread — Konsole"], "desktopFile" = [Variant(QString): "org.kde.konsole"], "pid" = [Variant(int): 4242], "resourceClass" = [Variant(QString): "org.kde.konsole"], "height" = [Variant(double): 2173.33], "width" = [Variant(double): 2087.33], "x" = [Variant(double): 1987.8], "y" = [Variant(double): 472.226]}]"#;

        let info = parse_get_window_info(literal);
        assert_eq!(
            info.caption.as_deref(),
            Some("oxidentp : MainThread — Konsole")
        );
        assert_eq!(info.desktop_file.as_deref(), Some("org.kde.konsole"));
        assert_eq!(info.pid, Some(4242));
        let geometry = info.geometry.expect("geometry parsed");
        assert_eq!(geometry.x, 1988);
        assert_eq!(geometry.y, 472);
        assert_eq!(geometry.width, 2087);
        assert_eq!(geometry.height, 2173);
        assert_eq!(geometry.space, CoordinateSpace::LogicalPixel);
    }

    #[test]
    fn formats_runner_match_id_for_focus() {
        assert_eq!(
            runner_match_id("{96d3c5da-75ec-4a2a-b75f-05c4c077153b}").expect("braced id formats"),
            "0_{96d3c5da-75ec-4a2a-b75f-05c4c077153b}"
        );
        assert_eq!(
            runner_match_id("96d3c5da-75ec-4a2a-b75f-05c4c077153b").expect("bare id formats"),
            "0_{96d3c5da-75ec-4a2a-b75f-05c4c077153b}"
        );
        assert_eq!(
            runner_match_id("0_{96d3c5da-75ec-4a2a-b75f-05c4c077153b}")
                .expect("runner id stays stable"),
            "0_{96d3c5da-75ec-4a2a-b75f-05c4c077153b}"
        );
    }
}
