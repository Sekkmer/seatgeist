//! Finite native gesture contract. No held state survives a request.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeGestureRequest {
    pub session_id: String,
    pub steps: Vec<GestureStep>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum GestureStep {
    Key {
        code: u16,
        pressed: bool,
    },
    Button {
        button: GestureButton,
        pressed: bool,
    },
    Motion {
        x: f64,
        y: f64,
    },
    RelativeMotion {
        dx: f64,
        dy: f64,
    },
    Scroll {
        axis: GestureAxis,
        delta: f64,
        delta_v120: i32,
        source: GestureScrollSource,
        #[serde(default)]
        inverted: bool,
    },
    Wait {
        duration_ms: u64,
    },
    Await {
        condition: GestureCondition,
        timeout_ms: u64,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GestureButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}
impl GestureButton {
    pub fn code(self) -> u16 {
        match self {
            Self::Left => 272,
            Self::Right => 273,
            Self::Middle => 274,
            Self::Back => 275,
            Self::Forward => 276,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GestureAxis {
    Vertical,
    Horizontal,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GestureScrollSource {
    Wheel,
    Finger,
    Continuous,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GestureCondition {
    BindingsReady,
    PointerLocked,
    ClientUnlocked,
}
impl NativeGestureRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.session_id.trim().is_empty() || self.steps.is_empty() || self.steps.len() > 256 {
            return Err("gesture requires an exact session and 1..256 steps");
        }
        let mut keys = BTreeSet::new();
        let mut buttons = BTreeSet::new();
        let mut budget = 0u64;
        let mut input = false;
        for step in &self.steps {
            match *step {
                GestureStep::Key { code, pressed } => {
                    input = true;
                    if code == 0
                        || code > 0x2ff
                        || (if pressed {
                            !keys.insert(code)
                        } else {
                            !keys.remove(&code)
                        })
                    {
                        return Err("unbalanced or invalid gesture key");
                    }
                    let ctrl = keys.contains(&29) || keys.contains(&97);
                    let alt = keys.contains(&56) || keys.contains(&100);
                    let meta = keys.contains(&125) || keys.contains(&126);
                    if (alt && keys.contains(&62))
                        || (ctrl && (keys.contains(&16) || keys.contains(&17)))
                        || (meta && keys.contains(&16))
                    {
                        return Err("close/quit chord is forbidden in an exact-session gesture");
                    }
                }
                GestureStep::Button { button, pressed } => {
                    input = true;
                    let code = button.code();
                    if if pressed {
                        !buttons.insert(code)
                    } else {
                        !buttons.remove(&code)
                    } {
                        return Err("unbalanced gesture button");
                    }
                }
                GestureStep::Motion { x, y } => {
                    input = true;
                    if !x.is_finite() || !y.is_finite() || x < 0. || y < 0. {
                        return Err("gesture motion requires finite window-local coordinates");
                    }
                }
                GestureStep::RelativeMotion { dx, dy } => {
                    input = true;
                    if !dx.is_finite()
                        || !dy.is_finite()
                        || dx.abs() > 4096.
                        || dy.abs() > 4096.
                        || (dx == 0. && dy == 0.)
                    {
                        return Err("invalid finite gesture relative motion");
                    }
                }
                GestureStep::Scroll {
                    delta,
                    source,
                    delta_v120,
                    ..
                } => {
                    if source != GestureScrollSource::Wheel && delta_v120 != 0 {
                        return Err("v120 scroll deltas require wheel source");
                    }
                    input = true;
                    if !delta.is_finite() || delta.abs() > 1_000_000. {
                        return Err("invalid gesture scroll delta");
                    }
                }
                GestureStep::Wait { duration_ms } => {
                    if duration_ms == 0 {
                        return Err("gesture wait must be positive");
                    }
                    budget = budget.saturating_add(duration_ms);
                }
                GestureStep::Await { timeout_ms, .. } => {
                    if timeout_ms == 0 {
                        return Err("gesture condition timeout must be positive");
                    }
                    budget = budget.saturating_add(timeout_ms);
                }
            }
        }
        if !input || !keys.is_empty() || !buttons.is_empty() || budget > 1500 {
            return Err(
                "gesture must balance all keys/buttons and reserve at most 1500ms of waits",
            );
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeatStateRequest {
    pub session_id: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeatState {
    pub window_id: String,
    pub target_bound: bool,
    #[serde(default)]
    pub keyboard_focused: bool,
    #[serde(default)]
    pub pointer_focused: bool,
    pub keyboard_bound: bool,
    pub pointer_bound: bool,
    pub relative_pointer_bound: bool,
    /// None means an older host did not report the exact seat/client request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pointer_lock_requested: Option<bool>,
    /// Actual committed constraint region contains this seat's local position.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pointer_lock_region_contains: Option<bool>,
    pub pointer_locked: bool,
    pub pointer_position: Option<[f64; 2]>,
    pub cursor: SeatCursorState,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatCursorState {
    /// Monotonic per-lane change counter, not a sampled history or pixel capture.
    pub revision: u64,
    /// unknown, named, surface, or hidden. Surface cursor shapes are not inferred.
    pub kind: String,
    pub name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_host_lock_request_observation_is_unknown_not_false() {
        let old = serde_json::json!({"window_id":"exact", "target_bound":true,
            "keyboard_bound":true,"pointer_bound":true,"relative_pointer_bound":true,
            "pointer_locked":false,"pointer_position":null,
            "cursor":{"revision":0,"kind":"unknown","name":null}});
        let state: SeatState = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(state.pointer_lock_requested, None);
        assert_eq!(state.pointer_lock_region_contains, None);
        let mut new = old;
        new["pointer_lock_requested"] = true.into();
        new["pointer_lock_region_contains"] = false.into();
        let state: SeatState = serde_json::from_value(new).unwrap();
        assert_eq!(state.pointer_lock_requested, Some(true));
        assert_eq!(state.pointer_lock_region_contains, Some(false));
        assert!(!state.pointer_locked);
    }
    fn req(steps: serde_json::Value) -> NativeGestureRequest {
        serde_json::from_value(serde_json::json!({"session_id":"owned","steps":steps})).unwrap()
    }
    #[test]
    fn native_gesture_is_finite_balanced_and_strict() {
        let good = serde_json::json!([{"op":"key","code":17,"pressed":true},{"op":"wait","duration_ms":200},{"op":"key","code":17,"pressed":false}]);
        assert!(req(good.clone()).validate().is_ok());
        for steps in [
            serde_json::json!([]),
            serde_json::json!([{"op":"key","code":17,"pressed":true}]),
            serde_json::json!([{"op":"motion","x":-1,"y":0}]),
            serde_json::json!([{"op":"motion","x":1,"y":0},{"op":"wait","duration_ms":1501}]),
            serde_json::json!([{"op":"relative_motion","dx":0,"dy":0}]),
        ] {
            assert!(req(steps).validate().is_err());
        }
        assert!(serde_json::from_value::<NativeGestureRequest>(serde_json::json!({"session_id":"owned","steps":[{"op":"motion","x":1,"y":2,"extra":true}]})).is_err());
        assert!(
            serde_json::from_value::<NativeGestureRequest>(
                serde_json::json!({"session_id":"owned","steps":good,"hold":true})
            )
            .is_err()
        );
    }
    #[test]
    fn native_gesture_rejects_close_chords_across_steps() {
        let r = req(
            serde_json::json!([{"op":"key","code":29,"pressed":true},{"op":"wait","duration_ms":10},{"op":"key","code":17,"pressed":true},{"op":"key","code":17,"pressed":false},{"op":"key","code":29,"pressed":false}]),
        );
        assert!(r.validate().is_err());
    }
    #[test]
    fn native_gesture_accepts_camera_chord_precision_scroll_and_back() {
        let r = req(
            serde_json::json!([{"op":"key","code":56,"pressed":true},{"op":"relative_motion","dx":8.5,"dy":-3},{"op":"scroll","axis":"horizontal","delta":0.5,"delta_v120":0,"source":"finger"},{"op":"scroll","axis":"horizontal","delta":0,"delta_v120":0,"source":"finger"},{"op":"button","button":"back","pressed":true},{"op":"button","button":"back","pressed":false},{"op":"key","code":56,"pressed":false}]),
        );
        assert!(r.validate().is_ok());
    }
}
