//! Opt-in, payload-free boundaries for the local blocking MCP/Unix path.
use std::{
    io::Write,
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

pub fn next_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Method/stage names are static program metadata, never client arguments.
/// stderr must be drained by the owning process when diagnostics are enabled.
pub fn record(stage: &'static str, id: u64, method: Option<&'static str>, peer_pid: Option<u32>) {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    if !*ENABLED.get_or_init(|| {
        matches!(
            std::env::var("SEATGEIST_REQUEST_TIMING").as_deref(),
            Ok("1" | "true")
        )
    }) {
        return;
    }
    static START: OnceLock<Instant> = OnceLock::new();
    let value = serde_json::json!({
        "event":"seatgeist_request_timing", "pid":std::process::id(), "id":id,
        "stage":stage, "method":method, "peer_pid":peer_pid,
        "unix_time_ms":SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis(),
        "process_elapsed_ns":START.get_or_init(Instant::now).elapsed().as_nanos()
    });
    // Diagnostics never turn a broken stderr sink into a control failure.
    let _ = writeln!(std::io::stderr().lock(), "{value}");
}
