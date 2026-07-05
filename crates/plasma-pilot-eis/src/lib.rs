use libplasma_pilot::{Point, PointerButton};
use std::{marker::PhantomData, ptr::NonNull};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EisDeviceKind {
    Virtual,
    Physical,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EisRegion {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
}

impl EisRegion {
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EisDeviceInfo {
    pub id: String,
    pub name: Option<String>,
    pub kind: EisDeviceKind,
    pub resumed: bool,
    pub capabilities: Vec<EisCapability>,
    pub regions: Vec<EisRegion>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EisDeviceSelection {
    pub device_id: String,
    pub device_name: Option<String>,
    pub matched_region: Option<EisRegion>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EisDeviceSelectionError {
    #[error("no resumed EIS device provides the required capabilities")]
    NoCapableResumedDevice,
    #[error("no resumed EIS absolute-pointer device covers every target coordinate")]
    NoRegionForAbsolutePointer,
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

pub fn select_resumed_device_for_plan(
    plan: &EisActionPlan,
    devices: &[EisDeviceInfo],
) -> std::result::Result<EisDeviceSelection, EisDeviceSelectionError> {
    let capable = devices
        .iter()
        .filter(|device| device.resumed && device_supports_plan(device, plan));

    if !plan
        .required_capabilities
        .contains(&EisCapability::PointerAbsolute)
    {
        let device = capable
            .into_iter()
            .next()
            .ok_or(EisDeviceSelectionError::NoCapableResumedDevice)?;
        return Ok(device_selection(device, None));
    }

    let absolute_points = absolute_pointer_points(plan);
    let mut saw_capable = false;
    for device in capable {
        saw_capable = true;
        if let Some(region) = region_covering_points(device, &absolute_points) {
            return Ok(device_selection(device, Some(region.clone())));
        }
    }

    if saw_capable {
        Err(EisDeviceSelectionError::NoRegionForAbsolutePointer)
    } else {
        Err(EisDeviceSelectionError::NoCapableResumedDevice)
    }
}

pub fn device_supports_plan(device: &EisDeviceInfo, plan: &EisActionPlan) -> bool {
    plan.required_capabilities
        .iter()
        .all(|capability| device.capabilities.contains(capability))
}

fn device_selection(
    device: &EisDeviceInfo,
    matched_region: Option<EisRegion>,
) -> EisDeviceSelection {
    EisDeviceSelection {
        device_id: device.id.clone(),
        device_name: device.name.clone(),
        matched_region,
    }
}

fn absolute_pointer_points(plan: &EisActionPlan) -> Vec<(f64, f64)> {
    plan.events
        .iter()
        .filter_map(|event| match event {
            EisEvent::PointerMotionAbsolute { x, y } => Some((*x, *y)),
            _ => None,
        })
        .collect()
}

fn region_covering_points<'a>(
    device: &'a EisDeviceInfo,
    points: &[(f64, f64)],
) -> Option<&'a EisRegion> {
    match device.kind {
        EisDeviceKind::Virtual => device
            .regions
            .iter()
            .find(|region| points.iter().all(|(x, y)| region.contains(*x, *y))),
        EisDeviceKind::Physical => None,
    }
}

pub trait EisEventSink {
    type Error;

    fn start_emulating(&mut self, sequence: u32) -> std::result::Result<(), Self::Error>;
    fn stop_emulating(&mut self) -> std::result::Result<(), Self::Error>;
    fn frame(&mut self) -> std::result::Result<(), Self::Error>;
    fn pointer_motion_absolute(&mut self, x: f64, y: f64) -> std::result::Result<(), Self::Error>;
    fn button(&mut self, button: u32, is_press: bool) -> std::result::Result<(), Self::Error>;
    fn scroll_discrete(&mut self, x: i32, y: i32) -> std::result::Result<(), Self::Error>;
    fn scroll_stop(&mut self, stop_x: bool, stop_y: bool) -> std::result::Result<(), Self::Error>;
    fn text_utf8(&mut self, text: &str) -> std::result::Result<(), Self::Error>;
}

pub fn apply_plan_to_sink<S>(
    plan: &EisActionPlan,
    sink: &mut S,
) -> std::result::Result<(), S::Error>
where
    S: EisEventSink,
{
    for event in &plan.events {
        match event {
            EisEvent::StartEmulating { sequence } => sink.start_emulating(*sequence)?,
            EisEvent::StopEmulating => sink.stop_emulating()?,
            EisEvent::Frame => sink.frame()?,
            EisEvent::PointerMotionAbsolute { x, y } => sink.pointer_motion_absolute(*x, *y)?,
            EisEvent::Button { button, is_press } => sink.button(*button, *is_press)?,
            EisEvent::ScrollDiscrete { x, y } => sink.scroll_discrete(*x, *y)?,
            EisEvent::ScrollStop { stop_x, stop_y } => sink.scroll_stop(*stop_x, *stop_y)?,
            EisEvent::TextUtf8 { text } => sink.text_utf8(text)?,
        }
    }
    Ok(())
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LibeiSinkError {
    #[error("libei text event contains an interior NUL byte")]
    InteriorNulText,
}

#[repr(C)]
pub struct Ei {
    _private: [u8; 0],
}

#[repr(C)]
pub struct EiDevice {
    _private: [u8; 0],
}

#[link(name = "ei")]
unsafe extern "C" {
    fn ei_now(ei: *mut Ei) -> u64;
    fn ei_device_start_emulating(device: *mut EiDevice, sequence: u32);
    fn ei_device_stop_emulating(device: *mut EiDevice);
    fn ei_device_frame(device: *mut EiDevice, time: u64);
    fn ei_device_pointer_motion_absolute(device: *mut EiDevice, x: f64, y: f64);
    fn ei_device_button_button(device: *mut EiDevice, button: u32, is_press: bool);
    fn ei_device_scroll_discrete(device: *mut EiDevice, x: i32, y: i32);
    fn ei_device_scroll_stop(device: *mut EiDevice, stop_x: bool, stop_y: bool);
    fn ei_device_text_utf8_with_length(
        device: *mut EiDevice,
        text: *const libc::c_char,
        length: usize,
    );
}

pub struct LibeiDeviceSink<'a> {
    context: NonNull<Ei>,
    device: NonNull<EiDevice>,
    _marker: PhantomData<&'a mut EiDevice>,
}

impl<'a> LibeiDeviceSink<'a> {
    /// Create a libei sender sink from raw pointers owned by a live libei event loop.
    ///
    /// # Safety
    ///
    /// `context` and `device` must be valid non-null pointers from the same live
    /// sender context. The device must be resumed and expose every capability
    /// required by the plan passed to [`apply_plan_to_sink`]. The caller remains
    /// responsible for the libei connection lifecycle and device refcounts.
    pub unsafe fn from_raw(context: *mut Ei, device: *mut EiDevice) -> Option<Self> {
        Some(Self {
            context: NonNull::new(context)?,
            device: NonNull::new(device)?,
            _marker: PhantomData,
        })
    }

    fn now(&self) -> u64 {
        // SAFETY: `context` is guaranteed valid by `from_raw` for this sink's lifetime.
        unsafe { ei_now(self.context.as_ptr()) }
    }
}

impl EisEventSink for LibeiDeviceSink<'_> {
    type Error = LibeiSinkError;

    fn start_emulating(&mut self, sequence: u32) -> std::result::Result<(), Self::Error> {
        // SAFETY: `device` is guaranteed valid by `from_raw` for this sink's lifetime.
        unsafe { ei_device_start_emulating(self.device.as_ptr(), sequence) };
        Ok(())
    }

    fn stop_emulating(&mut self) -> std::result::Result<(), Self::Error> {
        // SAFETY: `device` is guaranteed valid by `from_raw` for this sink's lifetime.
        unsafe { ei_device_stop_emulating(self.device.as_ptr()) };
        Ok(())
    }

    fn frame(&mut self) -> std::result::Result<(), Self::Error> {
        let now = self.now();
        // SAFETY: `device` is guaranteed valid by `from_raw`; `now` comes from libei.
        unsafe { ei_device_frame(self.device.as_ptr(), now) };
        Ok(())
    }

    fn pointer_motion_absolute(&mut self, x: f64, y: f64) -> std::result::Result<(), Self::Error> {
        // SAFETY: `device` is guaranteed valid by `from_raw` for this sink's lifetime.
        unsafe { ei_device_pointer_motion_absolute(self.device.as_ptr(), x, y) };
        Ok(())
    }

    fn button(&mut self, button: u32, is_press: bool) -> std::result::Result<(), Self::Error> {
        // SAFETY: `device` is guaranteed valid by `from_raw` for this sink's lifetime.
        unsafe { ei_device_button_button(self.device.as_ptr(), button, is_press) };
        Ok(())
    }

    fn scroll_discrete(&mut self, x: i32, y: i32) -> std::result::Result<(), Self::Error> {
        // SAFETY: `device` is guaranteed valid by `from_raw` for this sink's lifetime.
        unsafe { ei_device_scroll_discrete(self.device.as_ptr(), x, y) };
        Ok(())
    }

    fn scroll_stop(&mut self, stop_x: bool, stop_y: bool) -> std::result::Result<(), Self::Error> {
        // SAFETY: `device` is guaranteed valid by `from_raw` for this sink's lifetime.
        unsafe { ei_device_scroll_stop(self.device.as_ptr(), stop_x, stop_y) };
        Ok(())
    }

    fn text_utf8(&mut self, text: &str) -> std::result::Result<(), Self::Error> {
        if text.as_bytes().contains(&0) {
            return Err(LibeiSinkError::InteriorNulText);
        }
        // SAFETY: `device` is valid, and the pointer/length pair is valid for this call.
        unsafe {
            ei_device_text_utf8_with_length(
                self.device.as_ptr(),
                text.as_ptr().cast::<libc::c_char>(),
                text.len(),
            )
        };
        Ok(())
    }
}

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

    #[derive(Debug, Default)]
    struct RecordingSink {
        calls: Vec<String>,
    }

    impl EisEventSink for RecordingSink {
        type Error = std::convert::Infallible;

        fn start_emulating(&mut self, sequence: u32) -> std::result::Result<(), Self::Error> {
            self.calls.push(format!("start:{sequence}"));
            Ok(())
        }

        fn stop_emulating(&mut self) -> std::result::Result<(), Self::Error> {
            self.calls.push("stop".to_string());
            Ok(())
        }

        fn frame(&mut self) -> std::result::Result<(), Self::Error> {
            self.calls.push("frame".to_string());
            Ok(())
        }

        fn pointer_motion_absolute(
            &mut self,
            x: f64,
            y: f64,
        ) -> std::result::Result<(), Self::Error> {
            self.calls.push(format!("abs:{x:.0},{y:.0}"));
            Ok(())
        }

        fn button(&mut self, button: u32, is_press: bool) -> std::result::Result<(), Self::Error> {
            self.calls.push(format!("button:{button}:{is_press}"));
            Ok(())
        }

        fn scroll_discrete(&mut self, x: i32, y: i32) -> std::result::Result<(), Self::Error> {
            self.calls.push(format!("scroll:{x},{y}"));
            Ok(())
        }

        fn scroll_stop(
            &mut self,
            stop_x: bool,
            stop_y: bool,
        ) -> std::result::Result<(), Self::Error> {
            self.calls.push(format!("scroll-stop:{stop_x},{stop_y}"));
            Ok(())
        }

        fn text_utf8(&mut self, text: &str) -> std::result::Result<(), Self::Error> {
            self.calls.push(format!("text:{text}"));
            Ok(())
        }
    }

    #[derive(Debug, Error, PartialEq, Eq)]
    enum FailingSinkError {
        #[error("injected sink failure")]
        Injected,
    }

    #[derive(Debug)]
    struct FailingSink;

    impl EisEventSink for FailingSink {
        type Error = FailingSinkError;

        fn start_emulating(&mut self, _sequence: u32) -> std::result::Result<(), Self::Error> {
            Ok(())
        }

        fn stop_emulating(&mut self) -> std::result::Result<(), Self::Error> {
            Ok(())
        }

        fn frame(&mut self) -> std::result::Result<(), Self::Error> {
            Err(FailingSinkError::Injected)
        }

        fn pointer_motion_absolute(
            &mut self,
            _x: f64,
            _y: f64,
        ) -> std::result::Result<(), Self::Error> {
            Ok(())
        }

        fn button(
            &mut self,
            _button: u32,
            _is_press: bool,
        ) -> std::result::Result<(), Self::Error> {
            Ok(())
        }

        fn scroll_discrete(&mut self, _x: i32, _y: i32) -> std::result::Result<(), Self::Error> {
            Ok(())
        }

        fn scroll_stop(
            &mut self,
            _stop_x: bool,
            _stop_y: bool,
        ) -> std::result::Result<(), Self::Error> {
            Ok(())
        }

        fn text_utf8(&mut self, _text: &str) -> std::result::Result<(), Self::Error> {
            Ok(())
        }
    }

    fn point(x: f64, y: f64) -> Point {
        Point {
            x,
            y,
            space: CoordinateSpace::PhysicalPixel,
        }
    }

    fn region(x: f64, y: f64, width: f64, height: f64) -> EisRegion {
        EisRegion {
            x,
            y,
            width,
            height,
            scale: 1.0,
        }
    }

    fn device(
        id: &str,
        resumed: bool,
        capabilities: Vec<EisCapability>,
        regions: Vec<EisRegion>,
    ) -> EisDeviceInfo {
        EisDeviceInfo {
            id: id.to_string(),
            name: Some(format!("device {id}")),
            kind: EisDeviceKind::Virtual,
            resumed,
            capabilities,
            regions,
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

    #[test]
    fn applies_plan_to_sender_sink_in_order() {
        let plan = plan_pointer_click_absolute(42, point(10.0, 20.0), PointerButton::Left, 1)
            .expect("click plan");
        let mut sink = RecordingSink::default();

        apply_plan_to_sink(&plan, &mut sink).expect("apply plan");

        assert_eq!(
            sink.calls,
            vec![
                "start:42",
                "abs:10,20",
                "frame",
                "button:272:true",
                "frame",
                "button:272:false",
                "frame",
                "stop",
            ]
        );
    }

    #[test]
    fn apply_plan_stops_on_sink_error() {
        let plan = plan_pointer_move_absolute(42, point(10.0, 20.0));
        let mut sink = FailingSink;

        assert_eq!(
            apply_plan_to_sink(&plan, &mut sink),
            Err(FailingSinkError::Injected)
        );
    }

    #[test]
    fn libei_sink_rejects_interior_nul_text_before_ffi_call() {
        let context = NonNull::<Ei>::dangling().as_ptr();
        let device = NonNull::<EiDevice>::dangling().as_ptr();
        // SAFETY: this test only exercises the pre-FFI validation branch and never
        // calls into libei with these sentinel pointers.
        let mut sink = unsafe { LibeiDeviceSink::from_raw(context, device).expect("sink") };

        assert_eq!(
            sink.text_utf8("bad\0text"),
            Err(LibeiSinkError::InteriorNulText)
        );
    }

    #[test]
    fn selects_resumed_text_device_with_required_capability() {
        let plan = plan_text_utf8(1, "hello").expect("text plan");
        let devices = vec![
            device("paused", false, vec![EisCapability::Text], vec![]),
            device("text", true, vec![EisCapability::Text], vec![]),
        ];

        let selection = select_resumed_device_for_plan(&plan, &devices).expect("selection");

        assert_eq!(
            selection,
            EisDeviceSelection {
                device_id: "text".to_string(),
                device_name: Some("device text".to_string()),
                matched_region: None,
            }
        );
    }

    #[test]
    fn rejects_devices_missing_required_capabilities() {
        let plan = plan_pointer_click_absolute(1, point(10.0, 10.0), PointerButton::Left, 1)
            .expect("click plan");
        let devices = vec![device(
            "pointer-only",
            true,
            vec![EisCapability::PointerAbsolute],
            vec![region(0.0, 0.0, 100.0, 100.0)],
        )];

        assert_eq!(
            select_resumed_device_for_plan(&plan, &devices),
            Err(EisDeviceSelectionError::NoCapableResumedDevice)
        );
    }

    #[test]
    fn selects_absolute_pointer_device_with_covering_region() {
        let plan = plan_pointer_click_absolute(1, point(75.0, 80.0), PointerButton::Left, 1)
            .expect("click plan");
        let devices = vec![
            device(
                "wrong-region",
                true,
                vec![EisCapability::PointerAbsolute, EisCapability::Button],
                vec![region(200.0, 200.0, 100.0, 100.0)],
            ),
            device(
                "right-region",
                true,
                vec![EisCapability::PointerAbsolute, EisCapability::Button],
                vec![region(0.0, 0.0, 100.0, 100.0)],
            ),
        ];

        let selection = select_resumed_device_for_plan(&plan, &devices).expect("selection");

        assert_eq!(selection.device_id, "right-region");
        assert_eq!(
            selection.matched_region,
            Some(region(0.0, 0.0, 100.0, 100.0))
        );
    }

    #[test]
    fn rejects_absolute_pointer_points_outside_regions() {
        let plan = plan_pointer_move_absolute(1, point(150.0, 10.0));
        let devices = vec![device(
            "too-small",
            true,
            vec![EisCapability::PointerAbsolute],
            vec![region(0.0, 0.0, 100.0, 100.0)],
        )];

        assert_eq!(
            select_resumed_device_for_plan(&plan, &devices),
            Err(EisDeviceSelectionError::NoRegionForAbsolutePointer)
        );
    }

    #[test]
    fn rejects_drag_crossing_eis_regions_until_split_routing_exists() {
        let plan = plan_pointer_drag_absolute(
            1,
            point(10.0, 10.0),
            point(250.0, 10.0),
            PointerButton::Left,
        );
        let devices = vec![device(
            "two-regions",
            true,
            vec![EisCapability::PointerAbsolute, EisCapability::Button],
            vec![
                region(0.0, 0.0, 100.0, 100.0),
                region(200.0, 0.0, 100.0, 100.0),
            ],
        )];

        assert_eq!(
            select_resumed_device_for_plan(&plan, &devices),
            Err(EisDeviceSelectionError::NoRegionForAbsolutePointer)
        );
    }

    #[test]
    fn rejects_physical_absolute_pointer_device_without_explicit_mapping() {
        let plan = plan_pointer_move_absolute(1, point(10.0, 10.0));
        let mut physical = device(
            "physical",
            true,
            vec![EisCapability::PointerAbsolute],
            vec![region(0.0, 0.0, 100.0, 100.0)],
        );
        physical.kind = EisDeviceKind::Physical;

        assert_eq!(
            select_resumed_device_for_plan(&plan, &[physical]),
            Err(EisDeviceSelectionError::NoRegionForAbsolutePointer)
        );
    }
}
