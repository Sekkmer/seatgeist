use std::{
    env, fs,
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
};

use anyhow::{Context, Result};
use libseatgeist::{
    InputBackendStatus, LibeiStatus, RemoteDesktopPortalStatus, UinputStatus, XkbKeymapStatus,
    current_egid, current_euid,
};

use crate::{
    commands::{exists as command_exists, stdout as command_stdout, succeeds as command_success},
    config::InputBackendPreference,
};

pub(crate) fn uinput_status() -> Result<UinputStatus> {
    let path = seatgeist_uinput::uinput_path().to_path_buf();
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => Some(metadata),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(err).with_context(|| format!("stat {}", path.display())),
    };
    let available = seatgeist_uinput::available();
    let exists = metadata.is_some();
    let is_char_device = metadata
        .as_ref()
        .is_some_and(|metadata| metadata.file_type().is_char_device());
    let mode = metadata
        .as_ref()
        .map(|metadata| metadata.permissions().mode() & 0o7777);
    let owner_uid = metadata.as_ref().map(MetadataExt::uid);
    let owner_gid = metadata.as_ref().map(MetadataExt::gid);
    let process_uid = current_euid().context("read daemon effective uid")?;
    let process_gid = current_egid().context("read daemon effective gid")?;

    Ok(UinputStatus {
        path,
        available,
        exists,
        is_char_device,
        mode,
        owner_uid,
        owner_gid,
        process_uid,
        process_gid,
        setup_hint: uinput_setup_hint(available, exists, is_char_device),
    })
}

pub(crate) fn status(
    preference: InputBackendPreference,
    stored_session_active: bool,
    agent_seat_ready: bool,
    xkb_keymap: XkbKeymapStatus,
) -> Result<InputBackendStatus> {
    let uinput = uinput_status()?;
    let remote_desktop_portal = remote_desktop_portal_status();
    let libei = libei_status();
    let preferred_available_backend =
        preferred_input_backend(&remote_desktop_portal, &libei, uinput.available);
    let implemented_available_backend = implemented_input_backend(
        preference,
        uinput.available,
        stored_session_active,
        agent_seat_ready,
    );
    let setup_hint = input_backend_setup_hint(
        preference,
        preferred_available_backend.as_deref(),
        implemented_available_backend.as_deref(),
        &remote_desktop_portal,
        &libei,
        uinput.available,
        stored_session_active,
    );

    Ok(InputBackendStatus {
        uinput_available: uinput.available,
        remote_desktop_portal,
        libei,
        eis_keymap: xkb_keymap,
        configured_backend: preference.status_name().to_string(),
        preferred_available_backend,
        implemented_available_backend,
        setup_hint,
    })
}

fn uinput_setup_hint(available: bool, exists: bool, is_char_device: bool) -> String {
    if available {
        return "uinput available to daemon process".to_string();
    }
    if !exists {
        return "load the uinput kernel module and install the udev rule before starting seatgeistd"
            .to_string();
    }
    if !is_char_device {
        return "refusing /dev/uinput because it is not a character device".to_string();
    }
    "grant the daemon read/write access to /dev/uinput with the packaged udev rule, reload udev, add the user to the configured group, then restart the user session or service".to_string()
}

pub(crate) fn remote_desktop_portal_status() -> RemoteDesktopPortalStatus {
    let busctl_available = command_exists("busctl");
    if !busctl_available {
        return RemoteDesktopPortalStatus {
            busctl_available,
            portal_service_available: false,
            remote_desktop_interface_available: false,
            kde_portal_service_available: false,
            setup_hint: remote_desktop_portal_setup_hint(false, false, false, false),
        };
    }

    let service_list =
        command_stdout("busctl", &["--user", "--no-pager", "--list"]).unwrap_or_default();
    let portal_service_available = service_list.contains("org.freedesktop.portal.Desktop");
    let kde_portal_service_available =
        service_list.contains("org.freedesktop.impl.portal.desktop.kde");
    let remote_desktop_interface_available = portal_service_available
        && command_success(
            "busctl",
            &[
                "--user",
                "--no-pager",
                "introspect",
                "org.freedesktop.portal.Desktop",
                "/org/freedesktop/portal/desktop",
                "org.freedesktop.portal.RemoteDesktop",
            ],
        );

    RemoteDesktopPortalStatus {
        busctl_available,
        portal_service_available,
        remote_desktop_interface_available,
        kde_portal_service_available,
        setup_hint: remote_desktop_portal_setup_hint(
            busctl_available,
            portal_service_available,
            remote_desktop_interface_available,
            kde_portal_service_available,
        ),
    }
}

fn libei_status() -> LibeiStatus {
    let pkg_config_available = command_exists("pkg-config");
    let client_library_available =
        pkg_config_available && command_success("pkg-config", &["--exists", "libei-1.0"]);
    let socket_env_present = env::var_os("LIBEI_SOCKET").is_some();

    LibeiStatus {
        pkg_config_available,
        client_library_available,
        socket_env_present,
        setup_hint: libei_setup_hint(
            pkg_config_available,
            client_library_available,
            socket_env_present,
        ),
    }
}

fn preferred_input_backend(
    remote_desktop_portal: &RemoteDesktopPortalStatus,
    libei: &LibeiStatus,
    uinput_available: bool,
) -> Option<String> {
    if remote_desktop_portal.remote_desktop_interface_available {
        return Some("portal_remote_desktop".to_string());
    }
    if libei.socket_env_present || libei.client_library_available {
        return Some("libei".to_string());
    }
    if uinput_available {
        return Some("uinput".to_string());
    }
    None
}

fn implemented_input_backend(
    preference: InputBackendPreference,
    uinput_available: bool,
    stored_session_active: bool,
    agent_seat_ready: bool,
) -> Option<String> {
    match preference {
        InputBackendPreference::Auto | InputBackendPreference::Uinput => {
            uinput_available.then(|| "uinput".to_string())
        }
        InputBackendPreference::PortalRemoteDesktop => {
            stored_session_active.then(|| "portal_remote_desktop".to_string())
        }
        InputBackendPreference::Libei => stored_session_active.then(|| "libei".to_string()),
        InputBackendPreference::KwinAgentSeat => {
            agent_seat_ready.then(|| "kwin_agent_seat".to_string())
        }
    }
}

fn input_backend_setup_hint(
    preference: InputBackendPreference,
    preferred: Option<&str>,
    implemented: Option<&str>,
    remote_desktop_portal: &RemoteDesktopPortalStatus,
    libei: &LibeiStatus,
    uinput_available: bool,
    stored_session_active: bool,
) -> String {
    match preference {
        InputBackendPreference::KwinAgentSeat if implemented == Some("kwin_agent_seat") => {
            return "configured input backend kwin_agent_seat is registered; raw input requires an exact retained interaction session, remains policy-gated and journaled, and does not activate or raise the target window".to_string();
        }
        InputBackendPreference::KwinAgentSeat => {
            return "configured input backend kwin_agent_seat is unavailable; build, install, and enable the version-matched seatgeistagentseat KWin plugin, or select the nested portal/libei lane".to_string();
        }
        InputBackendPreference::PortalRemoteDesktop => {
            if stored_session_active {
                return "configured input backend portal_remote_desktop will use the stored RemoteDesktop EIS session after policy, panic-stop, active-window guard, and per-plan readiness checks".to_string();
            }
            if remote_desktop_portal.remote_desktop_interface_available {
                return "configured input backend portal_remote_desktop is visible; run remote_desktop_eis_start to create a stored session before raw input execution".to_string();
            }
            return "configured input backend portal_remote_desktop is not visible on the user bus; run in a KDE session with xdg-desktop-portal RemoteDesktop or configure input = \"auto\"/\"uinput\"".to_string();
        }
        InputBackendPreference::Libei => {
            if stored_session_active {
                return "configured input backend libei will use the stored EIS session after policy, panic-stop, active-window guard, and per-plan readiness checks".to_string();
            }
            if libei.socket_env_present || libei.client_library_available {
                return "configured input backend libei is visible; run remote_desktop_eis_start or provide a stored EIS session before raw input execution".to_string();
            }
            return "configured input backend libei is not visible and no stored EIS session is active; configure input = \"auto\"/\"uinput\" or create an EIS session".to_string();
        }
        InputBackendPreference::Uinput if implemented == Some("uinput") => {
            return "configured input backend uinput is available; keep it behind policy, panic-stop, active-window guards, and journal checks".to_string();
        }
        InputBackendPreference::Uinput => {
            return "configured input backend uinput is unavailable; install the uinput rule or configure input = \"auto\" after another backend lands".to_string();
        }
        InputBackendPreference::Auto => {}
    }

    match (preferred, implemented) {
        (Some("portal_remote_desktop"), Some("portal_remote_desktop")) => {
            "portal RemoteDesktop is visible and a stored EIS session is active for explicit portal_remote_desktop input".to_string()
        }
        (Some("libei"), Some("libei")) => {
            "libei support is visible and a stored EIS session is active for explicit libei input".to_string()
        }
        (Some("portal_remote_desktop"), Some("uinput")) => {
            "portal RemoteDesktop is visible; auto input currently uses uinput until an explicit stored-session backend is selected".to_string()
        }
        (Some("portal_remote_desktop"), _) => {
            "portal RemoteDesktop is visible; run remote_desktop_eis_start and select portal_remote_desktop to use the stored EIS session".to_string()
        }
        (Some("libei"), Some("uinput")) => {
            "libei client support is visible; auto input currently uses uinput until an explicit stored-session backend is selected".to_string()
        }
        (Some("libei"), _) => {
            "libei client support is visible; create or attach a stored EIS session and select libei for raw input".to_string()
        }
        (Some("uinput"), Some("uinput")) => {
            "only uinput is currently available; keep it behind policy, panic-stop, active-window guards, and journal checks".to_string()
        }
        _ if !remote_desktop_portal.busctl_available => {
            "install busctl/systemd tools or run in a user session with DBus before probing portal RemoteDesktop; configure libei or uinput fallback as needed".to_string()
        }
        _ if !remote_desktop_portal.remote_desktop_interface_available
            && !libei.client_library_available
            && !libei.socket_env_present
            && !uinput_available =>
        {
            "no input backend is currently available; configure portal RemoteDesktop/libei or install the uinput rule".to_string()
        }
        _ => "input backend state is partial; inspect individual portal, libei, and uinput fields".to_string(),
    }
}

fn remote_desktop_portal_setup_hint(
    busctl_available: bool,
    portal_service_available: bool,
    remote_desktop_interface_available: bool,
    kde_portal_service_available: bool,
) -> String {
    if !busctl_available {
        return "busctl is unavailable; cannot probe xdg-desktop-portal RemoteDesktop".to_string();
    }
    if !portal_service_available {
        return "org.freedesktop.portal.Desktop is not visible on the user bus".to_string();
    }
    if !remote_desktop_interface_available {
        return "portal service is visible, but org.freedesktop.portal.RemoteDesktop did not introspect successfully".to_string();
    }
    if !kde_portal_service_available {
        return "RemoteDesktop portal is visible; KDE portal backend service was not listed"
            .to_string();
    }
    "portal RemoteDesktop interface and KDE portal backend are visible".to_string()
}

fn libei_setup_hint(
    pkg_config_available: bool,
    client_library_available: bool,
    socket_env_present: bool,
) -> String {
    if socket_env_present {
        return "LIBEI_SOCKET is set; verify the socket belongs to the intended compositor or broker"
            .to_string();
    }
    if client_library_available {
        return "libei client library is available; an EIS connection still needs compositor or portal mediation".to_string();
    }
    if !pkg_config_available {
        return "pkg-config is unavailable; cannot probe libei client library metadata".to_string();
    }
    "libei client library metadata was not found by pkg-config".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn portal(available: bool) -> RemoteDesktopPortalStatus {
        RemoteDesktopPortalStatus {
            busctl_available: true,
            portal_service_available: available,
            remote_desktop_interface_available: available,
            kde_portal_service_available: available,
            setup_hint: String::new(),
        }
    }

    fn libei(available: bool, socket: bool) -> LibeiStatus {
        LibeiStatus {
            pkg_config_available: available,
            client_library_available: available,
            socket_env_present: socket,
            setup_hint: String::new(),
        }
    }

    #[test]
    fn uinput_hint_reports_access_state() {
        assert_eq!(
            uinput_setup_hint(true, true, true),
            "uinput available to daemon process"
        );
        assert!(uinput_setup_hint(false, false, false).contains("load the uinput kernel module"));
        assert!(uinput_setup_hint(false, true, false).contains("not a character device"));
        assert!(
            uinput_setup_hint(false, true, true).contains("grant the daemon read/write access")
        );
    }

    #[test]
    fn backend_preference_uses_portal_libei_then_uinput() {
        assert_eq!(
            preferred_input_backend(&portal(true), &libei(true, false), true).as_deref(),
            Some("portal_remote_desktop")
        );
        assert_eq!(
            preferred_input_backend(&portal(false), &libei(true, false), true).as_deref(),
            Some("libei")
        );
        assert_eq!(
            preferred_input_backend(&portal(false), &libei(false, false), true).as_deref(),
            Some("uinput")
        );
        assert_eq!(
            preferred_input_backend(&portal(false), &libei(false, false), false),
            None
        );
        assert_eq!(
            implemented_input_backend(InputBackendPreference::Auto, true, false, false).as_deref(),
            Some("uinput")
        );
        assert_eq!(
            implemented_input_backend(InputBackendPreference::Uinput, true, false, false)
                .as_deref(),
            Some("uinput")
        );
        assert_eq!(
            implemented_input_backend(
                InputBackendPreference::PortalRemoteDesktop,
                true,
                false,
                false,
            ),
            None
        );
        assert_eq!(
            implemented_input_backend(InputBackendPreference::Libei, true, false, false),
            None
        );
        assert_eq!(
            implemented_input_backend(
                InputBackendPreference::PortalRemoteDesktop,
                true,
                true,
                false,
            )
            .as_deref(),
            Some("portal_remote_desktop")
        );
        assert_eq!(
            implemented_input_backend(InputBackendPreference::Libei, true, true, false).as_deref(),
            Some("libei")
        );
        assert_eq!(
            implemented_input_backend(InputBackendPreference::Auto, false, true, false),
            None
        );
        assert_eq!(
            implemented_input_backend(InputBackendPreference::KwinAgentSeat, false, false, true,)
                .as_deref(),
            Some("kwin_agent_seat")
        );
    }

    #[test]
    fn setup_hints_report_missing_and_stored_paths() {
        let missing_portal = RemoteDesktopPortalStatus {
            busctl_available: false,
            portal_service_available: false,
            remote_desktop_interface_available: false,
            kde_portal_service_available: false,
            setup_hint: String::new(),
        };
        let no_libei = libei(false, false);
        let hint = input_backend_setup_hint(
            InputBackendPreference::Auto,
            None,
            None,
            &missing_portal,
            &no_libei,
            false,
            false,
        );
        assert!(hint.contains("busctl"));

        let visible_portal_hint = input_backend_setup_hint(
            InputBackendPreference::Auto,
            Some("portal_remote_desktop"),
            None,
            &portal(true),
            &no_libei,
            false,
            false,
        );
        assert!(visible_portal_hint.contains("remote_desktop_eis_start"));

        let configured_portal_hint = input_backend_setup_hint(
            InputBackendPreference::PortalRemoteDesktop,
            Some("portal_remote_desktop"),
            None,
            &portal(true),
            &no_libei,
            true,
            false,
        );
        assert!(configured_portal_hint.contains("remote_desktop_eis_start"));

        let configured_active_portal_hint = input_backend_setup_hint(
            InputBackendPreference::PortalRemoteDesktop,
            Some("portal_remote_desktop"),
            Some("portal_remote_desktop"),
            &portal(true),
            &no_libei,
            true,
            true,
        );
        assert!(configured_active_portal_hint.contains("stored RemoteDesktop EIS session"));

        let configured_uinput_hint = input_backend_setup_hint(
            InputBackendPreference::Uinput,
            Some("portal_remote_desktop"),
            Some("uinput"),
            &portal(true),
            &no_libei,
            true,
            false,
        );
        assert!(configured_uinput_hint.contains("configured input backend uinput is available"));
        assert!(remote_desktop_portal_setup_hint(false, false, false, false).contains("busctl"));
        assert!(
            remote_desktop_portal_setup_hint(true, true, false, true)
                .contains("did not introspect")
        );
        assert!(libei_setup_hint(false, false, false).contains("pkg-config"));
        assert!(libei_setup_hint(true, false, true).contains("LIBEI_SOCKET"));
    }
}
