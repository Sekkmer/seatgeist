//! Changeable action planning for the permanent native seat host.
//! The host consumes only balanced, bounded primitive event batches.
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

pub(super) const HOST_PROTOCOL: u32 = 1;
const MAX_EVENTS: usize = 32768;
const KEY_MAX: u16 = 0x2ff;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(super) enum Event {
    Key {
        code: u16,
        pressed: bool,
    },
    Motion {
        x: f64,
        y: f64,
    },
    Button {
        code: u32,
        pressed: bool,
    },
    Axis {
        axis: &'static str,
        delta: f64,
        delta_v120: i32,
    },
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum Action {
    KeyCombo {
        keycodes: Vec<u16>,
    },
    KeySequence {
        chords: Vec<Vec<u16>>,
    },
    PointerMove {
        x: f64,
        y: f64,
    },
    PointerClick {
        x: f64,
        y: f64,
        button: u32,
        clicks: u8,
    },
    PointerDrag {
        from_x: f64,
        from_y: f64,
        to_x: f64,
        to_y: f64,
        button: u32,
    },
    PointerScroll {
        vertical: i32,
        horizontal: i32,
    },
}

fn chord(events: &mut Vec<Event>, keys: &[u16], maximum: usize) -> Result<()> {
    let unique: std::collections::HashSet<_> = keys.iter().collect();
    if keys.is_empty()
        || keys.len() > maximum
        || unique.len() != keys.len()
        || keys.iter().any(|&key| key == 0 || key > KEY_MAX)
    {
        bail!("invalid agent-seat key chord");
    }
    events.extend(keys.iter().map(|&code| Event::Key {
        code,
        pressed: true,
    }));
    events.extend(keys.iter().rev().map(|&code| Event::Key {
        code,
        pressed: false,
    }));
    Ok(())
}

fn motion(x: f64, y: f64) -> Result<Event> {
    if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 {
        bail!("invalid agent-seat local pointer position");
    }
    Ok(Event::Motion { x, y })
}

fn button(code: u32, pressed: bool) -> Result<Event> {
    if !matches!(code, 0x110..=0x112) {
        bail!("invalid agent-seat pointer button");
    }
    Ok(Event::Button { code, pressed })
}

pub(super) fn plan(value: serde_json::Value) -> Result<Vec<Event>> {
    let mut events = Vec::new();
    match serde_json::from_value::<Action>(value)? {
        Action::KeyCombo { keycodes } => chord(&mut events, &keycodes, 8)?,
        Action::KeySequence { chords } => {
            if chords.is_empty() || chords.len() > 8192 {
                bail!("agent-seat sequence must contain 1..8192 chords");
            }
            for keys in chords {
                chord(&mut events, &keys, 2)?;
            }
        }
        Action::PointerMove { x, y } => events.push(motion(x, y)?),
        Action::PointerClick {
            x,
            y,
            button: code,
            clicks,
        } => {
            if !(1..=2).contains(&clicks) {
                bail!("invalid agent-seat click count");
            }
            events.push(motion(x, y)?);
            for _ in 0..clicks {
                events.push(button(code, true)?);
                events.push(button(code, false)?);
            }
        }
        Action::PointerDrag {
            from_x,
            from_y,
            to_x,
            to_y,
            button: code,
        } => {
            events.extend([
                motion(from_x, from_y)?,
                button(code, true)?,
                motion(to_x, to_y)?,
                button(code, false)?,
            ]);
        }
        Action::PointerScroll {
            vertical,
            horizontal,
        } => {
            for (axis, detents) in [("vertical", vertical), ("horizontal", horizontal)] {
                if detents != 0 {
                    let delta_v120 = detents
                        .checked_mul(-120)
                        .ok_or_else(|| anyhow::anyhow!("agent-seat scroll overflow"))?;
                    // Wayland surface axes increase down/right; the public API
                    // accepts positive up/left wheel detents.
                    let delta = -f64::from(detents) * 15.0;
                    if delta.abs() > 1_000_000.0 {
                        bail!("agent-seat scroll exceeds host bound");
                    }
                    events.push(Event::Axis {
                        axis,
                        delta,
                        delta_v120,
                    });
                }
            }
        }
    }
    if events.is_empty() || events.len() > MAX_EVENTS {
        bail!("agent-seat event batch exceeds host bounds");
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn chords_release_in_reverse_order() {
        assert_eq!(
            plan(json!({"action":"key_combo","keycodes":[29,30]})).unwrap(),
            vec![
                Event::Key {
                    code: 29,
                    pressed: true
                },
                Event::Key {
                    code: 30,
                    pressed: true
                },
                Event::Key {
                    code: 30,
                    pressed: false
                },
                Event::Key {
                    code: 29,
                    pressed: false
                },
            ]
        );
    }

    #[test]
    fn pointer_gestures_are_planned_in_daemon() {
        let click =
            plan(json!({"action":"pointer_click","x":10,"y":20,"button":272,"clicks":2})).unwrap();
        assert_eq!(click.len(), 5);
        assert_eq!(click[0], Event::Motion { x: 10.0, y: 20.0 });
        assert_eq!(
            click[4],
            Event::Button {
                code: 272,
                pressed: false
            }
        );
        let drag = plan(
            json!({"action":"pointer_drag","from_x":1,"from_y":2,"to_x":3,"to_y":4,"button":272}),
        )
        .unwrap();
        assert_eq!(
            drag,
            vec![
                Event::Motion { x: 1.0, y: 2.0 },
                Event::Button {
                    code: 272,
                    pressed: true
                },
                Event::Motion { x: 3.0, y: 4.0 },
                Event::Button {
                    code: 272,
                    pressed: false
                }
            ]
        );
        assert_eq!(
            plan(json!({"action":"pointer_scroll","vertical":-2,"horizontal":1})).unwrap(),
            vec![
                Event::Axis {
                    axis: "vertical",
                    delta: 30.0,
                    delta_v120: 240
                },
                Event::Axis {
                    axis: "horizontal",
                    delta: -15.0,
                    delta_v120: -120
                },
            ]
        );
    }

    #[test]
    fn advertised_scroll_directions_map_to_wayland_surface_axes() {
        // The public API promises positive up/left, while Wayland axes grow
        // down/right. Check both signs and single-axis requests independently.
        for (axis, vertical, horizontal) in [
            ("vertical", 1, 0),
            ("vertical", -1, 0),
            ("horizontal", 0, 1),
            ("horizontal", 0, -1),
        ] {
            let detents = if vertical != 0 { vertical } else { horizontal };
            assert_eq!(
                plan(
                    json!({"action":"pointer_scroll","vertical":vertical,"horizontal":horizontal})
                )
                .unwrap(),
                vec![Event::Axis {
                    axis,
                    delta: -f64::from(detents) * 15.0,
                    delta_v120: -detents * 120,
                }]
            );
        }
    }

    #[test]
    fn malformed_actions_never_produce_partial_batches() {
        for value in [
            json!({"action":"key_combo","keycodes":[29,29]}),
            json!({"action":"key_sequence","chords":[[30],[0]]}),
            json!({"action":"pointer_drag","from_x":1,"from_y":2,"to_x":-1,"to_y":4,"button":272}),
            json!({"action":"pointer_scroll","vertical":2147483647,"horizontal":0}),
            json!({"action":"pointer_scroll","vertical":-2147483648_i64,"horizontal":0}),
            json!({"action":"pointer_scroll","vertical":0,"horizontal":0}),
            json!({"action":"pointer_click","x":1,"y":2,"button":272,"clicks":0}),
        ] {
            assert!(plan(value).is_err());
        }
    }
}
