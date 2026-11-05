//! Conservative delivery evidence. A dispatch-time failure never implies that
//! input was absent merely because a transport returned an error.
use libseatgeist::{ActionDisposition, DaemonResponse, ErrorKind, RetryAdvice};
pub(crate) fn annotate(response: &mut DaemonResponse, control: bool, started: bool) {
    match response {
        DaemonResponse::Action(action) if action.disposition.is_none() => {
            action.disposition = Some(if action.ok {
                ActionDisposition::delivered()
            } else {
                ActionDisposition::unknown()
            });
        }
        DaemonResponse::Error {
            disposition,
            kind,
            reason_code,
            ..
        } if disposition.is_none() => {
            let refused = !started
                || !control
                || matches!(
                    kind,
                    ErrorKind::PolicyPromptRequired
                        | ErrorKind::PolicyDenied
                        | ErrorKind::AppDenied
                        | ErrorKind::PanicStop
                        | ErrorKind::RateLimited
                        | ErrorKind::SessionOwnerMismatch
                )
                || matches!(
                    reason_code.as_deref(),
                    Some(
                        "client_binding_unavailable"
                            | "native_wayland_required"
                            | "firefox_shared_process"
                            | "agent_target_user_active"
                    )
                );
            *disposition = Some(if refused {
                ActionDisposition::not_delivered(
                    if matches!(
                        kind,
                        ErrorKind::BackendFailed
                            | ErrorKind::AccessibilityUnavailable
                            | ErrorKind::Unknown
                    ) && !control
                    {
                        RetryAdvice::Safe
                    } else {
                        RetryAdvice::AfterChange
                    },
                )
            } else {
                ActionDisposition::unknown()
            });
        }
        _ => {}
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn error(kind: ErrorKind) -> DaemonResponse {
        DaemonResponse::Error {
            kind,
            reason_code: None,
            message: "transport timeout".into(),
            disposition: None,
        }
    }
    #[test]
    fn same_error_before_and_after_dispatch_has_different_retry_advice() {
        let mut before = error(ErrorKind::BackendFailed);
        annotate(&mut before, true, false);
        let mut after = error(ErrorKind::BackendFailed);
        annotate(&mut after, true, true);
        assert!(matches!(
            before,
            DaemonResponse::Error {
                disposition: Some(ActionDisposition {
                    delivery: libseatgeist::InputDelivery::NotDelivered,
                    ..
                }),
                ..
            }
        ));
        assert!(matches!(
            after,
            DaemonResponse::Error {
                disposition: Some(ActionDisposition {
                    delivery: libseatgeist::InputDelivery::Unknown,
                    retry: RetryAdvice::Observe
                }),
                ..
            }
        ));
    }
    #[test]
    fn backend_refusal_and_read_error_cannot_claim_delivery() {
        let mut refusal = error(ErrorKind::BackendUnavailable);
        if let DaemonResponse::Error { reason_code, .. } = &mut refusal {
            *reason_code = Some("client_binding_unavailable".into());
        }
        annotate(&mut refusal, true, true);
        assert!(matches!(
            refusal,
            DaemonResponse::Error {
                disposition: Some(ActionDisposition {
                    delivery: libseatgeist::InputDelivery::NotDelivered,
                    ..
                }),
                ..
            }
        ));
        let mut read = error(ErrorKind::BackendFailed);
        annotate(&mut read, false, true);
        assert!(matches!(
            read,
            DaemonResponse::Error {
                disposition: Some(ActionDisposition {
                    retry: RetryAdvice::Safe,
                    ..
                }),
                ..
            }
        ));
    }
}
