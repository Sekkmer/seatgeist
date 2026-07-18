use std::{
    fs::OpenOptions,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use libseatgeist::SeatgeistError;
use seatgeist_backend::{
    CaptureSession, CaptureSessionLifecycle, CaptureSessionMetadata, CaptureSourceType,
    CapturedFrame, FrameRequest, FrameWaitRequest, FrameWaitResult, Screenshot,
};
use seatgeist_portal::{
    PortalScreenCastOptions, PortalScreenCastOwnedSession, ScreenCastSourceTypes,
    request_screen_cast_pipewire_zbus,
};

use crate::{
    FrameSource, NativePipeWireFrameSource, PipeWireCaptureError, PipeWireStreamTarget, Result,
    encode_bounded_png,
};

pub struct PipeWireCaptureSession<S: FrameSource> {
    metadata: CaptureSessionMetadata,
    source: Mutex<S>,
    latest: Mutex<Option<CapturedFrame>>,
    closed: Mutex<bool>,
    default_max_edge: u32,
}

pub struct OpenedPortalCaptureSession {
    pub session: PortalPipeWireCaptureSession,
    pub restore_token: Option<String>,
}

pub struct PortalPipeWireCaptureSession {
    frames: PipeWireCaptureSession<NativePipeWireFrameSource>,
    portal: tokio::sync::Mutex<Option<PortalScreenCastOwnedSession>>,
    portal_closed: Arc<AtomicBool>,
}

impl PortalPipeWireCaptureSession {
    pub async fn open(
        session_id: String,
        expected_source_type: CaptureSourceType,
        options: &PortalScreenCastOptions,
        restore_token_reference: Option<String>,
        default_max_edge: u32,
        response_timeout: Duration,
    ) -> std::result::Result<OpenedPortalCaptureSession, SeatgeistError> {
        if session_id.trim().is_empty() {
            return Err(SeatgeistError::InvalidRequest(
                "capture session id must be non-empty".to_string(),
            ));
        }
        let expected_portal_source = portal_source_type(expected_source_type)?;
        if options.select_sources.types != expected_portal_source {
            return Err(SeatgeistError::InvalidRequest(format!(
                "capture source contract {:?} does not match portal source mask {}",
                expected_source_type,
                options.select_sources.types.bits()
            )));
        }
        let mut portal = request_screen_cast_pipewire_zbus(options, response_timeout)
            .await
            .map_err(|err| SeatgeistError::Io(err.to_string()))?
            .ok_or_else(|| {
                SeatgeistError::BackendUnavailable(
                    "ScreenCast consent was cancelled or denied".to_string(),
                )
            })?;
        if portal.session_start.streams.len() != 1 {
            let _ = portal.close().await;
            return Err(SeatgeistError::InvalidRequest(format!(
                "retained {:?} capture requires exactly one portal stream, got {}",
                expected_source_type,
                portal.session_start.streams.len()
            )));
        }
        let stream = portal.session_start.streams[0].clone();
        if stream
            .source_type
            .is_some_and(|types| !types.contains(expected_portal_source))
        {
            let _ = portal.close().await;
            return Err(SeatgeistError::InvalidRequest(format!(
                "portal selected a source that does not match retained {:?} capture",
                expected_source_type
            )));
        }
        let fd = portal
            .take_pipewire_fd()
            .map_err(|err| SeatgeistError::Io(err.to_string()))?;
        let source = match NativePipeWireFrameSource::open(
            fd,
            PipeWireStreamTarget {
                node_id: stream.node_id,
                pipewire_serial: stream.pipewire_serial,
            },
        ) {
            Ok(source) => source,
            Err(err) => {
                let _ = portal.close().await;
                return Err(pipewire_backend_error(err));
            }
        };
        let restore_token = portal.session_start.restore_token.clone();
        let restore_token_reference =
            if restore_token.is_some() || options.select_sources.restore_token.is_some() {
                restore_token_reference
            } else {
                None
            };
        let source_id = stream.id.or(stream.mapping_id);
        let frames = PipeWireCaptureSession::new(
            CaptureSessionMetadata {
                id: session_id,
                backend: "portal_screencast_pipewire".to_string(),
                source_type: expected_source_type,
                source_id,
                restore_token_reference,
                consent_required: true,
                occlusion_possible: false,
            },
            source,
            default_max_edge,
        )
        .map_err(pipewire_backend_error)?;
        Ok(OpenedPortalCaptureSession {
            session: PortalPipeWireCaptureSession {
                frames,
                portal: tokio::sync::Mutex::new(Some(portal)),
                portal_closed: Arc::new(AtomicBool::new(false)),
            },
            restore_token,
        })
    }
}

fn portal_source_type(
    source_type: CaptureSourceType,
) -> std::result::Result<ScreenCastSourceTypes, SeatgeistError> {
    match source_type {
        CaptureSourceType::Window => Ok(ScreenCastSourceTypes::WINDOW),
        CaptureSourceType::Monitor => Ok(ScreenCastSourceTypes::MONITOR),
        CaptureSourceType::VirtualOutput => Ok(ScreenCastSourceTypes::VIRTUAL),
        CaptureSourceType::DesktopCompatibility => Err(SeatgeistError::InvalidRequest(
            "desktop compatibility capture is not a retained portal ScreenCast source".to_string(),
        )),
    }
}

#[async_trait]
impl CaptureSession for PortalPipeWireCaptureSession {
    fn metadata(&self) -> CaptureSessionMetadata {
        self.frames.metadata()
    }

    async fn lifecycle(&self) -> CaptureSessionLifecycle {
        if self.portal_closed.load(Ordering::SeqCst) {
            return CaptureSessionLifecycle::PortalClosed;
        }
        let frame_lifecycle = self.frames.lifecycle().await;
        if frame_lifecycle != CaptureSessionLifecycle::Open {
            return frame_lifecycle;
        }
        let mut portal_slot = self.portal.lock().await;
        let Some(portal) = portal_slot.as_mut() else {
            return CaptureSessionLifecycle::ClientClosed;
        };
        match portal.wait_closed(Duration::from_millis(1)).await {
            Ok(true) => {
                self.portal_closed.store(true, Ordering::SeqCst);
                CaptureSessionLifecycle::PortalClosed
            }
            Ok(false) => CaptureSessionLifecycle::Open,
            Err(_) => CaptureSessionLifecycle::MonitorFailed,
        }
    }

    async fn snapshot(
        &self,
        request: FrameRequest,
    ) -> std::result::Result<CapturedFrame, SeatgeistError> {
        self.frames.snapshot(request).await
    }

    async fn wait_for_frame(
        &self,
        request: FrameWaitRequest,
    ) -> std::result::Result<FrameWaitResult, SeatgeistError> {
        self.frames.wait_for_frame(request).await
    }

    async fn close(&self) -> std::result::Result<(), SeatgeistError> {
        self.frames.close().await?;
        let mut portal_slot = self.portal.lock().await;
        if let Some(mut portal) = portal_slot.take()
            && !self.portal_closed.load(Ordering::SeqCst)
        {
            portal
                .close()
                .await
                .map_err(|err| SeatgeistError::Io(err.to_string()))?;
            if portal
                .wait_closed(Duration::from_secs(1))
                .await
                .map_err(|err| SeatgeistError::Io(err.to_string()))?
            {
                self.portal_closed.store(true, Ordering::SeqCst);
            }
        }
        Ok(())
    }
}

impl<S: FrameSource> PipeWireCaptureSession<S> {
    pub fn new(metadata: CaptureSessionMetadata, source: S, default_max_edge: u32) -> Result<Self> {
        if default_max_edge == 0 {
            return Err(PipeWireCaptureError::InvalidFrame(
                "default_max_edge must be greater than zero".to_string(),
            ));
        }
        Ok(Self {
            metadata,
            source: Mutex::new(source),
            latest: Mutex::new(None),
            closed: Mutex::new(false),
            default_max_edge,
        })
    }

    fn ensure_open(&self) -> std::result::Result<(), SeatgeistError> {
        if *lock_backend(&self.closed)? {
            return Err(SeatgeistError::BackendUnavailable(format!(
                "capture session {} is closed",
                self.metadata.id
            )));
        }
        Ok(())
    }

    fn capture(
        &self,
        request: &FrameRequest,
    ) -> std::result::Result<Option<CapturedFrame>, SeatgeistError> {
        self.ensure_open()?;
        if request.output.trim().is_empty() {
            return Err(SeatgeistError::InvalidRequest(
                "frame output path must be non-empty".to_string(),
            ));
        }
        let raw = lock_backend(&self.source)?
            .next_frame(Duration::from_millis(request.timeout_ms))
            .map_err(pipewire_backend_error)?;
        let Some(raw) = raw else {
            return Ok(None);
        };
        let encoded = encode_bounded_png(&raw, request.max_edge.unwrap_or(self.default_max_edge))
            .map_err(pipewire_backend_error)?;
        write_private_png(Path::new(&request.output), &encoded.png)
            .map_err(pipewire_backend_error)?;
        let frame = CapturedFrame {
            screenshot: Screenshot {
                path: request.output.clone(),
                source_width: encoded.source_width,
                source_height: encoded.source_height,
                width: encoded.output_width,
                height: encoded.output_height,
            },
            revision: encoded.revision,
            sequence: encoded.sequence,
            complete: true,
            damage_present: encoded.damage_present,
        };
        *lock_backend(&self.latest)? = Some(frame.clone());
        Ok(Some(frame))
    }
}

#[async_trait]
impl<S: FrameSource> CaptureSession for PipeWireCaptureSession<S> {
    fn metadata(&self) -> CaptureSessionMetadata {
        self.metadata.clone()
    }

    async fn lifecycle(&self) -> CaptureSessionLifecycle {
        match self.closed.lock() {
            Ok(closed) if *closed => CaptureSessionLifecycle::ClientClosed,
            Ok(_) => CaptureSessionLifecycle::Open,
            Err(_) => CaptureSessionLifecycle::MonitorFailed,
        }
    }

    async fn snapshot(
        &self,
        request: FrameRequest,
    ) -> std::result::Result<CapturedFrame, SeatgeistError> {
        self.capture(&request)?.ok_or_else(|| {
            SeatgeistError::BackendUnavailable(format!(
                "capture session {} produced no frame within {}ms",
                self.metadata.id, request.timeout_ms
            ))
        })
    }

    async fn wait_for_frame(
        &self,
        request: FrameWaitRequest,
    ) -> std::result::Result<FrameWaitResult, SeatgeistError> {
        self.ensure_open()?;
        let started = std::time::Instant::now();
        if let Some(latest) = lock_backend(&self.latest)?.clone()
            && request
                .after_revision
                .as_deref()
                .is_none_or(|revision| revision != latest.revision)
        {
            return Ok(FrameWaitResult {
                frame: latest,
                changed: true,
                timed_out: false,
                elapsed_ms: 0,
            });
        }
        let mut frame_request = request.frame;
        frame_request.timeout_ms = request.timeout_ms;
        match self.capture(&frame_request)? {
            Some(frame) => {
                let changed = request
                    .after_revision
                    .as_deref()
                    .is_none_or(|revision| revision != frame.revision);
                Ok(FrameWaitResult {
                    frame,
                    changed,
                    timed_out: !changed,
                    elapsed_ms: elapsed_ms(started.elapsed()),
                })
            }
            None => {
                let latest = lock_backend(&self.latest)?.clone().ok_or_else(|| {
                    SeatgeistError::BackendUnavailable(format!(
                        "capture session {} timed out before its first frame",
                        self.metadata.id
                    ))
                })?;
                Ok(FrameWaitResult {
                    frame: latest,
                    changed: false,
                    timed_out: true,
                    elapsed_ms: elapsed_ms(started.elapsed()),
                })
            }
        }
    }

    async fn close(&self) -> std::result::Result<(), SeatgeistError> {
        let mut closed = lock_backend(&self.closed)?;
        if !*closed {
            lock_backend(&self.source)?
                .close()
                .map_err(pipewire_backend_error)?;
            *closed = true;
        }
        Ok(())
    }
}

fn write_private_png(path: &Path, png: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
        .map_err(|err| PipeWireCaptureError::Png(format!("open {}: {err}", path.display())))?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(|err| PipeWireCaptureError::Png(format!("chmod {}: {err}", path.display())))?;
    file.write_all(png)
        .map_err(|err| PipeWireCaptureError::Png(format!("write {}: {err}", path.display())))?;
    file.sync_all()
        .map_err(|err| PipeWireCaptureError::Png(format!("sync {}: {err}", path.display())))
}

fn lock_backend<T>(
    mutex: &Mutex<T>,
) -> std::result::Result<std::sync::MutexGuard<'_, T>, SeatgeistError> {
    mutex
        .lock()
        .map_err(|_| SeatgeistError::Io("capture session lock poisoned".to_string()))
}

fn pipewire_backend_error(error: PipeWireCaptureError) -> SeatgeistError {
    SeatgeistError::Io(error.to_string())
}

fn elapsed_ms(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        fs,
        os::unix::fs::PermissionsExt,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    struct ScriptedFrameSource {
        frames: VecDeque<Option<crate::RawVideoFrame>>,
        closed: Arc<AtomicBool>,
    }

    impl FrameSource for ScriptedFrameSource {
        fn next_frame(&mut self, _timeout: Duration) -> Result<Option<crate::RawVideoFrame>> {
            Ok(self.frames.pop_front().flatten())
        }

        fn close(&mut self) -> Result<()> {
            self.closed.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn retained_source_contract_maps_only_portal_stream_sources() {
        assert_eq!(
            portal_source_type(CaptureSourceType::Window).expect("window source"),
            ScreenCastSourceTypes::WINDOW
        );
        assert_eq!(
            portal_source_type(CaptureSourceType::Monitor).expect("monitor source"),
            ScreenCastSourceTypes::MONITOR
        );
        assert_eq!(
            portal_source_type(CaptureSourceType::VirtualOutput).expect("virtual source"),
            ScreenCastSourceTypes::VIRTUAL
        );
        assert!(portal_source_type(CaptureSourceType::DesktopCompatibility).is_err());
    }

    #[tokio::test]
    async fn retained_session_snapshots_waits_times_out_and_closes() {
        let first = solid_frame(1, [255, 0, 0, 255]);
        let second = solid_frame(2, [0, 255, 0, 255]);
        let closed = Arc::new(AtomicBool::new(false));
        let source = ScriptedFrameSource {
            frames: VecDeque::from([Some(first), Some(second), None]),
            closed: Arc::clone(&closed),
        };
        let session = PipeWireCaptureSession::new(
            CaptureSessionMetadata {
                id: "pipewire-session-1".to_string(),
                backend: "portal_screencast_pipewire".to_string(),
                source_type: CaptureSourceType::Window,
                source_id: Some("portal-window-stream".to_string()),
                restore_token_reference: None,
                consent_required: true,
                occlusion_possible: false,
            },
            source,
            800,
        )
        .expect("session constructs");
        let first_path = temporary_path("snapshot.png");
        let first = session
            .snapshot(FrameRequest {
                output: first_path.display().to_string(),
                max_edge: Some(800),
                timeout_ms: 100,
            })
            .await
            .expect("first frame snapshots");
        assert_eq!(first.sequence, 1);
        assert_eq!(
            fs::metadata(&first_path)
                .expect("snapshot metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );

        let second_path = temporary_path("wait.png");
        let changed = session
            .wait_for_frame(FrameWaitRequest {
                after_revision: Some(first.revision.clone()),
                timeout_ms: 100,
                frame: FrameRequest {
                    output: second_path.display().to_string(),
                    max_edge: Some(800),
                    timeout_ms: 100,
                },
            })
            .await
            .expect("second frame waits");
        assert!(changed.changed);
        assert!(!changed.timed_out);
        assert_eq!(changed.frame.sequence, 2);

        let timeout_path = temporary_path("timeout.png");
        let timed_out = session
            .wait_for_frame(FrameWaitRequest {
                after_revision: Some(changed.frame.revision.clone()),
                timeout_ms: 25,
                frame: FrameRequest {
                    output: timeout_path.display().to_string(),
                    max_edge: Some(800),
                    timeout_ms: 25,
                },
            })
            .await
            .expect("no-change is a watchdog result");
        assert!(!timed_out.changed);
        assert!(timed_out.timed_out);
        assert_eq!(timed_out.frame.sequence, 2);
        assert!(!timeout_path.exists());

        session.close().await.expect("session closes");
        session.close().await.expect("session close is idempotent");
        assert!(closed.load(Ordering::SeqCst));
        assert!(
            session
                .snapshot(FrameRequest {
                    output: temporary_path("closed.png").display().to_string(),
                    max_edge: Some(800),
                    timeout_ms: 10,
                })
                .await
                .is_err()
        );
        fs::remove_file(first_path).ok();
        fs::remove_file(second_path).ok();
    }

    fn solid_frame(sequence: u64, color: [u8; 4]) -> crate::RawVideoFrame {
        crate::RawVideoFrame {
            width: 2,
            height: 2,
            stride: 8,
            format: crate::RawPixelFormat::Rgba,
            sequence,
            damage_present: false,
            data: color.repeat(4),
        }
    }

    fn temporary_path(suffix: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "seatgeist-pipewire-{}-{nanos}-{suffix}",
            std::process::id()
        ))
    }
}
