use std::{
    fmt::{self, Display},
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result};
use libseatgeist::{RemoteDesktopEisSessionStatus, RemoteDesktopSessionProbeRequest};

use crate::portal_eis_probe::{
    eis_capability_names, ensure_remote_desktop_portal_available, remote_desktop_device_names,
    remote_desktop_probe_setup,
};

const EIS_INITIALIZATION_TIMEOUT: Duration = Duration::from_secs(3);
const EIS_INITIALIZATION_POLL_INTERVAL: Duration = Duration::from_millis(10);

pub(crate) async fn remote_desktop_eis_start(
    request: RemoteDesktopSessionProbeRequest,
    store: &PortalEisSessionStore,
) -> Result<RemoteDesktopEisSessionStatus> {
    ensure_remote_desktop_portal_available()?;
    let (requested_devices, options, timeout) = remote_desktop_probe_setup(request)?;
    let result = seatgeist_portal::request_remote_desktop_eis_zbus(
        &options,
        &seatgeist_portal::PortalConnectToEisOptions::new(),
        timeout,
    )
    .await
    .context("request stored portal RemoteDesktop EIS connection")?;
    let Some(session) = result else {
        return Ok(remote_desktop_eis_session_status(
            None,
            None,
            "portal RemoteDesktop interaction was cancelled or ended before a stored EIS session was created".to_string(),
        ));
    };

    let mut session = DaemonPortalEisSession::from_portal_session(session)
        .context("initialize stored daemon EIS runtime")?;
    let capabilities = requested_eis_capabilities(&requested_devices);
    let event_count = initialize_eis_runtime(&mut session, &capabilities).await?;
    let status = remote_desktop_eis_session_status(
        Some(session.metadata()),
        Some(session.state()),
        format!(
            "stored portal RemoteDesktop EIS session initialized and polled {} pending events; no input was sent",
            event_count
        ),
    );
    store.replace(session)?;
    Ok(status)
}

fn requested_eis_capabilities(devices: &[String]) -> Vec<seatgeist_eis::EisCapability> {
    let mut capabilities = Vec::new();
    if devices.iter().any(|device| device == "keyboard") {
        capabilities.extend([
            seatgeist_eis::EisCapability::Keyboard,
            seatgeist_eis::EisCapability::Text,
        ]);
    }
    if devices.iter().any(|device| device == "pointer") {
        capabilities.extend([
            seatgeist_eis::EisCapability::PointerAbsolute,
            seatgeist_eis::EisCapability::Button,
            seatgeist_eis::EisCapability::Scroll,
        ]);
    }
    capabilities
}

async fn initialize_eis_runtime(
    session: &mut DaemonPortalEisSession,
    capabilities: &[seatgeist_eis::EisCapability],
) -> Result<usize> {
    let plan = seatgeist_eis::EisActionPlan {
        required_capabilities: capabilities.to_vec(),
        events: Vec::new(),
    };
    let deadline = tokio::time::Instant::now() + EIS_INITIALIZATION_TIMEOUT;
    let mut event_count = 0;
    loop {
        let readiness = session.runtime.refresh_for_plan(&plan);
        event_count += readiness.snapshots.len();
        let state = session.state();
        if state.connected()
            && !state.bound_capabilities().is_empty()
            && state.devices().iter().any(|device| device.resumed)
        {
            return Ok(event_count);
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!(
                "EIS initialization timed out before a connected resumed device was available"
            );
        }
        tokio::time::sleep(EIS_INITIALIZATION_POLL_INTERVAL).await;
    }
}

pub(crate) fn remote_desktop_eis_stop(
    store: &PortalEisSessionStore,
) -> Result<RemoteDesktopEisSessionStatus> {
    let was_active = store.clear()?;
    Ok(remote_desktop_eis_session_status(
        None,
        None,
        if was_active {
            "stored portal RemoteDesktop EIS session was dropped; no input was sent".to_string()
        } else {
            "no stored portal RemoteDesktop EIS session was active".to_string()
        },
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DaemonPortalEisSessionMetadata {
    pub(crate) selected_devices: Vec<String>,
    pub(crate) clipboard_enabled: bool,
    pub(crate) restore_token: Option<String>,
    pub(crate) session_handle: String,
    pub(crate) create_request_path: String,
    pub(crate) select_request_path: String,
    pub(crate) start_request_path: String,
}

pub(crate) struct DaemonPortalEisSession<S = seatgeist_eis::LibeiSenderContext> {
    metadata: DaemonPortalEisSessionMetadata,
    pub(crate) runtime: seatgeist_eis::EisSessionRuntime<S>,
    _portal_connection: Option<zbus::Connection>,
}

pub(crate) struct PortalEisSessionStore<S = seatgeist_eis::LibeiSenderContext> {
    pub(crate) inner: Arc<Mutex<Option<DaemonPortalEisSession<S>>>>,
}

impl<S> Clone for PortalEisSessionStore<S> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<S> fmt::Debug for PortalEisSessionStore<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PortalEisSessionStore")
            .finish_non_exhaustive()
    }
}

impl<S> Default for PortalEisSessionStore<S> {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
        }
    }
}

impl<S: seatgeist_eis::EisEventSource> PortalEisSessionStore<S> {
    pub(crate) fn replace(&self, session: DaemonPortalEisSession<S>) -> Result<()> {
        let mut stored = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("portal EIS session store lock is poisoned"))?;
        *stored = Some(session);
        Ok(())
    }

    pub(crate) fn clear(&self) -> Result<bool> {
        let mut stored = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("portal EIS session store lock is poisoned"))?;
        Ok(stored.take().is_some())
    }

    pub(crate) fn active(&self) -> Result<bool> {
        let stored = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("portal EIS session store lock is poisoned"))?;
        Ok(stored.is_some())
    }

    pub(crate) fn status(&self) -> Result<RemoteDesktopEisSessionStatus> {
        let stored = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("portal EIS session store lock is poisoned"))?;
        Ok(match stored.as_ref() {
            Some(session) => remote_desktop_eis_session_status(
                Some(session.metadata()),
                Some(session.state()),
                "stored portal RemoteDesktop EIS session is active; explicit portal/libei raw input uses this session after the per-plan readiness gate passes".to_string(),
            ),
            None => remote_desktop_eis_session_status(
                None,
                None,
                "no stored portal RemoteDesktop EIS session; start one before selecting portal/libei execution".to_string(),
            ),
        })
    }
}

impl<S> PortalEisSessionStore<S>
where
    S: seatgeist_eis::EisEventSource + seatgeist_eis::EisSelectedDeviceExecutor,
    S::Error: Display,
{
    pub(crate) fn execute_ready_plan(
        &self,
        backend_name: &'static str,
        plan: &seatgeist_eis::EisActionPlan,
    ) -> Result<()> {
        let mut stored = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("portal EIS session store lock is poisoned"))?;
        let session = stored.as_mut().ok_or_else(|| {
            anyhow::anyhow!(
                "configured input backend {backend_name} requires a stored RemoteDesktop EIS session; run remote_desktop_eis_start before selecting portal/libei execution"
            )
        })?;
        session.execute_ready_plan(plan)?;
        Ok(())
    }
}

impl DaemonPortalEisSession<seatgeist_eis::LibeiSenderContext> {
    pub(crate) fn from_portal_session(
        session: seatgeist_portal::PortalRemoteDesktopEisSession,
    ) -> Result<Self> {
        let seatgeist_portal::PortalRemoteDesktopEisSession {
            session_start,
            eis,
            session_connection,
        } = session;
        let metadata =
            DaemonPortalEisSessionMetadata::from_session_start(&session_start, eis.session_handle);
        let runtime = seatgeist_eis::EisSessionRuntime::from_owned_fd(eis.fd, "Seatgeist")
            .map_err(|err| anyhow::anyhow!(err))?;
        Ok(Self {
            metadata,
            runtime,
            _portal_connection: Some(session_connection),
        })
    }
}

impl DaemonPortalEisSessionMetadata {
    fn from_session_start(
        session_start: &seatgeist_portal::PortalRemoteDesktopSessionStart,
        session_handle: String,
    ) -> Self {
        Self {
            selected_devices: remote_desktop_device_names(session_start.start.devices),
            clipboard_enabled: session_start.start.clipboard_enabled,
            restore_token: session_start.start.restore_token.clone(),
            session_handle,
            create_request_path: session_start.create_request_path.clone(),
            select_request_path: session_start.select_request_path.clone(),
            start_request_path: session_start.start_request_path.clone(),
        }
    }
}

fn remote_desktop_eis_session_status(
    metadata: Option<&DaemonPortalEisSessionMetadata>,
    state: Option<&seatgeist_eis::EisRuntimeState>,
    setup_hint: String,
) -> RemoteDesktopEisSessionStatus {
    RemoteDesktopEisSessionStatus {
        active: metadata.is_some(),
        runtime_connected: state.is_some_and(seatgeist_eis::EisRuntimeState::connected),
        bound_capabilities: state
            .map(|state| eis_capability_names(state.bound_capabilities()))
            .unwrap_or_default(),
        resumed_device_count: state
            .map(|state| {
                state
                    .devices()
                    .iter()
                    .filter(|device| device.resumed)
                    .count()
            })
            .unwrap_or_default(),
        selected_devices: metadata
            .map(|metadata| metadata.selected_devices.clone())
            .unwrap_or_default(),
        clipboard_enabled: metadata.is_some_and(|metadata| metadata.clipboard_enabled),
        restore_token: metadata.and_then(|metadata| metadata.restore_token.clone()),
        session_handle: metadata.map(|metadata| metadata.session_handle.clone()),
        create_request_path: metadata.map(|metadata| metadata.create_request_path.clone()),
        select_request_path: metadata.map(|metadata| metadata.select_request_path.clone()),
        start_request_path: metadata.map(|metadata| metadata.start_request_path.clone()),
        setup_hint,
    }
}

impl<S: seatgeist_eis::EisEventSource> DaemonPortalEisSession<S> {
    #[cfg(test)]
    pub(crate) fn from_runtime(
        session_start: seatgeist_portal::PortalRemoteDesktopSessionStart,
        session_handle: String,
        runtime: seatgeist_eis::EisSessionRuntime<S>,
    ) -> Self {
        let metadata =
            DaemonPortalEisSessionMetadata::from_session_start(&session_start, session_handle);
        Self {
            metadata,
            runtime,
            _portal_connection: None,
        }
    }

    pub(crate) fn metadata(&self) -> &DaemonPortalEisSessionMetadata {
        &self.metadata
    }

    pub(crate) fn state(&self) -> &seatgeist_eis::EisRuntimeState {
        self.runtime.state()
    }

    pub(crate) fn dispatch_pending(&mut self) -> Vec<seatgeist_eis::LibeiEventSnapshot> {
        self.runtime.dispatch_pending()
    }

    #[cfg(test)]
    pub(crate) fn refresh_execution_readiness(
        &mut self,
        plan: &seatgeist_eis::EisActionPlan,
    ) -> seatgeist_eis::EisExecutionReadiness {
        self.runtime.refresh_execution_readiness(plan)
    }
}

impl<S> DaemonPortalEisSession<S>
where
    S: seatgeist_eis::EisEventSource + seatgeist_eis::EisSelectedDeviceExecutor,
    S::Error: Display,
{
    pub(crate) fn execute_ready_plan(
        &mut self,
        plan: &seatgeist_eis::EisActionPlan,
    ) -> Result<seatgeist_eis::EisExecutedPlan> {
        self.runtime
            .execute_ready_plan(plan)
            .map_err(|err| anyhow::anyhow!("{err}"))
    }
}

#[cfg(test)]
mod tests {
    use super::requested_eis_capabilities;
    use seatgeist_eis::EisCapability;

    #[test]
    fn requested_devices_bind_all_future_keyboard_and_pointer_capabilities() {
        assert_eq!(
            requested_eis_capabilities(&["keyboard".to_string(), "pointer".to_string()]),
            vec![
                EisCapability::Keyboard,
                EisCapability::Text,
                EisCapability::PointerAbsolute,
                EisCapability::Button,
                EisCapability::Scroll,
            ]
        );
    }

    #[test]
    fn touchscreen_only_request_does_not_claim_unimplemented_eis_capabilities() {
        assert!(requested_eis_capabilities(&["touchscreen".to_string()]).is_empty());
    }
}
