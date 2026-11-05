//! One-shot Unix transport. Never reconnect/replay after attempting a write.
use crate::{ActionDisposition, DaemonRequestEnvelope, DaemonResponse, ErrorKind, RetryAdvice};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::Path,
    time::Duration,
};

// Existing capture/portal opens allow five minutes, plus settle/capture grace.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(360);
pub fn send(
    socket: &Path,
    request: &DaemonRequestEnvelope,
) -> Result<DaemonResponse, serde_json::Error> {
    send_with_timeout(socket, request, RESPONSE_TIMEOUT)
}
fn send_with_timeout(
    socket: &Path,
    request: &DaemonRequestEnvelope,
    timeout: Duration,
) -> Result<DaemonResponse, serde_json::Error> {
    let line = serde_json::to_string(request)?;
    let control = !request.request.is_read_only();
    let mut stream = match UnixStream::connect(socket) {
        Ok(stream) => stream,
        Err(_) => return Ok(failure(false, control)),
    };
    if stream.set_read_timeout(Some(timeout)).is_err()
        || stream.set_write_timeout(Some(timeout)).is_err()
    {
        return Ok(failure(false, control));
    }
    if stream
        .write_all(line.as_bytes())
        .and_then(|_| stream.write_all(b"\n"))
        .is_err()
    {
        return Ok(failure(true, control));
    }
    let mut response = String::new();
    if BufReader::new(stream).read_line(&mut response).is_err() {
        return Ok(failure(true, control));
    }
    Ok(serde_json::from_str(&response).unwrap_or_else(|_| failure(true, control)))
}
fn failure(attempted: bool, control: bool) -> DaemonResponse {
    let uncertain = attempted && control;
    DaemonResponse::Error {
        kind: if attempted { ErrorKind::BackendFailed } else { ErrorKind::BackendUnavailable },
        reason_code: Some(if uncertain { "daemon_transport_delivery_unknown" } else { "daemon_transport_not_delivered" }.into()),
        disposition: Some(if uncertain { ActionDisposition::unknown() } else { ActionDisposition::not_delivered(if control { RetryAdvice::AfterChange } else { RetryAdvice::Safe }) }),
        message: if uncertain { "Daemon response unavailable after a write attempt. The action may still run or may have completed. Observe the exact target before deciding again; do not replay automatically." } else { "Daemon transport unavailable; this request delivered no control input." }.into(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AccessibilityAction, AccessibilityRefActionRequest, DaemonRequest};
    use std::{
        os::unix::net::UnixListener,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };
    fn request() -> DaemonRequestEnvelope {
        DaemonRequestEnvelope {
            request: DaemonRequest::AccessibilityRefAction(AccessibilityRefActionRequest {
                ref_id: "@qualified:e1".into(),
                action: AccessibilityAction::Press,
                text: None,
                destructive: false,
                target_guard: None,
            }),
            client: None,
            response_options: None,
        }
    }
    #[test]
    fn unavailable_socket_proves_non_delivery() {
        let socket = std::env::temp_dir().join(format!("sg-absent-{}.sock", uuid::Uuid::new_v4()));
        let response = send_with_timeout(&socket, &request(), Duration::from_millis(50)).unwrap();
        assert!(matches!(
            response.disposition(),
            Some(ActionDisposition {
                delivery: crate::InputDelivery::NotDelivered,
                ..
            })
        ));
    }
    #[test]
    fn lost_reply_and_timeout_are_uncertain_and_never_open_a_second_connection() {
        for timeout in [false, true] {
            let dir = std::env::temp_dir().join(format!("sg-transport-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&dir).unwrap();
            let socket = dir.join("bus");
            let listener = UnixListener::bind(&socket).unwrap();
            let received = Arc::new(AtomicUsize::new(0));
            let count = received.clone();
            let thread = std::thread::spawn(move || {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                count.fetch_add(1, Ordering::SeqCst);
                if timeout {
                    std::thread::sleep(Duration::from_millis(150));
                }
                listener.set_nonblocking(true).unwrap();
                assert!(listener.accept().is_err(), "no reconnect/replay");
            });
            let response =
                send_with_timeout(&socket, &request(), Duration::from_millis(30)).unwrap();
            assert_eq!(response.disposition(), Some(ActionDisposition::unknown()));
            thread.join().unwrap();
            assert_eq!(received.load(Ordering::SeqCst), 1);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
    #[test]
    fn response_shape_does_not_make_portal_or_lifecycle_calls_safe_reads() {
        assert!(DaemonRequest::Health.is_read_only());
        assert!(DaemonRequest::CaptureSessionStatus.is_read_only());
        assert!(
            !DaemonRequest::SetPanicStop(crate::SetPanicStopRequest { enabled: true })
                .is_read_only()
        );
        let start = DaemonRequest::RemoteDesktopEisStart(
            serde_json::from_value(serde_json::json!({})).unwrap(),
        );
        assert!(!start.is_read_only());
        assert_eq!(
            failure(true, !start.is_read_only()).disposition(),
            Some(ActionDisposition::unknown())
        );
    }
}
