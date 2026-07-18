use std::time::Duration;

use async_trait::async_trait;
use libseatgeist::SeatgeistError;
use seatgeist_backend::{
    CaptureCapabilities, CaptureSession, CaptureSessionRequest, CaptureSource, CaptureSourceType,
    Result as BackendResult, ScreenBackend,
};
use seatgeist_pipewire::PortalPipeWireCaptureSession;
use seatgeist_portal::{
    PortalScreenCastOptions, RemoteDesktopPersistMode, ScreenCastCursorMode, ScreenCastSourceTypes,
};
use uuid::Uuid;

use crate::capture_restore::{CaptureRestoreTokenStore, StoredRestoreToken};

const MAX_CAPTURE_OPEN_TIMEOUT_MS: u64 = 300_000;

#[derive(Debug, Clone)]
pub(crate) struct PortalScreenBackend {
    restore_store: CaptureRestoreTokenStore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CaptureSourceContract {
    source_type: CaptureSourceType,
    portal_source_type: ScreenCastSourceTypes,
    restore_target: Option<String>,
}

impl PortalScreenBackend {
    pub(crate) fn new(restore_store: CaptureRestoreTokenStore) -> Self {
        Self { restore_store }
    }

    fn validate_request(request: &CaptureSessionRequest) -> BackendResult<CaptureSourceContract> {
        if request.open_timeout_ms == 0 || request.open_timeout_ms > MAX_CAPTURE_OPEN_TIMEOUT_MS {
            return Err(SeatgeistError::InvalidRequest(format!(
                "capture open timeout_ms must be between 1 and {MAX_CAPTURE_OPEN_TIMEOUT_MS}"
            )));
        }
        if request.default_max_edge == 0 {
            return Err(SeatgeistError::InvalidRequest(
                "capture default_max_edge must be greater than zero".to_string(),
            ));
        }
        let contract = match &request.source {
            CaptureSource::Window {
                requested_window_id,
            } => CaptureSourceContract {
                source_type: CaptureSourceType::Window,
                portal_source_type: ScreenCastSourceTypes::WINDOW,
                restore_target: requested_window_id.clone(),
            },
            CaptureSource::Monitor {
                requested_monitor_id,
            } => {
                if requested_monitor_id
                    .as_deref()
                    .is_some_and(|id| id.trim().is_empty())
                {
                    return Err(SeatgeistError::InvalidRequest(
                        "requested monitor id must not be blank".to_string(),
                    ));
                }
                CaptureSourceContract {
                    source_type: CaptureSourceType::Monitor,
                    portal_source_type: ScreenCastSourceTypes::MONITOR,
                    restore_target: None,
                }
            }
            CaptureSource::VirtualOutput => CaptureSourceContract {
                source_type: CaptureSourceType::VirtualOutput,
                portal_source_type: ScreenCastSourceTypes::VIRTUAL,
                restore_target: None,
            },
            CaptureSource::DesktopCompatibility { .. } => {
                return Err(SeatgeistError::BackendUnavailable(
                    "desktop compatibility capture requires an explicit compatibility backend"
                        .to_string(),
                ));
            }
        };
        if request.persist && contract.restore_target.is_none() {
            return Err(SeatgeistError::InvalidRequest(
                "persistent capture requires an exact requested window id".to_string(),
            ));
        }
        if !request.persist && request.restore_token_reference.is_some() {
            return Err(SeatgeistError::InvalidRequest(
                "capture restore-token reference requires persistence".to_string(),
            ));
        }
        Ok(contract)
    }
}

#[async_trait]
impl ScreenBackend for PortalScreenBackend {
    async fn capabilities(&self) -> BackendResult<CaptureCapabilities> {
        Ok(CaptureCapabilities {
            backend: "portal_screencast_pipewire".to_string(),
            source_types: vec![
                CaptureSourceType::Window,
                CaptureSourceType::Monitor,
                CaptureSourceType::VirtualOutput,
            ],
            retained_sessions: true,
            wait_for_frame: true,
            restore_tokens: true,
            damage_tracking: true,
        })
    }

    async fn list_monitors(&self) -> BackendResult<Vec<libseatgeist::MonitorInfo>> {
        seatgeist_kwin::list_monitors().map_err(|err| {
            SeatgeistError::BackendUnavailable(format!("KWin monitor discovery failed: {err}"))
        })
    }

    async fn open_capture(
        &self,
        request: CaptureSessionRequest,
    ) -> BackendResult<Box<dyn CaptureSession>> {
        let contract = Self::validate_request(&request)?;
        let requested_window_id = contract.restore_target.clone();
        let stored_restore = if request.persist {
            requested_window_id
                .as_deref()
                .map(|window_id| self.restore_store.load(window_id))
                .transpose()
                .map_err(restore_store_error)?
                .flatten()
        } else {
            None
        };
        let restore_reference = if request.persist {
            let window_id = requested_window_id
                .as_deref()
                .expect("persistent request validation requires a window id");
            let reference = match stored_restore.as_ref() {
                Some(stored) => stored.reference.clone(),
                None => self
                    .restore_store
                    .reference_for(window_id)
                    .map_err(restore_store_error)?,
            };
            if request
                .restore_token_reference
                .as_deref()
                .is_some_and(|provided| provided != reference)
            {
                return Err(SeatgeistError::InvalidRequest(
                    "capture restore-token reference does not match the requested window"
                        .to_string(),
                ));
            }
            Some(reference)
        } else {
            None
        };

        let token = Uuid::new_v4().simple().to_string();
        let mut options = PortalScreenCastOptions::new_for_source(
            format!("seatgeist_sc_create_{token}"),
            format!("seatgeist_sc_session_{token}"),
            format!("seatgeist_sc_select_{token}"),
            format!("seatgeist_sc_start_{token}"),
            contract.portal_source_type,
        );
        options.start.parent_window = request.consent_parent_window;
        options.select_sources.cursor_mode = ScreenCastCursorMode::Embedded;
        configure_restore_options(&mut options, request.persist, stored_restore.as_ref());

        let session_id = format!("capture-{}", Uuid::new_v4().simple());
        let opened = PortalPipeWireCaptureSession::open(
            session_id,
            contract.source_type,
            &options,
            restore_reference,
            request.default_max_edge,
            Duration::from_millis(request.open_timeout_ms),
        )
        .await?;

        if let (Some(window_id), Some(rotated_token)) = (
            requested_window_id.as_deref(),
            opened.restore_token.as_deref(),
        ) && let Err(err) = self.restore_store.save(window_id, rotated_token)
        {
            let _ = opened.session.close().await;
            return Err(restore_store_error(err));
        }
        Ok(Box::new(opened.session))
    }
}

fn configure_restore_options(
    options: &mut PortalScreenCastOptions,
    persist: bool,
    stored_restore: Option<&StoredRestoreToken>,
) {
    if !persist {
        return;
    }
    options.select_sources.persist_mode = RemoteDesktopPersistMode::ExplicitlyRevoked;
    options.select_sources.restore_token = stored_restore.map(|stored| stored.token.clone());
}

fn restore_store_error(err: anyhow::Error) -> SeatgeistError {
    SeatgeistError::Io(format!("ScreenCast restore-token state failed: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn request(source: CaptureSource) -> CaptureSessionRequest {
        CaptureSessionRequest {
            source,
            restore_token_reference: None,
            persist: false,
            consent_parent_window: String::new(),
            open_timeout_ms: 30_000,
            default_max_edge: 1_600,
        }
    }

    #[tokio::test]
    async fn capabilities_advertise_the_three_retained_portal_source_contracts() {
        let backend = PortalScreenBackend::new(CaptureRestoreTokenStore::new(PathBuf::from(
            "/unused/restore.json",
        )));
        let capabilities = backend.capabilities().await.expect("capabilities");
        assert_eq!(
            capabilities.source_types,
            vec![
                CaptureSourceType::Window,
                CaptureSourceType::Monitor,
                CaptureSourceType::VirtualOutput,
            ]
        );
        assert!(capabilities.retained_sessions);
        assert!(capabilities.restore_tokens);
    }

    #[test]
    fn retained_sources_map_exactly_and_unsafe_persistence_fails_before_portal_use() {
        let monitor = request(CaptureSource::Monitor {
            requested_monitor_id: Some("DP-1".to_string()),
        });
        let contract = PortalScreenBackend::validate_request(&monitor)
            .expect("monitor source contract is supported");
        assert_eq!(contract.source_type, CaptureSourceType::Monitor);
        assert_eq!(contract.portal_source_type, ScreenCastSourceTypes::MONITOR);

        let virtual_output = request(CaptureSource::VirtualOutput);
        let contract = PortalScreenBackend::validate_request(&virtual_output)
            .expect("virtual source contract is supported");
        assert_eq!(contract.source_type, CaptureSourceType::VirtualOutput);
        assert_eq!(contract.portal_source_type, ScreenCastSourceTypes::VIRTUAL);

        let compatibility = request(CaptureSource::DesktopCompatibility {
            requested_window_id: None,
        });
        let error = PortalScreenBackend::validate_request(&compatibility)
            .expect_err("desktop compatibility requires its own backend");
        assert!(error.to_string().contains("explicit compatibility backend"));

        let mut unbound_persist = request(CaptureSource::Window {
            requested_window_id: None,
        });
        unbound_persist.persist = true;
        let error = PortalScreenBackend::validate_request(&unbound_persist)
            .expect_err("unbound persistence fails closed");
        assert!(error.to_string().contains("exact requested window id"));

        let mut monitor_persist = request(CaptureSource::Monitor {
            requested_monitor_id: Some("DP-1".to_string()),
        });
        monitor_persist.persist = true;
        let error = PortalScreenBackend::validate_request(&monitor_persist)
            .expect_err("monitor persistence is not silently treated as window persistence");
        assert!(error.to_string().contains("exact requested window id"));
    }

    #[test]
    fn restore_options_never_persist_without_explicit_request() {
        let stored = StoredRestoreToken {
            token: "private-restore-token".to_string(),
            reference: "screencast-reference".to_string(),
        };
        let mut options = PortalScreenCastOptions::new_window(
            "create_token",
            "session_token",
            "select_token",
            "start_token",
        );
        configure_restore_options(&mut options, true, Some(&stored));
        assert_eq!(
            options.select_sources.persist_mode,
            RemoteDesktopPersistMode::ExplicitlyRevoked
        );
        assert_eq!(
            options.select_sources.restore_token.as_deref(),
            Some("private-restore-token")
        );

        let mut compatibility = PortalScreenCastOptions::new_window(
            "create_other",
            "session_other",
            "select_other",
            "start_other",
        );
        configure_restore_options(&mut compatibility, false, Some(&stored));
        assert_eq!(
            compatibility.select_sources.persist_mode,
            RemoteDesktopPersistMode::DoNotPersist
        );
        assert!(compatibility.select_sources.restore_token.is_none());
    }
}
