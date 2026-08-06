use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Result, bail};
use serde::Deserialize;

pub(crate) const KWIN_INPUT_SPY_BACKEND: &str = "kwin_input_spy_v2";
pub(crate) const LEGACY_KWIN_INPUT_SPY_BACKEND: &str = "kwin_input_spy_v1";
const TARGET_HISTORY_TTL: Duration = Duration::from_secs(60);

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
    #[serde(default)]
    window_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActivityRecord {
    pub interference: bool,
    pub window_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ActivitySnapshot {
    backend_generation: u64,
    interference_generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TargetActivitySnapshot {
    backend_generation: u64,
    target_generation: u64,
    targeted_backend: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TargetInterferenceState {
    pub available: bool,
    pub fresh: bool,
    pub age_ms: Option<u64>,
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
    target_generations: HashMap<String, u64>,
    last_target_interference: HashMap<String, Instant>,
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

    pub(crate) fn record_payload(&self, payload: &str) -> Result<ActivityRecord> {
        let payload: ActivityPayload = serde_json::from_str(payload)?;
        validate_backend(&payload.backend)?;
        if payload.seat != "default" {
            bail!("unsupported activity seat");
        }
        if payload.backend == KWIN_INPUT_SPY_BACKEND {
            let window_id = payload
                .window_id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("target-aware input activity omitted window_id"))?;
            validate_window_id(window_id)?;
        } else if let Some(window_id) = payload.window_id.as_deref() {
            validate_window_id(window_id)?;
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
        let interference = payload.provenance.is_interference();
        if interference {
            state.interference_generation = state.interference_generation.saturating_add(1);
            state.last_interference = Some(Instant::now());
            if let Some(window_id) = payload.window_id.as_ref() {
                let stale_targets = state
                    .last_target_interference
                    .iter()
                    .filter(|(_, last)| last.elapsed() > TARGET_HISTORY_TTL)
                    .map(|(target, _)| target.clone())
                    .collect::<Vec<_>>();
                for target in stale_targets {
                    state.last_target_interference.remove(&target);
                    state.target_generations.remove(&target);
                }
                let generation = state
                    .target_generations
                    .entry(window_id.clone())
                    .or_default();
                *generation = generation.saturating_add(1);
                state
                    .last_target_interference
                    .insert(window_id.clone(), Instant::now());
            }
        }
        Ok(ActivityRecord {
            interference,
            window_id: payload.window_id,
        })
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
        state.backend.as_deref().is_some_and(is_supported_backend)
            && state.backend_generation == snapshot.backend_generation
            && state.interference_generation == snapshot.interference_generation
    }

    pub(crate) fn target_snapshot(&self, window_id: &str) -> TargetActivitySnapshot {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        TargetActivitySnapshot {
            backend_generation: state.backend_generation,
            target_generation: state
                .target_generations
                .get(window_id)
                .copied()
                .unwrap_or_default(),
            targeted_backend: state.backend.as_deref() == Some(KWIN_INPUT_SPY_BACKEND),
        }
    }

    pub(crate) fn target_safe_since(
        &self,
        window_id: &str,
        snapshot: TargetActivitySnapshot,
    ) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        !snapshot.targeted_backend
            || (state.backend.as_deref() == Some(KWIN_INPUT_SPY_BACKEND)
                && state.backend_generation == snapshot.backend_generation
                && state
                    .target_generations
                    .get(window_id)
                    .copied()
                    .unwrap_or_default()
                    == snapshot.target_generation)
    }

    pub(crate) fn target_interference_state(
        &self,
        window_id: &str,
        quiet_for: Duration,
    ) -> TargetInterferenceState {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.backend.as_deref() != Some(KWIN_INPUT_SPY_BACKEND) {
            return TargetInterferenceState {
                available: false,
                fresh: false,
                age_ms: None,
            };
        }
        let Some(last) = state.last_target_interference.get(window_id) else {
            return TargetInterferenceState {
                available: true,
                fresh: false,
                age_ms: None,
            };
        };
        let age = last.elapsed();
        TargetInterferenceState {
            available: true,
            fresh: age <= quiet_for,
            age_ms: Some(age.as_millis().min(u128::from(u64::MAX)) as u64),
        }
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
            trusted: state.backend.as_deref().is_some_and(is_supported_backend),
            last_class: state.last_class.map(ActivityClass::as_str),
            last_provenance: state.last_provenance.map(ActivityProvenance::as_str),
        }
    }
}

fn validate_backend(backend: &str) -> Result<()> {
    if !is_supported_backend(backend) {
        bail!("unsupported input activity backend");
    }
    Ok(())
}

fn is_supported_backend(backend: &str) -> bool {
    matches!(
        backend,
        KWIN_INPUT_SPY_BACKEND | LEGACY_KWIN_INPUT_SPY_BACKEND
    )
}

fn validate_window_id(window_id: &str) -> Result<()> {
    if window_id.is_empty() || window_id.len() > 128 || window_id.chars().any(char::is_control) {
        bail!("invalid input activity window_id");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(provenance: &str) -> String {
        format!(
            r#"{{"backend":"kwin_input_spy_v2","seat":"default","class":"pointer","provenance":"{provenance}","monotonic_ms":42,"window_id":"window-1"}}"#
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

    #[test]
    fn legacy_backend_remains_trusted_but_target_preemption_is_unavailable() {
        let tracker = ActivityTracker::default();
        tracker
            .record_payload(
                r#"{"backend":"kwin_input_spy_v1","seat":"default","class":"keyboard","provenance":"trusted_physical","monotonic_ms":1}"#,
            )
            .expect("legacy activity records");
        assert!(tracker.status().trusted);
        assert!(
            !tracker
                .target_interference_state("window-1", Duration::from_millis(350))
                .available
        );
    }

    #[test]
    fn target_activity_preempts_only_the_matching_window() {
        let tracker = ActivityTracker::default();
        tracker
            .register_backend(KWIN_INPUT_SPY_BACKEND)
            .expect("backend registers");
        let target_snapshot = tracker.target_snapshot("window-1");
        let other_snapshot = tracker.target_snapshot("window-2");
        tracker
            .record_payload(&payload("trusted_physical"))
            .expect("physical activity records");
        let target = tracker.target_interference_state("window-1", Duration::from_millis(350));
        assert!(target.available);
        assert!(target.fresh);
        assert!(!tracker.target_safe_since("window-1", target_snapshot));
        assert!(tracker.target_safe_since("window-2", other_snapshot));
    }

    #[test]
    fn target_aware_backend_requires_a_bounded_window_id() {
        let tracker = ActivityTracker::default();
        assert!(
            tracker
                .record_payload(
                    r#"{"backend":"kwin_input_spy_v2","seat":"default","class":"pointer","provenance":"trusted_physical","monotonic_ms":1}"#,
                )
                .is_err()
        );
    }
}
