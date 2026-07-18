use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Result, bail};
use serde::Deserialize;

pub(crate) const KWIN_INPUT_SPY_BACKEND: &str = "kwin_input_spy_v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ActivityClass {
    Keyboard,
    Pointer,
    Touch,
}

impl ActivityClass {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Keyboard => "keyboard",
            Self::Pointer => "pointer",
            Self::Touch => "touch",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ActivityProvenance {
    TrustedPhysical,
    SeatgeistInjected,
    Unknown,
}

impl ActivityProvenance {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::TrustedPhysical => "trusted_physical",
            Self::SeatgeistInjected => "seatgeist_injected",
            Self::Unknown => "unknown",
        }
    }

    fn is_interference(self) -> bool {
        self != Self::SeatgeistInjected
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActivityPayload {
    backend: String,
    seat: String,
    class: ActivityClass,
    provenance: ActivityProvenance,
    monotonic_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ActivitySnapshot {
    backend_generation: u64,
    interference_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActivityStatus {
    pub backend: Option<String>,
    pub trusted: bool,
    pub last_class: Option<&'static str>,
    pub last_provenance: Option<&'static str>,
}

#[derive(Debug, Default)]
struct ActivityState {
    backend: Option<String>,
    backend_generation: u64,
    interference_generation: u64,
    last_class: Option<ActivityClass>,
    last_provenance: Option<ActivityProvenance>,
    last_interference: Option<Instant>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ActivityTracker {
    state: Arc<Mutex<ActivityState>>,
}

impl ActivityTracker {
    pub(crate) fn register_backend(&self, backend: &str) -> Result<()> {
        validate_backend(backend)?;
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.backend.as_deref() != Some(backend) {
            state.backend = Some(backend.to_string());
            state.backend_generation = state.backend_generation.saturating_add(1);
        }
        Ok(())
    }

    pub(crate) fn record_payload(&self, payload: &str) -> Result<()> {
        let payload: ActivityPayload = serde_json::from_str(payload)?;
        validate_backend(&payload.backend)?;
        if payload.seat != "default" {
            bail!("unsupported activity seat");
        }
        // A monotonic value is required from the compositor, but comparisons
        // use daemon receipt order so plugin restart epochs cannot be confused.
        let _source_monotonic_ms = payload.monotonic_ms;
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.backend.as_deref() != Some(payload.backend.as_str()) {
            state.backend = Some(payload.backend);
            state.backend_generation = state.backend_generation.saturating_add(1);
        }
        state.last_class = Some(payload.class);
        state.last_provenance = Some(payload.provenance);
        if payload.provenance.is_interference() {
            state.interference_generation = state.interference_generation.saturating_add(1);
            state.last_interference = Some(Instant::now());
        }
        Ok(())
    }

    pub(crate) fn snapshot(&self) -> ActivitySnapshot {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        ActivitySnapshot {
            backend_generation: state.backend_generation,
            interference_generation: state.interference_generation,
        }
    }

    pub(crate) fn safe_since(&self, snapshot: ActivitySnapshot) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.backend.as_deref() == Some(KWIN_INPUT_SPY_BACKEND)
            && state.backend_generation == snapshot.backend_generation
            && state.interference_generation == snapshot.interference_generation
    }

    pub(crate) fn interference_state(&self, quiet_for: Duration) -> (bool, Option<u64>) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(last) = state.last_interference else {
            return (false, None);
        };
        let age = last.elapsed();
        (
            age <= quiet_for,
            Some(age.as_millis().min(u128::from(u64::MAX)) as u64),
        )
    }

    pub(crate) fn status(&self) -> ActivityStatus {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        ActivityStatus {
            backend: state.backend.clone(),
            trusted: state.backend.as_deref() == Some(KWIN_INPUT_SPY_BACKEND),
            last_class: state.last_class.map(ActivityClass::as_str),
            last_provenance: state.last_provenance.map(ActivityProvenance::as_str),
        }
    }
}

fn validate_backend(backend: &str) -> Result<()> {
    if backend != KWIN_INPUT_SPY_BACKEND {
        bail!("unsupported input activity backend");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(provenance: &str) -> String {
        format!(
            r#"{{"backend":"kwin_input_spy_v1","seat":"default","class":"pointer","provenance":"{provenance}","monotonic_ms":42}}"#
        )
    }

    #[test]
    fn injected_activity_does_not_invalidate_restoration_snapshot() {
        let tracker = ActivityTracker::default();
        tracker
            .register_backend(KWIN_INPUT_SPY_BACKEND)
            .expect("backend registers");
        let snapshot = tracker.snapshot();
        tracker
            .record_payload(&payload("seatgeist_injected"))
            .expect("injected activity records");
        assert!(tracker.safe_since(snapshot));
    }

    #[test]
    fn physical_and_unknown_activity_invalidate_restoration_snapshot() {
        for provenance in ["trusted_physical", "unknown"] {
            let tracker = ActivityTracker::default();
            tracker
                .register_backend(KWIN_INPUT_SPY_BACKEND)
                .expect("backend registers");
            let snapshot = tracker.snapshot();
            tracker
                .record_payload(&payload(provenance))
                .expect("interference records");
            assert!(!tracker.safe_since(snapshot), "{provenance}");
        }
    }

    #[test]
    fn payload_rejects_details_outside_the_metadata_contract() {
        let tracker = ActivityTracker::default();
        let detailed = r#"{"backend":"kwin_input_spy_v1","seat":"default","class":"keyboard","provenance":"trusted_physical","monotonic_ms":1,"key":30}"#;
        assert!(tracker.record_payload(detailed).is_err());
    }
}
