use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Subcommand, ValueEnum};
use libseatgeist::{
    CaptureOpenRequest, CaptureSessionRequest, CaptureSnapshotRequest, CaptureSourceKind,
    CaptureWaitRequest, DEFAULT_REMOTE_DESKTOP_SESSION_TIMEOUT_MS,
    DEFAULT_WAIT_FOR_CHANGE_TIMEOUT_MS, DaemonRequest, WindowCaptureOpenRequest,
};

use super::screenshot_output_or_default;

#[derive(Debug, Subcommand)]
pub(crate) enum CaptureCommand {
    Open {
        #[arg(long, value_enum, default_value = "window")]
        source: CaptureSourceArg,
        #[arg(long)]
        requested_window_id: Option<String>,
        #[arg(long)]
        requested_source_id: Option<String>,
        #[arg(long, default_value = "")]
        parent_window: String,
        #[arg(long, default_value_t = DEFAULT_REMOTE_DESKTOP_SESSION_TIMEOUT_MS)]
        timeout_ms: u64,
    },
    Status,
    Renew {
        #[arg(long)]
        session_id: String,
    },
    Snapshot {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        max_edge: Option<u32>,
        #[arg(long, default_value_t = 1_500)]
        timeout_ms: u64,
    },
    Wait {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        after_revision: Option<String>,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        max_edge: Option<u32>,
        #[arg(long, default_value_t = DEFAULT_WAIT_FOR_CHANGE_TIMEOUT_MS)]
        timeout_ms: u64,
    },
    Close {
        #[arg(long)]
        session_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum CaptureSourceArg {
    Window,
    Monitor,
    VirtualOutput,
}

impl CaptureCommand {
    pub(crate) fn into_request(self) -> Result<DaemonRequest> {
        Ok(match self {
            Self::Open {
                source,
                requested_window_id,
                requested_source_id,
                parent_window,
                timeout_ms,
            } => match source {
                CaptureSourceArg::Window => {
                    if requested_window_id.is_some() && requested_source_id.is_some() {
                        bail!(
                            "capture open window accepts only one of --requested-window-id or --requested-source-id"
                        );
                    }
                    DaemonRequest::WindowCaptureOpen(WindowCaptureOpenRequest {
                        requested_window_id: requested_window_id.or(requested_source_id),
                        parent_window,
                        timeout_ms,
                    })
                }
                CaptureSourceArg::Monitor => {
                    if requested_window_id.is_some() {
                        bail!("--requested-window-id is valid only for --source window");
                    }
                    DaemonRequest::CaptureOpen(CaptureOpenRequest {
                        source: CaptureSourceKind::Monitor,
                        requested_source_id,
                        parent_window,
                        timeout_ms,
                    })
                }
                CaptureSourceArg::VirtualOutput => {
                    if requested_window_id.is_some() || requested_source_id.is_some() {
                        bail!("virtual-output capture does not accept a requested source id");
                    }
                    DaemonRequest::CaptureOpen(CaptureOpenRequest {
                        source: CaptureSourceKind::VirtualOutput,
                        requested_source_id: None,
                        parent_window,
                        timeout_ms,
                    })
                }
            },
            Self::Status => DaemonRequest::CaptureSessionStatus,
            Self::Renew { session_id } => {
                DaemonRequest::CaptureSessionRenew(CaptureSessionRequest { session_id })
            }
            Self::Snapshot {
                session_id,
                output,
                max_edge,
                timeout_ms,
            } => DaemonRequest::CaptureSnapshot(CaptureSnapshotRequest {
                session_id,
                output: screenshot_output_or_default(output, "window-snapshot")?,
                max_edge,
                timeout_ms,
            }),
            Self::Wait {
                session_id,
                after_revision,
                output,
                max_edge,
                timeout_ms,
            } => DaemonRequest::CaptureWait(CaptureWaitRequest {
                session_id,
                after_revision,
                output: screenshot_output_or_default(output, "window-wait")?,
                max_edge,
                timeout_ms,
            }),
            Self::Close { session_id } => {
                DaemonRequest::CaptureSessionClose(CaptureSessionRequest { session_id })
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renew_maps_to_bounded_session_request() {
        let request = CaptureCommand::Renew {
            session_id: "capture-1".to_string(),
        }
        .into_request()
        .expect("renew request maps");
        assert_eq!(
            request,
            DaemonRequest::CaptureSessionRenew(CaptureSessionRequest {
                session_id: "capture-1".to_string(),
            })
        );
    }

    #[test]
    fn monitor_open_maps_to_generic_retained_source_request() {
        let request = CaptureCommand::Open {
            source: CaptureSourceArg::Monitor,
            requested_window_id: None,
            requested_source_id: Some("DP-1".to_string()),
            parent_window: String::new(),
            timeout_ms: 30_000,
        }
        .into_request()
        .expect("monitor request maps");
        assert_eq!(
            request,
            DaemonRequest::CaptureOpen(CaptureOpenRequest {
                source: CaptureSourceKind::Monitor,
                requested_source_id: Some("DP-1".to_string()),
                parent_window: String::new(),
                timeout_ms: 30_000,
            })
        );
    }

    #[test]
    fn virtual_output_rejects_misleading_source_identity() {
        let error = CaptureCommand::Open {
            source: CaptureSourceArg::VirtualOutput,
            requested_window_id: None,
            requested_source_id: Some("DP-1".to_string()),
            parent_window: String::new(),
            timeout_ms: 30_000,
        }
        .into_request()
        .expect_err("virtual output cannot pretend to bind a monitor");
        assert!(error.to_string().contains("does not accept"));
    }
}
