use std::time::Duration;

use anyhow::{Context, Result, bail};
use libseatgeist::{
    RemoteDesktopEisProbe, RemoteDesktopPersistMode, RemoteDesktopSessionProbe,
    RemoteDesktopSessionProbeRequest,
};
use uuid::Uuid;

use crate::{
    input_diagnostics::remote_desktop_portal_status, portal_eis_session::DaemonPortalEisSession,
};

const MAX_REMOTE_DESKTOP_PROBE_TIMEOUT: Duration = Duration::from_secs(300);

pub(crate) async fn remote_desktop_session_probe(
    request: RemoteDesktopSessionProbeRequest,
) -> Result<RemoteDesktopSessionProbe> {
    ensure_remote_desktop_portal_available()?;
    let (requested_devices, options, timeout) = remote_desktop_probe_setup(request)?;

    let result = seatgeist_portal::request_remote_desktop_session_zbus(&options, timeout)
        .await
        .context("request transient portal RemoteDesktop session")?;
    let Some(session) = result else {
        return Ok(RemoteDesktopSessionProbe {
            started: false,
            requested_devices,
            selected_devices: Vec::new(),
            clipboard_enabled: false,
            restore_token: None,
            session_handle: None,
            create_request_path: None,
            select_request_path: None,
            start_request_path: None,
            transient_session_closed: true,
            setup_hint:
                "portal RemoteDesktop interaction was cancelled or ended before Start completed"
                    .to_string(),
        });
    };

    Ok(RemoteDesktopSessionProbe {
        started: true,
        requested_devices,
        selected_devices: remote_desktop_device_names(session.start.devices),
        clipboard_enabled: session.start.clipboard_enabled,
        restore_token: session.start.restore_token,
        session_handle: Some(session.session.actual_session_path),
        create_request_path: Some(session.create_request_path),
        select_request_path: Some(session.select_request_path),
        start_request_path: Some(session.start_request_path),
        transient_session_closed: true,
        setup_hint: "transient portal RemoteDesktop session reached Start; Seatgeist closed it after the probe and did not call ConnectToEIS or send input".to_string(),
    })
}

pub(crate) async fn remote_desktop_eis_probe(
    request: RemoteDesktopSessionProbeRequest,
) -> Result<RemoteDesktopEisProbe> {
    ensure_remote_desktop_portal_available()?;
    let (requested_devices, options, timeout) = remote_desktop_probe_setup(request)?;
    let result = seatgeist_portal::request_remote_desktop_eis_zbus(
        &options,
        &seatgeist_portal::PortalConnectToEisOptions::new(),
        timeout,
    )
    .await
    .context("request transient portal RemoteDesktop EIS connection")?;
    let Some(session) = result else {
        return Ok(RemoteDesktopEisProbe {
            started: false,
            eis_connected: false,
            eis_runtime_connected: false,
            eis_event_count: 0,
            eis_bound_capabilities: Vec::new(),
            eis_resumed_device_count: 0,
            requested_devices,
            selected_devices: Vec::new(),
            clipboard_enabled: false,
            restore_token: None,
            session_handle: None,
            create_request_path: None,
            select_request_path: None,
            start_request_path: None,
            eis_fd_closed: true,
            transient_session_closed: true,
            setup_hint:
                "portal RemoteDesktop interaction was cancelled or ended before EIS connected"
                    .to_string(),
        });
    };

    let mut session = DaemonPortalEisSession::from_portal_session(session)
        .context("initialize transient daemon EIS runtime")?;
    let metadata = session.metadata().clone();
    let snapshots = session.dispatch_pending();
    let runtime_connected = session.state().connected();
    let bound_capabilities = eis_capability_names(session.state().bound_capabilities());
    let resumed_device_count = session
        .state()
        .devices()
        .iter()
        .filter(|device| device.resumed)
        .count();
    drop(session);

    Ok(RemoteDesktopEisProbe {
        started: true,
        eis_connected: true,
        eis_runtime_connected: runtime_connected,
        eis_event_count: snapshots.len(),
        eis_bound_capabilities: bound_capabilities,
        eis_resumed_device_count: resumed_device_count,
        requested_devices,
        selected_devices: metadata.selected_devices,
        clipboard_enabled: metadata.clipboard_enabled,
        restore_token: metadata.restore_token,
        session_handle: Some(metadata.session_handle),
        create_request_path: Some(metadata.create_request_path),
        select_request_path: Some(metadata.select_request_path),
        start_request_path: Some(metadata.start_request_path),
        eis_fd_closed: true,
        transient_session_closed: true,
        setup_hint: "transient portal RemoteDesktop session reached Start, initialized a daemon EIS runtime, polled pending events, closed the EIS FD, and sent no input".to_string(),
    })
}

pub(crate) fn ensure_remote_desktop_portal_available() -> Result<()> {
    let portal_status = remote_desktop_portal_status();
    if !portal_status.remote_desktop_interface_available {
        bail!(
            "xdg-desktop-portal RemoteDesktop is not available: {}",
            portal_status.setup_hint
        );
    }
    Ok(())
}

pub(crate) fn remote_desktop_probe_setup(
    request: RemoteDesktopSessionProbeRequest,
) -> Result<(
    Vec<String>,
    seatgeist_portal::PortalRemoteDesktopOptions,
    Duration,
)> {
    let requested_devices = remote_desktop_requested_devices(&request);
    let device_types = remote_desktop_device_types(&request)?;
    let timeout = remote_desktop_probe_timeout(request.timeout_ms)?;
    let token_seed = Uuid::new_v4().simple().to_string();
    let mut options = seatgeist_portal::PortalRemoteDesktopOptions::new(
        format!("seatgeist_create_{token_seed}"),
        format!("seatgeist_session_{token_seed}"),
        format!("seatgeist_select_{token_seed}"),
        format!("seatgeist_start_{token_seed}"),
    );
    options.select_devices.types = Some(device_types);
    options.select_devices.restore_token = request.restore_token;
    options.select_devices.persist_mode = request.persist_mode.map(remote_desktop_persist_mode);
    options.start.parent_window = request.parent_window.unwrap_or_default();
    Ok((requested_devices, options, timeout))
}

pub(crate) fn eis_capability_names(capabilities: &[seatgeist_eis::EisCapability]) -> Vec<String> {
    capabilities
        .iter()
        .map(|capability| match capability {
            seatgeist_eis::EisCapability::PointerAbsolute => "pointer_absolute",
            seatgeist_eis::EisCapability::Keyboard => "keyboard",
            seatgeist_eis::EisCapability::Button => "button",
            seatgeist_eis::EisCapability::Scroll => "scroll",
            seatgeist_eis::EisCapability::Text => "text",
        })
        .map(str::to_string)
        .collect()
}

fn remote_desktop_requested_devices(request: &RemoteDesktopSessionProbeRequest) -> Vec<String> {
    let mut devices = Vec::new();
    if request.keyboard {
        devices.push("keyboard".to_string());
    }
    if request.pointer {
        devices.push("pointer".to_string());
    }
    if request.touchscreen {
        devices.push("touchscreen".to_string());
    }
    devices
}

pub(crate) fn remote_desktop_device_types(
    request: &RemoteDesktopSessionProbeRequest,
) -> Result<seatgeist_portal::RemoteDesktopDeviceTypes> {
    let mut bits = 0;
    if request.keyboard {
        bits |= seatgeist_portal::RemoteDesktopDeviceTypes::KEYBOARD.bits();
    }
    if request.pointer {
        bits |= seatgeist_portal::RemoteDesktopDeviceTypes::POINTER.bits();
    }
    if request.touchscreen {
        bits |= seatgeist_portal::RemoteDesktopDeviceTypes::TOUCHSCREEN.bits();
    }
    if bits == 0 {
        bail!("remote desktop session probe must request at least one input device");
    }
    seatgeist_portal::RemoteDesktopDeviceTypes::try_from(bits).map_err(|err| anyhow::anyhow!(err))
}

pub(crate) fn remote_desktop_device_names(
    devices: seatgeist_portal::RemoteDesktopDeviceTypes,
) -> Vec<String> {
    let mut names = Vec::new();
    if devices.contains(seatgeist_portal::RemoteDesktopDeviceTypes::KEYBOARD) {
        names.push("keyboard".to_string());
    }
    if devices.contains(seatgeist_portal::RemoteDesktopDeviceTypes::POINTER) {
        names.push("pointer".to_string());
    }
    if devices.contains(seatgeist_portal::RemoteDesktopDeviceTypes::TOUCHSCREEN) {
        names.push("touchscreen".to_string());
    }
    names
}

fn remote_desktop_persist_mode(
    mode: RemoteDesktopPersistMode,
) -> seatgeist_portal::RemoteDesktopPersistMode {
    match mode {
        RemoteDesktopPersistMode::DoNotPersist => {
            seatgeist_portal::RemoteDesktopPersistMode::DoNotPersist
        }
        RemoteDesktopPersistMode::ApplicationLifetime => {
            seatgeist_portal::RemoteDesktopPersistMode::ApplicationLifetime
        }
        RemoteDesktopPersistMode::ExplicitlyRevoked => {
            seatgeist_portal::RemoteDesktopPersistMode::ExplicitlyRevoked
        }
    }
}

pub(crate) fn remote_desktop_probe_timeout(timeout_ms: u64) -> Result<Duration> {
    if timeout_ms == 0 {
        bail!("remote desktop session probe timeout_ms must be greater than zero");
    }
    let timeout = Duration::from_millis(timeout_ms);
    if timeout > MAX_REMOTE_DESKTOP_PROBE_TIMEOUT {
        bail!(
            "remote desktop session probe timeout_ms must be at most {}",
            MAX_REMOTE_DESKTOP_PROBE_TIMEOUT.as_millis()
        );
    }
    Ok(timeout)
}
