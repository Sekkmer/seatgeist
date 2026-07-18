use libseatgeist::{
    CaptureBackendStatus, KwinMetadataStatus, ScreenshotPortalStatus, SpectacleStatus,
};

use crate::commands::{
    exists as command_exists, stdout as command_stdout, succeeds as command_success,
};

pub(crate) fn status() -> CaptureBackendStatus {
    let screenshot_portal = screenshot_portal_status();
    let kwin_metadata = kwin_metadata_status();
    let spectacle = spectacle_status();
    let preferred_available_backend =
        preferred_capture_backend(&screenshot_portal, spectacle.command_available);
    let implemented_available_backend =
        implemented_capture_backend(&screenshot_portal, spectacle.command_available);
    let setup_hint = capture_backend_setup_hint(
        preferred_available_backend.as_deref(),
        implemented_available_backend.as_deref(),
        &screenshot_portal,
        &kwin_metadata,
        &spectacle,
    );

    CaptureBackendStatus {
        screenshot_portal,
        kwin_metadata,
        spectacle,
        preferred_available_backend,
        implemented_available_backend,
        setup_hint,
    }
}

pub(crate) fn screenshot_portal_status() -> ScreenshotPortalStatus {
    let busctl_available = command_exists("busctl");
    if !busctl_available {
        return ScreenshotPortalStatus {
            busctl_available,
            portal_service_available: false,
            screenshot_interface_available: false,
            screenshot_interface_version: None,
            screenshot_available_targets_mask: None,
            screenshot_available_targets: Vec::new(),
            screenshot_target_option_supported: false,
            screencast_interface_available: false,
            kde_portal_service_available: false,
            setup_hint: screenshot_portal_setup_hint(false, false, false, None, None, false, false),
        };
    }

    let service_list =
        command_stdout("busctl", &["--user", "--no-pager", "--list"]).unwrap_or_default();
    let portal_service_available = service_list.contains("org.freedesktop.portal.Desktop");
    let kde_portal_service_available =
        service_list.contains("org.freedesktop.impl.portal.desktop.kde");
    let screenshot_interface_available = portal_service_available
        && command_success(
            "busctl",
            &[
                "--user",
                "--no-pager",
                "introspect",
                "org.freedesktop.portal.Desktop",
                "/org/freedesktop/portal/desktop",
                "org.freedesktop.portal.Screenshot",
            ],
        );
    let screencast_interface_available = portal_service_available
        && command_success(
            "busctl",
            &[
                "--user",
                "--no-pager",
                "introspect",
                "org.freedesktop.portal.Desktop",
                "/org/freedesktop/portal/desktop",
                "org.freedesktop.portal.ScreenCast",
            ],
        );
    let screenshot_interface_version = screenshot_interface_available
        .then(|| busctl_user_get_u32_property("org.freedesktop.portal.Screenshot", "version"))
        .flatten();
    let screenshot_available_targets_mask = screenshot_interface_available
        .then(|| {
            busctl_user_get_u32_property("org.freedesktop.portal.Screenshot", "AvailableTargets")
        })
        .flatten();
    let screenshot_available_targets = screenshot_available_targets_mask
        .map(decode_screenshot_available_targets)
        .unwrap_or_default();
    let screenshot_target_option_supported =
        screenshot_interface_version.is_some_and(|version| version >= 3);

    ScreenshotPortalStatus {
        busctl_available,
        portal_service_available,
        screenshot_interface_available,
        screenshot_interface_version,
        screenshot_available_targets_mask,
        screenshot_available_targets,
        screenshot_target_option_supported,
        screencast_interface_available,
        kde_portal_service_available,
        setup_hint: screenshot_portal_setup_hint(
            busctl_available,
            portal_service_available,
            screenshot_interface_available,
            screenshot_interface_version,
            screenshot_available_targets_mask,
            screencast_interface_available,
            kde_portal_service_available,
        ),
    }
}

fn busctl_user_get_u32_property(interface: &str, property: &str) -> Option<u32> {
    let output = command_stdout(
        "busctl",
        &[
            "--user",
            "--no-pager",
            "get-property",
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            interface,
            property,
        ],
    )
    .ok()?;
    parse_busctl_u32_property(&output)
}

fn parse_busctl_u32_property(output: &str) -> Option<u32> {
    let mut parts = output.split_whitespace();
    match (parts.next(), parts.next(), parts.next()) {
        (Some("u"), Some(value), None) => value.parse().ok(),
        _ => None,
    }
}

fn decode_screenshot_available_targets(mask: u32) -> Vec<String> {
    let mut targets = Vec::new();
    if mask & seatgeist_portal::PortalScreenshotTarget::Screen.value() != 0 {
        targets.push("screen".to_string());
    }
    if mask & seatgeist_portal::PortalScreenshotTarget::Window.value() != 0 {
        targets.push("window".to_string());
    }
    if mask & seatgeist_portal::PortalScreenshotTarget::Area.value() != 0 {
        targets.push("area".to_string());
    }
    if mask & seatgeist_portal::PortalScreenshotTarget::ActiveWindow.value() != 0 {
        targets.push("active_window".to_string());
    }
    targets
}

fn kwin_metadata_status() -> KwinMetadataStatus {
    let busctl_available = command_exists("busctl");
    let kwin_service_available = busctl_available
        && command_stdout("busctl", &["--user", "--no-pager", "--list"])
            .unwrap_or_default()
            .contains("org.kde.KWin");
    let support_information_available = command_exists("qdbus6")
        && command_success(
            "qdbus6",
            &["org.kde.KWin", "/KWin", "org.kde.KWin.supportInformation"],
        );

    KwinMetadataStatus {
        busctl_available,
        kwin_service_available,
        support_information_available,
        setup_hint: kwin_metadata_setup_hint(
            busctl_available,
            kwin_service_available,
            support_information_available,
        ),
    }
}

fn spectacle_status() -> SpectacleStatus {
    let command_available = command_exists("spectacle");
    SpectacleStatus {
        command_available,
        setup_hint: spectacle_setup_hint(command_available),
    }
}

fn preferred_capture_backend(
    screenshot_portal: &ScreenshotPortalStatus,
    spectacle_available: bool,
) -> Option<String> {
    if screenshot_portal.screenshot_interface_available {
        return Some("portal_screenshot".to_string());
    }
    if spectacle_available {
        return Some("spectacle".to_string());
    }
    None
}

fn implemented_capture_backend(
    screenshot_portal: &ScreenshotPortalStatus,
    spectacle_available: bool,
) -> Option<String> {
    if screenshot_portal.screenshot_interface_available {
        return Some("portal_screenshot".to_string());
    }
    spectacle_available.then(|| "spectacle".to_string())
}

pub(crate) fn tile_capture_backend(
    screenshot_portal: &ScreenshotPortalStatus,
    spectacle_available: bool,
) -> Option<&'static str> {
    if screenshot_portal.screenshot_interface_available {
        return Some("portal_screenshot");
    }
    spectacle_available.then_some("spectacle")
}

fn capture_backend_setup_hint(
    preferred: Option<&str>,
    implemented: Option<&str>,
    screenshot_portal: &ScreenshotPortalStatus,
    kwin_metadata: &KwinMetadataStatus,
    spectacle: &SpectacleStatus,
) -> String {
    match (preferred, implemented) {
        (Some("portal_screenshot"), Some("portal_screenshot"))
            if kwin_metadata.support_information_available =>
        {
            "using portal Screenshot for full-screen capture with KWin metadata for monitor scale and coordinate mapping; Spectacle remains the tile and compatibility fallback".to_string()
        }
        (Some("portal_screenshot"), Some("portal_screenshot")) => {
            "using portal Screenshot for full-screen capture; KWin supportInformation is unavailable, so monitor scale metadata may be incomplete and Spectacle remains the tile fallback".to_string()
        }
        (Some("portal_screenshot"), _) => {
            "portal Screenshot is visible, but no executable capture backend was selected; inspect portal diagnostics and Spectacle fallback state".to_string()
        }
        (Some("spectacle"), Some("spectacle")) if kwin_metadata.support_information_available => {
            "using Spectacle command fallback with KWin metadata for monitor scale and coordinate mapping".to_string()
        }
        (Some("spectacle"), Some("spectacle")) => {
            "using Spectacle command fallback; KWin supportInformation is unavailable, so monitor scale metadata may be incomplete".to_string()
        }
        _ if !screenshot_portal.busctl_available && !spectacle.command_available => {
            "install busctl/systemd tools or Spectacle before probing or using capture backends".to_string()
        }
        _ if !screenshot_portal.screenshot_interface_available && !spectacle.command_available => {
            "no capture backend is currently available; configure xdg-desktop-portal Screenshot or install Spectacle".to_string()
        }
        _ => "capture backend state is partial; inspect portal, KWin metadata, and Spectacle fields".to_string(),
    }
}

fn screenshot_portal_setup_hint(
    busctl_available: bool,
    portal_service_available: bool,
    screenshot_interface_available: bool,
    screenshot_interface_version: Option<u32>,
    screenshot_available_targets_mask: Option<u32>,
    screencast_interface_available: bool,
    kde_portal_service_available: bool,
) -> String {
    if !busctl_available {
        return "busctl is unavailable; cannot probe xdg-desktop-portal capture interfaces"
            .to_string();
    }
    if !portal_service_available {
        return "org.freedesktop.portal.Desktop is not visible on the user bus".to_string();
    }
    if !screenshot_interface_available && !screencast_interface_available {
        return "portal service is visible, but Screenshot and ScreenCast did not introspect successfully".to_string();
    }
    if !kde_portal_service_available {
        return "portal capture interface is visible; KDE portal backend service was not listed"
            .to_string();
    }
    if screenshot_interface_available
        && screenshot_interface_version.is_some_and(|version| version < 3)
    {
        return format!(
            "portal Screenshot v{} and KDE portal backend are visible; target-specific Screenshot v3/AvailableTargets is unavailable, so requests use the v2 full-screen contract",
            screenshot_interface_version.unwrap_or_default()
        );
    }
    if screenshot_interface_available
        && screenshot_interface_version.is_some_and(|version| version >= 3)
        && screenshot_available_targets_mask.is_none()
    {
        return "portal Screenshot v3+ and KDE portal backend are visible, but AvailableTargets did not read successfully"
            .to_string();
    }
    if screenshot_interface_available && screencast_interface_available {
        return "portal Screenshot, ScreenCast, and KDE portal backend are visible".to_string();
    }
    if screenshot_interface_available {
        return "portal Screenshot and KDE portal backend are visible".to_string();
    }
    "portal ScreenCast and KDE portal backend are visible; still need a Screenshot or stream capture implementation".to_string()
}

fn kwin_metadata_setup_hint(
    busctl_available: bool,
    kwin_service_available: bool,
    support_information_available: bool,
) -> String {
    if support_information_available {
        return "KWin supportInformation is available for monitor scale and geometry metadata"
            .to_string();
    }
    if !busctl_available {
        return "busctl is unavailable; cannot confirm org.kde.KWin on the user bus".to_string();
    }
    if !kwin_service_available {
        return "org.kde.KWin is not visible on the user bus".to_string();
    }
    "org.kde.KWin is visible, but qdbus6 supportInformation did not succeed".to_string()
}

fn spectacle_setup_hint(command_available: bool) -> String {
    if command_available {
        return "Spectacle command backend is available as the current fallback".to_string();
    }
    "Spectacle command backend is not on PATH".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn portal(available: bool, busctl_available: bool) -> ScreenshotPortalStatus {
        ScreenshotPortalStatus {
            busctl_available,
            portal_service_available: available,
            screenshot_interface_available: available,
            screenshot_interface_version: available.then_some(3),
            screenshot_available_targets_mask: available.then_some(15),
            screenshot_available_targets: if available {
                vec![
                    "screen".to_string(),
                    "window".to_string(),
                    "area".to_string(),
                    "active_window".to_string(),
                ]
            } else {
                Vec::new()
            },
            screenshot_target_option_supported: available,
            screencast_interface_available: available,
            kde_portal_service_available: available,
            setup_hint: String::new(),
        }
    }

    #[test]
    fn backend_preference_uses_portal_then_spectacle() {
        assert_eq!(
            preferred_capture_backend(&portal(true, true), true).as_deref(),
            Some("portal_screenshot")
        );
        assert_eq!(
            preferred_capture_backend(&portal(false, true), true).as_deref(),
            Some("spectacle")
        );
        assert_eq!(preferred_capture_backend(&portal(false, true), false), None);
        assert_eq!(
            implemented_capture_backend(&portal(true, true), true).as_deref(),
            Some("portal_screenshot")
        );
        assert_eq!(
            implemented_capture_backend(&portal(false, true), true).as_deref(),
            Some("spectacle")
        );
        assert_eq!(
            implemented_capture_backend(&portal(false, true), false),
            None
        );
    }

    #[test]
    fn tile_backend_prefers_portal_then_spectacle() {
        assert_eq!(
            tile_capture_backend(&portal(true, true), true),
            Some("portal_screenshot")
        );
        assert_eq!(
            tile_capture_backend(&portal(true, true), false),
            Some("portal_screenshot")
        );
        assert_eq!(
            tile_capture_backend(&portal(false, true), true),
            Some("spectacle")
        );
        assert_eq!(tile_capture_backend(&portal(false, true), false), None);
    }

    #[test]
    fn setup_hints_report_missing_probe_paths() {
        let kwin = KwinMetadataStatus {
            busctl_available: false,
            kwin_service_available: false,
            support_information_available: false,
            setup_hint: String::new(),
        };
        let spectacle = SpectacleStatus {
            command_available: false,
            setup_hint: String::new(),
        };

        let hint = capture_backend_setup_hint(None, None, &portal(false, false), &kwin, &spectacle);
        assert!(hint.contains("busctl") || hint.contains("capture backend"));
        let visible_portal_hint = capture_backend_setup_hint(
            Some("portal_screenshot"),
            Some("portal_screenshot"),
            &portal(true, true),
            &kwin,
            &spectacle,
        );
        assert!(visible_portal_hint.contains("using portal Screenshot"));
        assert!(
            screenshot_portal_setup_hint(false, false, false, None, None, false, false)
                .contains("busctl")
        );
        assert!(
            screenshot_portal_setup_hint(true, true, false, None, None, false, true)
                .contains("did not introspect")
        );
        assert!(
            screenshot_portal_setup_hint(true, true, true, Some(2), None, true, true)
                .contains("v2 full-screen contract")
        );
        assert!(kwin_metadata_setup_hint(false, false, false).contains("busctl"));
        assert!(kwin_metadata_setup_hint(true, false, false).contains("org.kde.KWin"));
        assert!(spectacle_setup_hint(false).contains("not on PATH"));
    }

    #[test]
    fn parses_portal_properties_and_target_masks() {
        assert_eq!(parse_busctl_u32_property("u 15\n"), Some(15));
        assert_eq!(parse_busctl_u32_property("s 15\n"), None);
        assert_eq!(
            decode_screenshot_available_targets(15),
            vec!["screen", "window", "area", "active_window"]
        );
    }
}
