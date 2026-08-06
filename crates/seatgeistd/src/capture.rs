use std::{
    collections::{HashMap, HashSet},
    fmt,
    path::PathBuf,
    sync::Arc,
};

use anyhow::{Result, bail};
use libseatgeist::{
    CaptureFrameResult, CaptureOpenRequest, CaptureSessionRequest, CaptureSessionStatus,
    CaptureSnapshotRequest, CaptureSourceKind, CaptureWaitRequest, CaptureWaitResult,
    CoordinateSpace, ScreenshotInfo, ScreenshotTransform,
};
use seatgeist_backend::{
    CaptureSession, CaptureSessionLifecycle, CaptureSessionRequest as BackendCaptureSessionRequest,
    CaptureSource, CapturedFrame, FrameRequest, FrameWaitRequest, ScreenBackend,
};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::session_owner::{SessionOwner, SessionOwnerIdentity};

#[cfg(test)]
use libseatgeist::WindowCaptureOpenRequest;

pub(crate) const MAX_CAPTURE_FRAME_TIMEOUT_MS: u64 = 30_000;
const MAX_CAPTURE_OPEN_TIMEOUT_MS: u64 = 300_000;
const MAX_EXACT_WINDOW_SESSIONS_PER_OWNER: usize = 4;

#[derive(Clone, Default)]
pub(crate) struct CaptureSessionStore {
    state: Arc<Mutex<CaptureSessionState>>,
}

impl fmt::Debug for CaptureSessionStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CaptureSessionStore(..)")
    }
}

#[derive(Default)]
struct CaptureSessionState {
    pending: HashMap<Uuid, PendingCaptureOpen>,
    sessions: HashMap<String, CaptureSessionSlot>,
    last_end_reasons: HashMap<SessionOwnerIdentity, String>,
}

struct PendingCaptureOpen {
    requested_source: CaptureSource,
    owner: SessionOwner,
    direct_exact_window: bool,
}

struct CaptureSessionSlot {
    requested_source: CaptureSource,
    owner: SessionOwner,
    latest_frame: Option<CaptureFrameResult>,
    latest_frame_user_invalidated: bool,
    direct_exact_window: bool,
    session: Arc<dyn CaptureSession>,
}

impl CaptureSessionStore {
    pub(crate) async fn active_session_ids(&self) -> HashSet<String> {
        self.reap_ended().await;
        self.state.lock().await.sessions.keys().cloned().collect()
    }

    pub(crate) async fn invalidate_latest_frames_for_window(&self, window_id: &str) -> usize {
        let mut state = self.state.lock().await;
        let mut invalidated = 0;
        for slot in state.sessions.values_mut() {
            let matches = matches!(
                &slot.requested_source,
                CaptureSource::Window {
                    requested_window_id: Some(requested),
                } if requested == window_id
            );
            if matches {
                slot.latest_frame_user_invalidated = true;
                if slot.latest_frame.take().is_some() {
                    invalidated += 1;
                }
            }
        }
        invalidated
    }

    async fn reap_ended(&self) {
        let sessions = {
            let state = self.state.lock().await;
            state
                .sessions
                .iter()
                .map(|(id, slot)| (id.clone(), Arc::clone(&slot.session)))
                .collect::<Vec<_>>()
        };
        for (session_id, session) in sessions {
            let lifecycle = session.lifecycle().await;
            let Some(reason) = lifecycle.end_reason() else {
                continue;
            };
            let removed = {
                let mut state = self.state.lock().await;
                let removed = state.sessions.remove(&session_id);
                if let Some(slot) = removed.as_ref() {
                    state
                        .last_end_reasons
                        .insert(slot.owner.identity(), reason.to_string());
                }
                removed
            };
            if let Some(slot) = removed {
                let _ = slot.session.close().await;
            }
        }
    }

    async fn begin_open(
        &self,
        requested_source: CaptureSource,
        owner: SessionOwner,
    ) -> Result<Uuid> {
        self.reap_ended().await;
        let mut state = self.state.lock().await;
        let owner_identity = owner.identity();
        let direct_exact_window = is_direct_exact_window(&requested_source);
        let owner_pending = state
            .pending
            .values()
            .filter(|pending| pending.owner.identity() == owner_identity)
            .count();
        let owner_sessions = state
            .sessions
            .values()
            .filter(|slot| slot.owner.identity() == owner_identity)
            .count();
        if !direct_exact_window && owner_pending + owner_sessions > 0 {
            bail!("this client already has an opening or active capture session");
        }
        if direct_exact_window
            && owner_pending + owner_sessions >= MAX_EXACT_WINDOW_SESSIONS_PER_OWNER
        {
            bail!(
                "this client reached the exact-window capture session quota ({MAX_EXACT_WINDOW_SESSIONS_PER_OWNER})"
            );
        }
        if !direct_exact_window
            && (state
                .pending
                .values()
                .any(|pending| !pending.direct_exact_window)
                || state
                    .sessions
                    .values()
                    .any(|slot| !slot.direct_exact_window))
        {
            bail!("a chooser-backed portal capture session is already opening or active");
        }
        let open_id = Uuid::new_v4();
        state.last_end_reasons.remove(&owner_identity);
        state.pending.insert(
            open_id,
            PendingCaptureOpen {
                requested_source,
                owner,
                direct_exact_window,
            },
        );
        Ok(open_id)
    }

    async fn finish_open(&self, open_id: Uuid, session: Box<dyn CaptureSession>) -> Result<String> {
        let mut state = self.state.lock().await;
        let pending = state
            .pending
            .remove(&open_id)
            .ok_or_else(|| anyhow::anyhow!("capture open reservation was lost"))?;
        let session: Arc<dyn CaptureSession> = Arc::from(session);
        let session_id = session.metadata().id;
        if state.sessions.contains_key(&session_id) {
            bail!("capture backend returned a duplicate session id");
        }
        state.sessions.insert(
            session_id.clone(),
            CaptureSessionSlot {
                requested_source: pending.requested_source,
                owner: pending.owner,
                latest_frame: None,
                latest_frame_user_invalidated: false,
                direct_exact_window: pending.direct_exact_window,
                session,
            },
        );
        Ok(session_id)
    }

    async fn fail_open(&self, open_id: Uuid) {
        self.state.lock().await.pending.remove(&open_id);
    }

    pub(crate) async fn status(&self) -> CaptureSessionStatus {
        self.reap_ended().await;
        let state = self.state.lock().await;
        if state.sessions.len() == 1
            && let Some(slot) = state.sessions.values().next()
        {
            return status_from_slot(slot, None);
        }
        empty_capture_status(
            !state.sessions.is_empty(),
            !state.pending.is_empty(),
            None,
            None,
            (state.last_end_reasons.len() == 1)
                .then(|| state.last_end_reasons.values().next().cloned())
                .flatten(),
        )
    }

    pub(crate) async fn status_for_owner(&self, owner: &SessionOwner) -> CaptureSessionStatus {
        self.reap_ended().await;
        let state = self.state.lock().await;
        let identity = owner.identity();
        let mut slots = state
            .sessions
            .values()
            .filter(|slot| slot.owner.identity() == identity);
        if let Some(slot) = slots.next() {
            if slots.next().is_none() {
                return status_from_slot(slot, None);
            }
            return empty_capture_status(true, false, None, Some(owner), None);
        }
        let pending = state
            .pending
            .values()
            .find(|pending| pending.owner.identity() == identity);
        empty_capture_status(
            false,
            pending.is_some(),
            pending.map(|pending| &pending.requested_source),
            pending.map(|pending| &pending.owner),
            state.last_end_reasons.get(&identity).cloned(),
        )
    }

    pub(crate) async fn status_for_session(&self, session_id: &str) -> CaptureSessionStatus {
        self.reap_ended().await;
        let state = self.state.lock().await;
        state
            .sessions
            .get(session_id)
            .map(|slot| status_from_slot(slot, None))
            .unwrap_or_else(|| empty_capture_status(false, false, None, None, None))
    }

    pub(crate) async fn require_active(&self, requested_id: &str) -> Result<()> {
        self.reap_ended().await;
        if self.state.lock().await.sessions.contains_key(requested_id) {
            Ok(())
        } else {
            bail!("interaction target lost: capture session ended or is not active")
        }
    }

    pub(crate) async fn require_owner(
        &self,
        requested_id: &str,
        client: Option<&libseatgeist::JournalClientContext>,
    ) -> Result<()> {
        self.reap_ended().await;
        let state = self.state.lock().await;
        let Some(slot) = state.sessions.get(requested_id) else {
            bail!("session owner mismatch");
        };
        slot.owner.require_matches(client)
    }

    pub(crate) async fn snapshot(
        &self,
        request: CaptureSnapshotRequest,
    ) -> Result<CaptureFrameResult> {
        self.reap_ended().await;
        let session = {
            let state = self.state.lock().await;
            Arc::clone(
                &state
                    .sessions
                    .get(&request.session_id)
                    .ok_or_else(|| anyhow::anyhow!("no active capture session with that id"))?
                    .session,
            )
        };
        let metadata = session.metadata();
        let frame = session
            .snapshot(FrameRequest {
                output: request.output.display().to_string(),
                max_edge: request.max_edge,
                timeout_ms: request.timeout_ms,
            })
            .await
            .map_err(anyhow::Error::msg)?;
        let result = capture_frame_result(
            &request.session_id,
            &metadata.backend,
            metadata.occlusion_possible,
            frame,
        );
        self.update_latest_frame(&result).await?;
        Ok(result)
    }

    pub(crate) async fn wait(&self, request: CaptureWaitRequest) -> Result<CaptureWaitResult> {
        self.reap_ended().await;
        let session = {
            let state = self.state.lock().await;
            Arc::clone(
                &state
                    .sessions
                    .get(&request.session_id)
                    .ok_or_else(|| anyhow::anyhow!("no active capture session with that id"))?
                    .session,
            )
        };
        let metadata = session.metadata();
        let timeout_ms = request.timeout_ms;
        let result = session
            .wait_for_frame(FrameWaitRequest {
                after_revision: request.after_revision,
                timeout_ms,
                frame: FrameRequest {
                    output: request.output.display().to_string(),
                    max_edge: request.max_edge,
                    timeout_ms,
                },
            })
            .await
            .map_err(anyhow::Error::msg)?;
        let frame = capture_frame_result(
            &request.session_id,
            &metadata.backend,
            metadata.occlusion_possible,
            result.frame,
        );
        self.update_latest_frame(&frame).await?;
        Ok(CaptureWaitResult {
            frame,
            changed: result.changed,
            timed_out: result.timed_out,
            timeout_ms,
            elapsed_ms: result.elapsed_ms,
        })
    }

    pub(crate) async fn update_latest_frame(&self, frame: &CaptureFrameResult) -> Result<()> {
        let mut state = self.state.lock().await;
        let slot = state
            .sessions
            .get_mut(&frame.session_id)
            .ok_or_else(|| anyhow::anyhow!("capture session ended before frame metadata update"))?;
        slot.latest_frame = Some(frame.clone());
        slot.latest_frame_user_invalidated = false;
        Ok(())
    }

    pub(crate) async fn resolve_capture_output_point(
        &self,
        session_id: &str,
        capture_revision: &str,
        point: libseatgeist::Point,
    ) -> Result<libseatgeist::Point> {
        if point.space != CoordinateSpace::CaptureOutput {
            bail!("capture output mapping requires capture_output coordinates");
        }
        let state = self.state.lock().await;
        let slot = state
            .sessions
            .get(session_id)
            .ok_or_else(|| anyhow::anyhow!("no active capture session with that id"))?;
        if slot.latest_frame_user_invalidated {
            bail!(
                "capture frame invalidated by user input; acquire a fresh frame before preview-derived pointer input"
            );
        }
        let frame = slot
            .latest_frame
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("capture session has no captured frame to map"))?;
        if frame.revision != capture_revision {
            bail!("capture revision is stale; acquire a fresh frame before pointer input");
        }
        if !point.x.is_finite()
            || !point.y.is_finite()
            || point.x < 0.0
            || point.y < 0.0
            || point.x >= f64::from(frame.screenshot.output_width)
            || point.y >= f64::from(frame.screenshot.output_height)
        {
            bail!(
                "capture_output pointer coordinate {},{} is outside preview {}x{}",
                point.x,
                point.y,
                frame.screenshot.output_width,
                frame.screenshot.output_height
            );
        }
        frame
            .screenshot
            .transform
            .output_to_source_point(point.x, point.y)
            .ok_or_else(|| anyhow::anyhow!("capture frame has an invalid coordinate transform"))
    }

    pub(crate) async fn close(
        &self,
        request: CaptureSessionRequest,
    ) -> Result<CaptureSessionStatus> {
        let slot = self
            .state
            .lock()
            .await
            .sessions
            .remove(&request.session_id)
            .ok_or_else(|| anyhow::anyhow!("no active capture session with that id"))?;
        slot.session.close().await.map_err(anyhow::Error::msg)?;
        let mut state = self.state.lock().await;
        state.last_end_reasons.insert(
            slot.owner.identity(),
            CaptureSessionLifecycle::ClientClosed
                .end_reason()
                .expect("client close has an end reason")
                .to_string(),
        );
        Ok(empty_capture_status(
            false,
            false,
            None,
            None,
            Some("client_closed".to_string()),
        ))
    }

    #[cfg(test)]
    async fn install(&self, session: Box<dyn CaptureSession>, requested_window_id: Option<String>) {
        let session: Arc<dyn CaptureSession> = Arc::from(session);
        let session_id = session.metadata().id;
        self.state.lock().await.sessions.insert(
            session_id,
            CaptureSessionSlot {
                requested_source: CaptureSource::Window {
                    requested_window_id: requested_window_id.clone(),
                },
                owner: SessionOwner::test_process(1),
                latest_frame: None,
                latest_frame_user_invalidated: false,
                direct_exact_window: requested_window_id.is_some(),
                session,
            },
        );
    }
}

fn is_direct_exact_window(source: &CaptureSource) -> bool {
    matches!(
        source,
        CaptureSource::Window {
            requested_window_id: Some(_)
        }
    )
}

fn status_from_slot(
    slot: &CaptureSessionSlot,
    last_end_reason: Option<String>,
) -> CaptureSessionStatus {
    let metadata = slot.session.metadata();
    CaptureSessionStatus {
        active: true,
        opening: false,
        session_id: Some(metadata.id),
        backend: Some(metadata.backend),
        source_type: Some(capture_source_type_name(metadata.source_type).to_string()),
        source_id: metadata.source_id,
        restore_token_reference: metadata.restore_token_reference,
        requested_window_id: requested_window_id(&slot.requested_source).map(str::to_string),
        requested_source_type: Some(
            capture_source_type_name(slot.requested_source.source_type()).to_string(),
        ),
        requested_source_id: requested_source_id(&slot.requested_source).map(str::to_string),
        owner_tool: slot.owner.tool().map(str::to_string),
        owner_pid: Some(slot.owner.pid()),
        owner_scope: Some(slot.owner.scope().as_str().to_string()),
        latest_revision: slot
            .latest_frame
            .as_ref()
            .map(|frame| frame.revision.clone()),
        consent_required: metadata.consent_required,
        occlusion_possible: metadata.occlusion_possible,
        sticky_target_bound: false,
        target_window_id: None,
        target_app_id: None,
        target_pid: None,
        target_expires_in_ms: None,
        last_end_reason,
        execution: None,
    }
}

fn empty_capture_status(
    active: bool,
    opening: bool,
    requested_source: Option<&CaptureSource>,
    owner: Option<&SessionOwner>,
    last_end_reason: Option<String>,
) -> CaptureSessionStatus {
    CaptureSessionStatus {
        active,
        opening,
        session_id: None,
        backend: None,
        source_type: None,
        source_id: None,
        restore_token_reference: None,
        requested_window_id: requested_source
            .and_then(requested_window_id)
            .map(str::to_string),
        requested_source_type: requested_source
            .map(|source| capture_source_type_name(source.source_type()).to_string()),
        requested_source_id: requested_source
            .and_then(requested_source_id)
            .map(str::to_string),
        owner_tool: owner.and_then(SessionOwner::tool).map(str::to_string),
        owner_pid: owner.map(SessionOwner::pid),
        owner_scope: owner.map(|owner| owner.scope().as_str().to_string()),
        latest_revision: None,
        consent_required: false,
        occlusion_possible: false,
        sticky_target_bound: false,
        target_window_id: None,
        target_app_id: None,
        target_pid: None,
        target_expires_in_ms: None,
        last_end_reason,
        execution: None,
    }
}

#[cfg(test)]
async fn window_capture_open(
    request: WindowCaptureOpenRequest,
    store: &CaptureSessionStore,
    backend: &dyn ScreenBackend,
    preview_max_edge: u32,
) -> Result<CaptureSessionStatus> {
    let source = CaptureSource::Window {
        requested_window_id: request.requested_window_id,
    };
    open_capture_source(
        source,
        SessionOwner::test_process(1),
        request.parent_window,
        request.timeout_ms,
        store,
        backend,
        preview_max_edge,
    )
    .await
}

pub(crate) async fn capture_open(
    request: CaptureOpenRequest,
    owner: SessionOwner,
    store: &CaptureSessionStore,
    backend: &dyn ScreenBackend,
    preview_max_edge: u32,
) -> Result<CaptureSessionStatus> {
    let requested_source_id = request
        .requested_source_id
        .map(|id| {
            let id = id.trim().to_string();
            if id.is_empty() {
                bail!("requested_source_id must not be blank");
            }
            Ok(id)
        })
        .transpose()?;
    let source = match request.source {
        CaptureSourceKind::Window => CaptureSource::Window {
            requested_window_id: requested_source_id,
        },
        CaptureSourceKind::Monitor => CaptureSource::Monitor {
            requested_monitor_id: requested_source_id,
        },
        CaptureSourceKind::VirtualOutput => {
            if requested_source_id.is_some() {
                bail!("virtual-output capture does not accept requested_source_id");
            }
            CaptureSource::VirtualOutput
        }
    };
    open_capture_source(
        source,
        owner,
        request.parent_window,
        request.timeout_ms,
        store,
        backend,
        preview_max_edge,
    )
    .await
}

async fn open_capture_source(
    source: CaptureSource,
    owner: SessionOwner,
    parent_window: String,
    timeout_ms: u64,
    store: &CaptureSessionStore,
    backend: &dyn ScreenBackend,
    preview_max_edge: u32,
) -> Result<CaptureSessionStatus> {
    if timeout_ms == 0 || timeout_ms > MAX_CAPTURE_OPEN_TIMEOUT_MS {
        bail!("capture open timeout_ms must be between 1 and {MAX_CAPTURE_OPEN_TIMEOUT_MS}");
    }
    let persist = requested_window_id(&source).is_some();
    let open_id = store.begin_open(source.clone(), owner).await?;
    let opened = backend
        .open_capture(BackendCaptureSessionRequest {
            source,
            restore_token_reference: None,
            persist,
            consent_parent_window: parent_window,
            open_timeout_ms: timeout_ms,
            default_max_edge: preview_max_edge,
        })
        .await;
    match opened {
        Ok(session) => {
            let session_id = store.finish_open(open_id, session).await?;
            Ok(store.status_for_session(&session_id).await)
        }
        Err(err) => {
            store.fail_open(open_id).await;
            Err(anyhow::Error::new(err))
        }
    }
}

pub(crate) fn normalize_capture_frame_request(
    max_edge: &mut Option<u32>,
    timeout_ms: u64,
    preview_max_edge: u32,
) -> Result<()> {
    if timeout_ms == 0 || timeout_ms > MAX_CAPTURE_FRAME_TIMEOUT_MS {
        bail!(
            "capture frame timeout_ms must be between 1 and {}",
            MAX_CAPTURE_FRAME_TIMEOUT_MS
        );
    }
    let requested_max_edge = max_edge.unwrap_or(preview_max_edge);
    if requested_max_edge == 0 || requested_max_edge > preview_max_edge {
        bail!(
            "capture frame max_edge must be between 1 and the configured preview bound ({preview_max_edge})"
        );
    }
    *max_edge = Some(requested_max_edge);
    Ok(())
}

fn capture_source_type_name(source_type: seatgeist_backend::CaptureSourceType) -> &'static str {
    match source_type {
        seatgeist_backend::CaptureSourceType::Window => "window",
        seatgeist_backend::CaptureSourceType::Monitor => "monitor",
        seatgeist_backend::CaptureSourceType::VirtualOutput => "virtual_output",
        seatgeist_backend::CaptureSourceType::DesktopCompatibility => "desktop_compatibility",
    }
}

fn requested_window_id(source: &CaptureSource) -> Option<&str> {
    match source {
        CaptureSource::Window {
            requested_window_id,
        } => requested_window_id.as_deref(),
        _ => None,
    }
}

fn requested_source_id(source: &CaptureSource) -> Option<&str> {
    match source {
        CaptureSource::Window {
            requested_window_id,
        } => requested_window_id.as_deref(),
        CaptureSource::Monitor {
            requested_monitor_id,
        } => requested_monitor_id.as_deref(),
        CaptureSource::VirtualOutput => None,
        CaptureSource::DesktopCompatibility {
            requested_window_id,
        } => requested_window_id.as_deref(),
    }
}

fn capture_frame_result(
    session_id: &str,
    backend: &str,
    occlusion_possible: bool,
    frame: CapturedFrame,
) -> CaptureFrameResult {
    let screenshot = frame.screenshot;
    CaptureFrameResult {
        session_id: session_id.to_string(),
        screenshot: ScreenshotInfo {
            path: PathBuf::from(screenshot.path),
            backend: backend.to_string(),
            occlusion_possible,
            source_width: screenshot.source_width,
            source_height: screenshot.source_height,
            output_width: screenshot.width,
            output_height: screenshot.height,
            transform: ScreenshotTransform {
                source_coordinate_space: CoordinateSpace::PhysicalPixel,
                output_coordinate_space: CoordinateSpace::CaptureOutput,
                source_extent_width: Some(screenshot.source_width),
                source_extent_height: Some(screenshot.source_height),
                source_origin_x: 0,
                source_origin_y: 0,
                scale_x: f64::from(screenshot.width) / f64::from(screenshot.source_width.max(1)),
                scale_y: f64::from(screenshot.height) / f64::from(screenshot.source_height.max(1)),
            },
            coordinate_space: CoordinateSpace::PhysicalPixel,
            monitors: Vec::new(),
        },
        revision: frame.revision,
        sequence: frame.sequence,
        complete: frame.complete,
        damage_present: frame.damage_present,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Mutex as StdMutex,
        atomic::{AtomicBool, Ordering},
    };

    fn client(tool: &str, pid: u32, process_name: &str) -> libseatgeist::JournalClientContext {
        libseatgeist::JournalClientContext {
            tool: Some(tool.to_string()),
            pid: Some(pid),
            process_name: Some(process_name.to_string()),
        }
    }

    struct MockCaptureSession {
        id: String,
        source_type: seatgeist_backend::CaptureSourceType,
        closed: Arc<AtomicBool>,
        portal_ended: Arc<AtomicBool>,
    }

    fn mock_session(id: &str) -> Box<dyn CaptureSession> {
        Box::new(MockCaptureSession {
            id: id.to_string(),
            source_type: seatgeist_backend::CaptureSourceType::Window,
            closed: Arc::new(AtomicBool::new(false)),
            portal_ended: Arc::new(AtomicBool::new(false)),
        })
    }

    #[derive(Debug, Clone)]
    struct RecordingScreenBackend {
        request: Arc<StdMutex<Option<BackendCaptureSessionRequest>>>,
    }

    #[async_trait::async_trait]
    impl ScreenBackend for RecordingScreenBackend {
        async fn capabilities(
            &self,
        ) -> seatgeist_backend::Result<seatgeist_backend::CaptureCapabilities> {
            Ok(seatgeist_backend::CaptureCapabilities {
                backend: "recording".to_string(),
                source_types: vec![seatgeist_backend::CaptureSourceType::Window],
                retained_sessions: true,
                wait_for_frame: true,
                restore_tokens: true,
                damage_tracking: true,
            })
        }

        async fn list_monitors(&self) -> seatgeist_backend::Result<Vec<libseatgeist::MonitorInfo>> {
            Ok(Vec::new())
        }

        async fn open_capture(
            &self,
            request: BackendCaptureSessionRequest,
        ) -> seatgeist_backend::Result<Box<dyn CaptureSession>> {
            let source_type = request.source.source_type();
            *self.request.lock().expect("recording request lock") = Some(request);
            Ok(Box::new(MockCaptureSession {
                id: "capture-from-trait".to_string(),
                source_type,
                closed: Arc::new(AtomicBool::new(false)),
                portal_ended: Arc::new(AtomicBool::new(false)),
            }))
        }
    }

    #[async_trait::async_trait]
    impl CaptureSession for MockCaptureSession {
        fn metadata(&self) -> seatgeist_backend::CaptureSessionMetadata {
            seatgeist_backend::CaptureSessionMetadata {
                id: self.id.clone(),
                backend: "mock_capture".to_string(),
                source_type: self.source_type,
                source_id: Some("opaque-source".to_string()),
                restore_token_reference: Some("screencast-reference".to_string()),
                consent_required: true,
                occlusion_possible: false,
            }
        }

        async fn lifecycle(&self) -> CaptureSessionLifecycle {
            if self.portal_ended.load(Ordering::SeqCst) {
                CaptureSessionLifecycle::PortalClosed
            } else {
                CaptureSessionLifecycle::Open
            }
        }

        async fn snapshot(
            &self,
            request: FrameRequest,
        ) -> seatgeist_backend::Result<CapturedFrame> {
            Ok(CapturedFrame {
                screenshot: seatgeist_backend::Screenshot {
                    path: request.output,
                    source_width: 1280,
                    source_height: 720,
                    width: 640,
                    height: 360,
                },
                revision: "revision-1".to_string(),
                sequence: 1,
                complete: true,
                damage_present: true,
            })
        }

        async fn wait_for_frame(
            &self,
            request: FrameWaitRequest,
        ) -> seatgeist_backend::Result<seatgeist_backend::FrameWaitResult> {
            Ok(seatgeist_backend::FrameWaitResult {
                frame: self.snapshot(request.frame).await?,
                changed: request.after_revision.as_deref() != Some("revision-1"),
                timed_out: false,
                elapsed_ms: 1,
            })
        }

        async fn close(&self) -> seatgeist_backend::Result<()> {
            self.closed.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn store_keeps_session_on_mismatched_close_id() {
        let store = CaptureSessionStore::default();
        let closed = Arc::new(AtomicBool::new(false));
        let portal_ended = Arc::new(AtomicBool::new(false));
        store
            .install(
                Box::new(MockCaptureSession {
                    id: "capture-correct".to_string(),
                    source_type: seatgeist_backend::CaptureSourceType::Window,
                    closed: Arc::clone(&closed),
                    portal_ended,
                }),
                Some("kwin-window-7".to_string()),
            )
            .await;

        let error = store
            .close(CaptureSessionRequest {
                session_id: "capture-wrong".to_string(),
            })
            .await
            .expect_err("mismatched close id is rejected");
        assert!(error.to_string().contains("no active capture session"));
        let active_status = store.status().await;
        assert!(active_status.active);
        assert_eq!(
            active_status.restore_token_reference.as_deref(),
            Some("screencast-reference")
        );
        assert!(!closed.load(Ordering::SeqCst));

        let status = store
            .close(CaptureSessionRequest {
                session_id: "capture-correct".to_string(),
            })
            .await
            .expect("matching close id closes session");
        assert!(!status.active);
        assert_eq!(status.last_end_reason.as_deref(), Some("client_closed"));
        assert!(closed.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn same_window_physical_activity_invalidates_only_matching_preview_frames() {
        let store = CaptureSessionStore::default();
        store
            .install(mock_session("capture-1"), Some("window-1".to_string()))
            .await;
        store
            .snapshot(CaptureSnapshotRequest {
                session_id: "capture-1".to_string(),
                output: PathBuf::from("/tmp/seatgeist-target-invalidation.png"),
                max_edge: Some(640),
                timeout_ms: 1000,
            })
            .await
            .expect("frame is retained");
        assert_eq!(
            store
                .status_for_session("capture-1")
                .await
                .latest_revision
                .as_deref(),
            Some("revision-1")
        );
        assert_eq!(
            store.invalidate_latest_frames_for_window("window-2").await,
            0
        );
        assert_eq!(
            store.invalidate_latest_frames_for_window("window-1").await,
            1
        );
        assert_eq!(
            store.status_for_session("capture-1").await.latest_revision,
            None
        );
        let error = store
            .resolve_capture_output_point(
                "capture-1",
                "revision-1",
                libseatgeist::Point {
                    x: 10.0,
                    y: 10.0,
                    space: CoordinateSpace::CaptureOutput,
                },
            )
            .await
            .expect_err("preview-derived input requires a new frame after user activity");
        assert!(error.to_string().contains("invalidated by user input"));
        store
            .snapshot(CaptureSnapshotRequest {
                session_id: "capture-1".to_string(),
                output: PathBuf::from("/tmp/seatgeist-target-refreshed.png"),
                max_edge: Some(640),
                timeout_ms: 1000,
            })
            .await
            .expect("fresh frame clears user invalidation");
        let mapped = store
            .resolve_capture_output_point(
                "capture-1",
                "revision-1",
                libseatgeist::Point {
                    x: 10.0,
                    y: 10.0,
                    space: CoordinateSpace::CaptureOutput,
                },
            )
            .await
            .expect("preview-derived input may resume after a fresh frame");
        assert_eq!(mapped.space, CoordinateSpace::PhysicalPixel);
    }

    #[tokio::test]
    async fn daemon_open_routes_the_retained_window_request_through_screen_backend() {
        let seen = Arc::new(StdMutex::new(None));
        let backend = RecordingScreenBackend {
            request: Arc::clone(&seen),
        };
        let store = CaptureSessionStore::default();
        let status = window_capture_open(
            WindowCaptureOpenRequest {
                requested_window_id: Some("kwin-window-7".to_string()),
                parent_window: "wayland:parent".to_string(),
                timeout_ms: 42_000,
            },
            &store,
            &backend,
            1_200,
        )
        .await
        .expect("trait-backed window session opens");

        assert!(status.active);
        assert_eq!(status.session_id.as_deref(), Some("capture-from-trait"));
        let request = seen
            .lock()
            .expect("recording request lock")
            .clone()
            .expect("backend request recorded");
        assert_eq!(
            request.source,
            CaptureSource::Window {
                requested_window_id: Some("kwin-window-7".to_string())
            }
        );
        assert!(request.persist);
        assert_eq!(request.consent_parent_window, "wayland:parent");
        assert_eq!(request.open_timeout_ms, 42_000);
        assert_eq!(request.default_max_edge, 1_200);
    }

    #[tokio::test]
    async fn exact_window_sessions_are_parallel_with_bounded_per_owner_quota() {
        let store = CaptureSessionStore::default();
        let exact = |id: &str| CaptureSource::Window {
            requested_window_id: Some(id.to_string()),
        };
        let first = store
            .begin_open(exact("window-1"), SessionOwner::test_process(1))
            .await
            .expect("first exact reservation opens");
        store
            .finish_open(first, mock_session("capture-1"))
            .await
            .expect("first exact session installs");
        let second = store
            .begin_open(exact("window-2"), SessionOwner::test_process(2))
            .await
            .expect("second owner may open an exact session concurrently");
        store
            .finish_open(second, mock_session("capture-2"))
            .await
            .expect("second exact session installs");

        assert_eq!(
            store
                .status_for_owner(&SessionOwner::test_process(1))
                .await
                .session_id
                .as_deref(),
            Some("capture-1")
        );
        assert_eq!(
            store
                .status_for_owner(&SessionOwner::test_process(2))
                .await
                .session_id
                .as_deref(),
            Some("capture-2")
        );
        assert!(store.status().await.active);
        assert_eq!(store.status().await.session_id, None);
        for window_id in ["window-3", "window-4", "window-5"] {
            store
                .begin_open(exact(window_id), SessionOwner::test_process(1))
                .await
                .expect("one owner may reserve several exact observation sessions");
        }
        let quota_error = store
            .begin_open(exact("window-6"), SessionOwner::test_process(1))
            .await
            .expect_err("the per-owner exact observation quota remains bounded");
        assert!(quota_error.to_string().contains("session quota"));

        let portal_source = CaptureSource::Window {
            requested_window_id: None,
        };
        let portal = store
            .begin_open(portal_source.clone(), SessionOwner::test_process(3))
            .await
            .expect("one portal session may coexist with exact observation sessions");
        store
            .finish_open(portal, mock_session("capture-portal"))
            .await
            .expect("portal session installs");
        assert!(
            store
                .begin_open(portal_source, SessionOwner::test_process(4))
                .await
                .is_err(),
            "chooser-backed portal sessions remain globally serialized"
        );
    }

    #[tokio::test]
    async fn capture_output_mapping_uses_exact_revision_and_preview_transform() {
        let store = CaptureSessionStore::default();
        store
            .install(
                mock_session("capture-map"),
                Some("kwin-window-scaled".to_string()),
            )
            .await;
        let mut frame = store
            .snapshot(CaptureSnapshotRequest {
                session_id: "capture-map".to_string(),
                output: PathBuf::from("/tmp/capture-map.png"),
                max_edge: Some(1_280),
                timeout_ms: 5_000,
            })
            .await
            .expect("frame is captured");
        frame.screenshot.output_width = 1_280;
        frame.screenshot.output_height = 720;
        frame.screenshot.transform = ScreenshotTransform {
            source_coordinate_space: CoordinateSpace::WindowLocal,
            output_coordinate_space: CoordinateSpace::CaptureOutput,
            source_extent_width: Some(1_920),
            source_extent_height: Some(1_080),
            source_origin_x: 0,
            source_origin_y: 0,
            scale_x: 1_280.0 / 1_920.0,
            scale_y: 720.0 / 1_080.0,
        };
        store
            .update_latest_frame(&frame)
            .await
            .expect("annotated transform is retained atomically");

        let point = store
            .resolve_capture_output_point(
                "capture-map",
                "revision-1",
                libseatgeist::Point {
                    x: 640.0,
                    y: 360.0,
                    space: CoordinateSpace::CaptureOutput,
                },
            )
            .await
            .expect("preview coordinate maps through DPI-aware transform");
        assert_eq!(point.space, CoordinateSpace::WindowLocal);
        assert_eq!(point.x, 960.0);
        assert_eq!(point.y, 540.0);

        let stale = store
            .resolve_capture_output_point(
                "capture-map",
                "revision-0",
                libseatgeist::Point {
                    x: 640.0,
                    y: 360.0,
                    space: CoordinateSpace::CaptureOutput,
                },
            )
            .await
            .expect_err("stale preview clicks are rejected");
        assert!(stale.to_string().contains("revision is stale"));

        let outside = store
            .resolve_capture_output_point(
                "capture-map",
                "revision-1",
                libseatgeist::Point {
                    x: 1_280.0,
                    y: 360.0,
                    space: CoordinateSpace::CaptureOutput,
                },
            )
            .await
            .expect_err("preview boundary is exclusive");
        assert!(outside.to_string().contains("outside preview"));
    }

    #[tokio::test]
    async fn expert_open_routes_monitor_intent_without_enabling_window_persistence() {
        let seen = Arc::new(StdMutex::new(None));
        let backend = RecordingScreenBackend {
            request: Arc::clone(&seen),
        };
        let store = CaptureSessionStore::default();
        let status = capture_open(
            CaptureOpenRequest {
                source: CaptureSourceKind::Monitor,
                requested_source_id: Some("DP-1".to_string()),
                parent_window: String::new(),
                timeout_ms: 30_000,
            },
            SessionOwner::test_process(1),
            &store,
            &backend,
            1_600,
        )
        .await
        .expect("monitor source routes through ScreenBackend");

        assert_eq!(status.source_type.as_deref(), Some("monitor"));
        assert_eq!(status.requested_source_type.as_deref(), Some("monitor"));
        assert_eq!(status.requested_source_id.as_deref(), Some("DP-1"));
        assert_eq!(status.requested_window_id, None);
        assert_eq!(status.owner_tool.as_deref(), Some("test-client"));
        assert_eq!(status.owner_pid, Some(1));
        assert_eq!(status.owner_scope.as_deref(), Some("process"));
        let request = seen
            .lock()
            .expect("recording request lock")
            .clone()
            .expect("backend request recorded");
        assert_eq!(
            request.source,
            CaptureSource::Monitor {
                requested_monitor_id: Some("DP-1".to_string())
            }
        );
        assert!(!request.persist);
    }

    #[tokio::test]
    async fn process_owner_rejects_a_different_mcp_process() {
        let backend = RecordingScreenBackend {
            request: Arc::new(StdMutex::new(None)),
        };
        let store = CaptureSessionStore::default();
        let opening_client = client("seatgeist-mcp", 100, "seatgeist-mcp");
        capture_open(
            CaptureOpenRequest {
                source: CaptureSourceKind::Monitor,
                requested_source_id: Some("DP-1".to_string()),
                parent_window: String::new(),
                timeout_ms: 30_000,
            },
            SessionOwner::from_client(Some(&opening_client)).expect("owner constructs"),
            &store,
            &backend,
            1_600,
        )
        .await
        .expect("capture opens");

        store
            .require_owner("capture-from-trait", Some(&opening_client))
            .await
            .expect("opening MCP process owns the session");
        let error = store
            .require_owner(
                "capture-from-trait",
                Some(&client("seatgeist-mcp", 101, "seatgeist-mcp")),
            )
            .await
            .expect_err("another MCP process cannot reuse the session");
        assert!(error.to_string().contains("session owner mismatch"));
        let wrong_id = store
            .require_owner("capture-other", Some(&opening_client))
            .await
            .expect_err("session identifiers are not an ownership oracle");
        assert_eq!(wrong_id.to_string(), error.to_string());
    }

    #[tokio::test]
    async fn verified_cli_owner_allows_a_later_cli_invocation() {
        let backend = RecordingScreenBackend {
            request: Arc::new(StdMutex::new(None)),
        };
        let store = CaptureSessionStore::default();
        let opening_client = client("seatgeist-cli", 200, "seatgeist-cli");
        capture_open(
            CaptureOpenRequest {
                source: CaptureSourceKind::Monitor,
                requested_source_id: Some("DP-1".to_string()),
                parent_window: String::new(),
                timeout_ms: 30_000,
            },
            SessionOwner::from_client(Some(&opening_client)).expect("owner constructs"),
            &store,
            &backend,
            1_600,
        )
        .await
        .expect("capture opens");

        store
            .require_owner(
                "capture-from-trait",
                Some(&client("seatgeist-cli", 201, "seatgeist-cli")),
            )
            .await
            .expect("verified CLI lifecycle is tool scoped");
    }

    #[tokio::test]
    async fn store_reaps_portal_closed_session_and_frees_the_slot() {
        let store = CaptureSessionStore::default();
        let closed = Arc::new(AtomicBool::new(false));
        let portal_ended = Arc::new(AtomicBool::new(false));
        store
            .install(
                Box::new(MockCaptureSession {
                    id: "capture-revoked".to_string(),
                    source_type: seatgeist_backend::CaptureSourceType::Window,
                    closed: Arc::clone(&closed),
                    portal_ended: Arc::clone(&portal_ended),
                }),
                Some("kwin-window-7".to_string()),
            )
            .await;

        assert!(store.status().await.active);
        portal_ended.store(true, Ordering::SeqCst);
        let status = store.status().await;
        assert!(!status.active);
        assert_eq!(status.last_end_reason.as_deref(), Some("portal_closed"));
        assert!(closed.load(Ordering::SeqCst));
        assert!(store.require_active("capture-revoked").await.is_err());

        let open_id = store
            .begin_open(
                CaptureSource::Window {
                    requested_window_id: Some("kwin-window-8".to_string()),
                },
                SessionOwner::test_process(1),
            )
            .await
            .expect("portal closure frees the retained-session slot");
        store.fail_open(open_id).await;
    }

    #[test]
    fn frame_requests_are_bounded_by_safety_config() {
        let mut default_edge = None;
        normalize_capture_frame_request(&mut default_edge, 1_500, 1600)
            .expect("default retained frame is bounded");
        assert_eq!(default_edge, Some(1600));

        let mut too_large = Some(1601);
        let error = normalize_capture_frame_request(&mut too_large, 1_500, 1600)
            .expect_err("frame larger than safety preview is rejected");
        assert!(error.to_string().contains("configured preview bound"));

        let mut valid_edge = Some(800);
        let error = normalize_capture_frame_request(
            &mut valid_edge,
            MAX_CAPTURE_FRAME_TIMEOUT_MS + 1,
            1600,
        )
        .expect_err("unbounded wait is rejected");
        assert!(error.to_string().contains("timeout_ms"));
    }
}
