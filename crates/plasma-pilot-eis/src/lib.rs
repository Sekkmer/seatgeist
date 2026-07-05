use libplasma_pilot::{Point, PointerButton};
use thiserror::Error;

pub const LIBEI_SCROLL_UNIT: i32 = 120;
pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;
pub const BTN_MIDDLE: u32 = 0x112;

#[derive(Debug, Clone, PartialEq)]
pub struct EisActionPlan {
    pub required_capabilities: Vec<EisCapability>,
    pub events: Vec<EisEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EisCapability {
    PointerAbsolute,
    Button,
    Scroll,
    Text,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EisEvent {
    StartEmulating { sequence: u32 },
    StopEmulating,
    Frame,
    PointerMotionAbsolute { x: f64, y: f64 },
    Button { button: u32, is_press: bool },
    ScrollDiscrete { x: i32, y: i32 },
    ScrollStop { stop_x: bool, stop_y: bool },
    TextUtf8 { text: String },
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EisPlanError {
    #[error("text must be non-empty")]
    EmptyText,
    #[error("clicks must be 1 or 2")]
    InvalidClickCount,
    #[error("scroll request must include a non-zero delta")]
    EmptyScroll,
}

pub type Result<T> = std::result::Result<T, EisPlanError>;

pub fn plan_text_utf8(sequence: u32, text: impl Into<String>) -> Result<EisActionPlan> {
    let text = text.into();
    if text.is_empty() {
        return Err(EisPlanError::EmptyText);
    }

    Ok(EisActionPlan {
        required_capabilities: vec![EisCapability::Text],
        events: vec![
            EisEvent::StartEmulating { sequence },
            EisEvent::TextUtf8 { text },
            EisEvent::Frame,
            EisEvent::StopEmulating,
        ],
    })
}

pub fn plan_pointer_move_absolute(sequence: u32, point: Point) -> EisActionPlan {
    EisActionPlan {
        required_capabilities: vec![EisCapability::PointerAbsolute],
        events: vec![
            EisEvent::StartEmulating { sequence },
            EisEvent::PointerMotionAbsolute {
                x: point.x,
                y: point.y,
            },
            EisEvent::Frame,
            EisEvent::StopEmulating,
        ],
    }
}

pub fn plan_pointer_click_absolute(
    sequence: u32,
    point: Point,
    button: PointerButton,
    clicks: u8,
) -> Result<EisActionPlan> {
    if clicks == 0 || clicks > 2 {
        return Err(EisPlanError::InvalidClickCount);
    }

    let mut events = vec![
        EisEvent::StartEmulating { sequence },
        EisEvent::PointerMotionAbsolute {
            x: point.x,
            y: point.y,
        },
        EisEvent::Frame,
    ];
    for _ in 0..clicks {
        events.push(EisEvent::Button {
            button: pointer_button_code(button),
            is_press: true,
        });
        events.push(EisEvent::Frame);
        events.push(EisEvent::Button {
            button: pointer_button_code(button),
            is_press: false,
        });
        events.push(EisEvent::Frame);
    }
    events.push(EisEvent::StopEmulating);

    Ok(EisActionPlan {
        required_capabilities: vec![EisCapability::PointerAbsolute, EisCapability::Button],
        events,
    })
}

pub fn plan_pointer_drag_absolute(
    sequence: u32,
    from: Point,
    to: Point,
    button: PointerButton,
) -> EisActionPlan {
    EisActionPlan {
        required_capabilities: vec![EisCapability::PointerAbsolute, EisCapability::Button],
        events: vec![
            EisEvent::StartEmulating { sequence },
            EisEvent::PointerMotionAbsolute {
                x: from.x,
                y: from.y,
            },
            EisEvent::Frame,
            EisEvent::Button {
                button: pointer_button_code(button),
                is_press: true,
            },
            EisEvent::Frame,
            EisEvent::PointerMotionAbsolute { x: to.x, y: to.y },
            EisEvent::Frame,
            EisEvent::Button {
                button: pointer_button_code(button),
                is_press: false,
            },
            EisEvent::Frame,
            EisEvent::StopEmulating,
        ],
    }
}

pub fn plan_pointer_scroll_discrete(
    sequence: u32,
    vertical: i32,
    horizontal: i32,
) -> Result<EisActionPlan> {
    if vertical == 0 && horizontal == 0 {
        return Err(EisPlanError::EmptyScroll);
    }

    Ok(EisActionPlan {
        required_capabilities: vec![EisCapability::Scroll],
        events: vec![
            EisEvent::StartEmulating { sequence },
            EisEvent::ScrollDiscrete {
                x: horizontal * LIBEI_SCROLL_UNIT,
                y: vertical * LIBEI_SCROLL_UNIT,
            },
            EisEvent::Frame,
            EisEvent::ScrollStop {
                stop_x: horizontal != 0,
                stop_y: vertical != 0,
            },
            EisEvent::Frame,
            EisEvent::StopEmulating,
        ],
    })
}

pub fn pointer_button_code(button: PointerButton) -> u32 {
    match button {
        PointerButton::Left => BTN_LEFT,
        PointerButton::Middle => BTN_MIDDLE,
        PointerButton::Right => BTN_RIGHT,
    }
}

#[cfg(test)]
mod tests {
    use libplasma_pilot::CoordinateSpace;

    use super::*;

    fn point(x: f64, y: f64) -> Point {
        Point {
            x,
            y,
            space: CoordinateSpace::PhysicalPixel,
        }
    }

    #[test]
    fn plans_utf8_text_with_transaction_and_frame() {
        let plan = plan_text_utf8(7, "hello").expect("text plan");

        assert_eq!(plan.required_capabilities, vec![EisCapability::Text]);
        assert_eq!(
            plan.events,
            vec![
                EisEvent::StartEmulating { sequence: 7 },
                EisEvent::TextUtf8 {
                    text: "hello".to_string()
                },
                EisEvent::Frame,
                EisEvent::StopEmulating,
            ]
        );
        assert_eq!(plan_text_utf8(1, ""), Err(EisPlanError::EmptyText));
    }

    #[test]
    fn plans_absolute_pointer_move() {
        let plan = plan_pointer_move_absolute(9, point(3840.0, 2160.0));

        assert_eq!(
            plan.required_capabilities,
            vec![EisCapability::PointerAbsolute]
        );
        assert_eq!(
            plan.events,
            vec![
                EisEvent::StartEmulating { sequence: 9 },
                EisEvent::PointerMotionAbsolute {
                    x: 3840.0,
                    y: 2160.0
                },
                EisEvent::Frame,
                EisEvent::StopEmulating,
            ]
        );
    }

    #[test]
    fn plans_clicks_with_linux_button_codes() {
        let plan = plan_pointer_click_absolute(10, point(10.0, 20.0), PointerButton::Right, 2)
            .expect("click plan");

        assert_eq!(
            plan.required_capabilities,
            vec![EisCapability::PointerAbsolute, EisCapability::Button]
        );
        assert_eq!(plan.events[0], EisEvent::StartEmulating { sequence: 10 });
        assert_eq!(
            plan.events[1],
            EisEvent::PointerMotionAbsolute { x: 10.0, y: 20.0 }
        );
        assert_eq!(
            plan.events
                .iter()
                .filter(|event| matches!(
                    event,
                    EisEvent::Button {
                        button: BTN_RIGHT,
                        ..
                    }
                ))
                .count(),
            4
        );
        assert_eq!(plan.events.last(), Some(&EisEvent::StopEmulating));
        assert_eq!(
            plan_pointer_click_absolute(1, point(0.0, 0.0), PointerButton::Left, 0),
            Err(EisPlanError::InvalidClickCount)
        );
    }

    #[test]
    fn plans_drag_with_release_before_stop() {
        let plan =
            plan_pointer_drag_absolute(11, point(1.0, 2.0), point(3.0, 4.0), PointerButton::Left);

        assert_eq!(
            plan.events,
            vec![
                EisEvent::StartEmulating { sequence: 11 },
                EisEvent::PointerMotionAbsolute { x: 1.0, y: 2.0 },
                EisEvent::Frame,
                EisEvent::Button {
                    button: BTN_LEFT,
                    is_press: true
                },
                EisEvent::Frame,
                EisEvent::PointerMotionAbsolute { x: 3.0, y: 4.0 },
                EisEvent::Frame,
                EisEvent::Button {
                    button: BTN_LEFT,
                    is_press: false
                },
                EisEvent::Frame,
                EisEvent::StopEmulating,
            ]
        );
    }

    #[test]
    fn plans_discrete_scroll_in_120_unit_steps() {
        let plan = plan_pointer_scroll_discrete(12, -2, 1).expect("scroll plan");

        assert_eq!(plan.required_capabilities, vec![EisCapability::Scroll]);
        assert_eq!(
            plan.events,
            vec![
                EisEvent::StartEmulating { sequence: 12 },
                EisEvent::ScrollDiscrete { x: 120, y: -240 },
                EisEvent::Frame,
                EisEvent::ScrollStop {
                    stop_x: true,
                    stop_y: true
                },
                EisEvent::Frame,
                EisEvent::StopEmulating,
            ]
        );
        assert_eq!(
            plan_pointer_scroll_discrete(1, 0, 0),
            Err(EisPlanError::EmptyScroll)
        );
    }
}
