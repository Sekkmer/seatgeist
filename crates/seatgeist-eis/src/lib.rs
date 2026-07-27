use libseatgeist::{Point, PointerButton};
use std::{
    ffi::{CStr, CString},
    marker::PhantomData,
    os::fd::{IntoRawFd, OwnedFd, RawFd},
    ptr::{self, NonNull},
};
use thiserror::Error;

pub const LIBEI_SCROLL_UNIT: i32 = 120;
pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;
pub const BTN_MIDDLE: u32 = 0x112;
pub const EI_CAP_POINTER_ABSOLUTE: u32 = 1 << 1;
pub const EI_CAP_KEYBOARD: u32 = 1 << 2;
pub const EI_CAP_BUTTON: u32 = 1 << 5;
pub const EI_CAP_SCROLL: u32 = 1 << 4;
pub const EI_CAP_TEXT: u32 = 1 << 6;
pub const EI_DEVICE_TYPE_VIRTUAL: u32 = 1;
pub const EI_DEVICE_TYPE_PHYSICAL: u32 = 2;
pub const XKB_EVDEV_OFFSET: u32 = 8;

#[derive(Debug, Clone, PartialEq)]
pub struct EisActionPlan {
    pub required_capabilities: Vec<EisCapability>,
    pub events: Vec<EisEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EisCapability {
    PointerAbsolute,
    Keyboard,
    Button,
    Scroll,
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EisDeviceKind {
    Virtual,
    Physical,
    Unknown(u32),
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

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum EisDeviceSelectionError {
    #[error("no resumed EIS device provides the required capabilities")]
    NoCapableResumedDevice,
    #[error("no resumed EIS absolute-pointer device covers every target coordinate")]
    NoRegionForAbsolutePointer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibeiEventType {
    Connect,
    Disconnect,
    SeatAdded,
    SeatRemoved,
    DeviceAdded,
    DeviceRemoved,
    DevicePaused,
    DeviceResumed,
    Other(u32),
}

impl LibeiEventType {
    pub fn from_raw(value: u32) -> Self {
        match value {
            1 => Self::Connect,
            2 => Self::Disconnect,
            3 => Self::SeatAdded,
            4 => Self::SeatRemoved,
            5 => Self::DeviceAdded,
            6 => Self::DeviceRemoved,
            7 => Self::DevicePaused,
            8 => Self::DeviceResumed,
            other => Self::Other(other),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LibeiEventSnapshot {
    Connect,
    Disconnect,
    SeatAdded {
        capabilities: Vec<EisCapability>,
        bound_capabilities: Vec<EisCapability>,
    },
    SeatRemoved,
    DeviceAdded(EisDeviceInfo),
    DeviceRemoved {
        device_id: String,
    },
    DevicePaused {
        device_id: String,
    },
    DeviceResumed(EisDeviceInfo),
    Other {
        event_type: u32,
    },
}

pub trait EisEventSource {
    fn event_fd(&self) -> RawFd;
    fn dispatch_pending(&mut self) -> Vec<LibeiEventSnapshot>;
    fn dispatch_pending_for_plan(&mut self, plan: &EisActionPlan) -> Vec<LibeiEventSnapshot>;
}

pub trait EisSelectedDeviceExecutor {
    type Error;

    fn apply_plan_to_selected_device(
        &mut self,
        selection: &EisDeviceSelection,
        plan: &EisActionPlan,
    ) -> std::result::Result<(), Self::Error>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct EisPlanReadiness {
    pub snapshots: Vec<LibeiEventSnapshot>,
    pub selection: std::result::Result<EisDeviceSelection, EisDeviceSelectionError>,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum EisExecutionReadinessError {
    #[error("EIS session is not connected")]
    NotConnected,
    #[error("EIS session has not bound required capabilities: {missing:?}")]
    MissingBoundCapabilities { missing: Vec<EisCapability> },
    #[error(transparent)]
    DeviceSelection(#[from] EisDeviceSelectionError),
}

#[derive(Debug, Clone, PartialEq)]
pub struct EisExecutionReadiness {
    pub snapshots: Vec<LibeiEventSnapshot>,
    pub selection: std::result::Result<EisDeviceSelection, EisExecutionReadinessError>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EisExecutedPlan {
    pub snapshots: Vec<LibeiEventSnapshot>,
    pub selection: EisDeviceSelection,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum EisPlanExecutionError<E> {
    #[error(transparent)]
    Readiness(#[from] EisExecutionReadinessError),
    #[error("EIS selected-device executor failed: {0}")]
    Executor(E),
}

pub struct EisSessionRuntime<S = LibeiSenderContext> {
    source: S,
    state: EisRuntimeState,
}

impl EisSessionRuntime<LibeiSenderContext> {
    pub fn from_owned_fd(
        fd: OwnedFd,
        client_name: &str,
    ) -> std::result::Result<Self, LibeiConnectionError> {
        Ok(Self::new(LibeiSenderContext::from_owned_fd(
            fd,
            client_name,
        )?))
    }
}

impl<S: EisEventSource> EisSessionRuntime<S> {
    pub fn new(source: S) -> Self {
        Self {
            source,
            state: EisRuntimeState::new(),
        }
    }

    pub fn event_fd(&self) -> RawFd {
        self.source.event_fd()
    }

    pub fn state(&self) -> &EisRuntimeState {
        &self.state
    }

    pub fn source(&self) -> &S {
        &self.source
    }

    pub fn source_mut(&mut self) -> &mut S {
        &mut self.source
    }

    pub fn dispatch_pending(&mut self) -> Vec<LibeiEventSnapshot> {
        let snapshots = self.source.dispatch_pending();
        self.state.apply_snapshots(snapshots.iter());
        snapshots
    }

    pub fn refresh_for_plan(&mut self, plan: &EisActionPlan) -> EisPlanReadiness {
        let snapshots = self.source.dispatch_pending_for_plan(plan);
        self.state.apply_snapshots(snapshots.iter());
        let selection = self.state.select_device_for_plan(plan);
        EisPlanReadiness {
            snapshots,
            selection,
        }
    }

    pub fn refresh_execution_readiness(&mut self, plan: &EisActionPlan) -> EisExecutionReadiness {
        let snapshots = self.source.dispatch_pending_for_plan(plan);
        self.state.apply_snapshots(snapshots.iter());
        let selection = self.state.execution_readiness_for_plan(plan);
        EisExecutionReadiness {
            snapshots,
            selection,
        }
    }
}

impl<S> EisSessionRuntime<S>
where
    S: EisEventSource + EisSelectedDeviceExecutor,
{
    pub fn execute_ready_plan(
        &mut self,
        plan: &EisActionPlan,
    ) -> std::result::Result<EisExecutedPlan, EisPlanExecutionError<S::Error>> {
        let snapshots = self.source.dispatch_pending_for_plan(plan);
        self.state.apply_snapshots(snapshots.iter());
        let selection = self
            .state
            .execution_readiness_for_plan(plan)
            .map_err(EisPlanExecutionError::Readiness)?;
        self.source
            .apply_plan_to_selected_device(&selection, plan)
            .map_err(EisPlanExecutionError::Executor)?;
        Ok(EisExecutedPlan {
            snapshots,
            selection,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct EisRuntimeState {
    connected: bool,
    seat_capabilities: Vec<EisCapability>,
    bound_capabilities: Vec<EisCapability>,
    devices: Vec<EisDeviceInfo>,
}

impl EisRuntimeState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn connected(&self) -> bool {
        self.connected
    }

    pub fn seat_capabilities(&self) -> &[EisCapability] {
        &self.seat_capabilities
    }

    pub fn bound_capabilities(&self) -> &[EisCapability] {
        &self.bound_capabilities
    }

    pub fn devices(&self) -> &[EisDeviceInfo] {
        &self.devices
    }

    pub fn apply_snapshots<'a>(
        &mut self,
        snapshots: impl IntoIterator<Item = &'a LibeiEventSnapshot>,
    ) {
        for snapshot in snapshots {
            self.apply_snapshot(snapshot);
        }
    }

    pub fn apply_snapshot(&mut self, snapshot: &LibeiEventSnapshot) {
        match snapshot {
            LibeiEventSnapshot::Connect => self.connected = true,
            LibeiEventSnapshot::Disconnect => self.clear_connection(),
            LibeiEventSnapshot::SeatAdded {
                capabilities,
                bound_capabilities,
            } => {
                self.seat_capabilities = capabilities.clone();
                self.bound_capabilities = bound_capabilities.clone();
            }
            LibeiEventSnapshot::SeatRemoved => {
                self.seat_capabilities.clear();
                self.bound_capabilities.clear();
                self.devices.clear();
            }
            LibeiEventSnapshot::DeviceAdded(device) | LibeiEventSnapshot::DeviceResumed(device) => {
                self.upsert_device(device.clone());
            }
            LibeiEventSnapshot::DevicePaused { device_id } => {
                if let Some(device) = self
                    .devices
                    .iter_mut()
                    .find(|device| device.id == *device_id)
                {
                    device.resumed = false;
                }
            }
            LibeiEventSnapshot::DeviceRemoved { device_id } => {
                self.devices.retain(|device| device.id != *device_id);
            }
            LibeiEventSnapshot::Other { .. } => {}
        }
    }

    pub fn select_device_for_plan(
        &self,
        plan: &EisActionPlan,
    ) -> std::result::Result<EisDeviceSelection, EisDeviceSelectionError> {
        select_resumed_device_for_plan(plan, &self.devices)
    }

    pub fn execution_readiness_for_plan(
        &self,
        plan: &EisActionPlan,
    ) -> std::result::Result<EisDeviceSelection, EisExecutionReadinessError> {
        if !self.connected {
            return Err(EisExecutionReadinessError::NotConnected);
        }

        let missing = unique_capabilities(&plan.required_capabilities)
            .into_iter()
            .filter(|capability| !self.bound_capabilities.contains(capability))
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(EisExecutionReadinessError::MissingBoundCapabilities { missing });
        }

        self.select_device_for_plan(plan)
            .map_err(EisExecutionReadinessError::DeviceSelection)
    }

    fn clear_connection(&mut self) {
        self.connected = false;
        self.seat_capabilities.clear();
        self.bound_capabilities.clear();
        self.devices.clear();
    }

    fn upsert_device(&mut self, device: EisDeviceInfo) {
        if let Some(existing) = self
            .devices
            .iter_mut()
            .find(|existing| existing.id == device.id)
        {
            *existing = device;
            return;
        }
        self.devices.push(device);
    }
}

#[derive(Debug, Error)]
pub enum LibeiConnectionError {
    #[error("libei client name contains an interior NUL byte")]
    InteriorNulClientName,
    #[error("libei failed to create a sender context")]
    CreateSenderContext,
    #[error("libei failed to set up fd backend: errno {errno}")]
    SetupBackendFd { errno: i32 },
}

#[derive(Debug, Clone, PartialEq)]
pub enum EisEvent {
    StartEmulating { sequence: u32 },
    StopEmulating,
    Frame,
    PointerMotionAbsolute { x: f64, y: f64 },
    KeyboardKey { keycode: u32, is_press: bool },
    TextKeysym { keysym: u32, is_press: bool },
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
    #[error("key combo must contain at least one key")]
    EmptyKeyCombo,
    #[error("keysym text must contain at least one keysym")]
    EmptyKeysymText,
    #[error("character {character:?} does not map to an XKB keysym")]
    UnmappedKeysym { character: char },
    #[error("XKB keymap name field {field} contains an interior NUL byte")]
    InvalidXkbKeymapName { field: &'static str },
    #[error("xkbcommon failed to create a context")]
    CreateXkbContext,
    #[error("xkbcommon failed to create a keymap")]
    CreateXkbKeymap,
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
        EisDeviceKind::Physical | EisDeviceKind::Unknown(_) => None,
    }
}

pub fn capability_to_libei(capability: EisCapability) -> u32 {
    match capability {
        EisCapability::PointerAbsolute => EI_CAP_POINTER_ABSOLUTE,
        EisCapability::Keyboard => EI_CAP_KEYBOARD,
        EisCapability::Button => EI_CAP_BUTTON,
        EisCapability::Scroll => EI_CAP_SCROLL,
        EisCapability::Text => EI_CAP_TEXT,
    }
}

pub fn capabilities_from_libei_bits(bits: u32) -> Vec<EisCapability> {
    [
        EisCapability::PointerAbsolute,
        EisCapability::Keyboard,
        EisCapability::Button,
        EisCapability::Scroll,
        EisCapability::Text,
    ]
    .into_iter()
    .filter(|capability| bits & capability_to_libei(*capability) != 0)
    .collect()
}

pub trait EisEventSink {
    type Error;

    fn start_emulating(&mut self, sequence: u32) -> std::result::Result<(), Self::Error>;
    fn stop_emulating(&mut self) -> std::result::Result<(), Self::Error>;
    fn frame(&mut self) -> std::result::Result<(), Self::Error>;
    fn pointer_motion_absolute(&mut self, x: f64, y: f64) -> std::result::Result<(), Self::Error>;
    fn keyboard_key(
        &mut self,
        keycode: u32,
        is_press: bool,
    ) -> std::result::Result<(), Self::Error>;
    fn text_keysym(&mut self, keysym: u32, is_press: bool) -> std::result::Result<(), Self::Error>;
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
            EisEvent::KeyboardKey { keycode, is_press } => {
                sink.keyboard_key(*keycode, *is_press)?
            }
            EisEvent::TextKeysym { keysym, is_press } => sink.text_keysym(*keysym, *is_press)?,
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

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LibeiExecutionError {
    #[error("selected EIS device is not retained by live libei context: {device_id}")]
    DeviceNotRetained { device_id: String },
    #[error(transparent)]
    Sink(#[from] LibeiSinkError),
}

#[repr(C)]
pub struct Ei {
    _private: [u8; 0],
}

#[repr(C)]
pub struct EiDevice {
    _private: [u8; 0],
}

#[repr(C)]
pub struct EiSeat {
    _private: [u8; 0],
}

#[repr(C)]
pub struct EiEvent {
    _private: [u8; 0],
}

#[repr(C)]
pub struct EiRegion {
    _private: [u8; 0],
}

#[link(name = "ei")]
unsafe extern "C" {
    fn ei_new_sender(user_data: *mut libc::c_void) -> *mut Ei;
    fn ei_unref(ei: *mut Ei) -> *mut Ei;
    fn ei_configure_name(ei: *mut Ei, name: *const libc::c_char);
    fn ei_setup_backend_fd(ei: *mut Ei, fd: libc::c_int) -> libc::c_int;
    fn ei_get_fd(ei: *mut Ei) -> libc::c_int;
    fn ei_dispatch(ei: *mut Ei);
    fn ei_get_event(ei: *mut Ei) -> *mut EiEvent;
    fn ei_now(ei: *mut Ei) -> u64;
    fn ei_event_unref(event: *mut EiEvent) -> *mut EiEvent;
    fn ei_event_get_type(event: *mut EiEvent) -> libc::c_uint;
    fn ei_event_get_device(event: *mut EiEvent) -> *mut EiDevice;
    fn ei_event_get_seat(event: *mut EiEvent) -> *mut EiSeat;
    fn ei_seat_has_capability(seat: *mut EiSeat, capability: libc::c_uint) -> bool;
    fn ei_seat_bind_capabilities(seat: *mut EiSeat, ...);
    fn ei_device_ref(device: *mut EiDevice) -> *mut EiDevice;
    fn ei_device_unref(device: *mut EiDevice) -> *mut EiDevice;
    fn ei_device_get_name(device: *mut EiDevice) -> *const libc::c_char;
    fn ei_device_get_type(device: *mut EiDevice) -> libc::c_uint;
    fn ei_device_has_capability(device: *mut EiDevice, capability: libc::c_uint) -> bool;
    fn ei_device_get_region(device: *mut EiDevice, index: usize) -> *mut EiRegion;
    fn ei_region_get_x(region: *mut EiRegion) -> u32;
    fn ei_region_get_y(region: *mut EiRegion) -> u32;
    fn ei_region_get_width(region: *mut EiRegion) -> u32;
    fn ei_region_get_height(region: *mut EiRegion) -> u32;
    fn ei_region_get_physical_scale(region: *mut EiRegion) -> f64;
    fn ei_device_start_emulating(device: *mut EiDevice, sequence: u32);
    fn ei_device_stop_emulating(device: *mut EiDevice);
    fn ei_device_frame(device: *mut EiDevice, time: u64);
    fn ei_device_pointer_motion_absolute(device: *mut EiDevice, x: f64, y: f64);
    fn ei_device_keyboard_key(device: *mut EiDevice, keycode: u32, is_press: bool);
    fn ei_device_text_keysym(device: *mut EiDevice, keysym: u32, is_press: bool);
    fn ei_device_button_button(device: *mut EiDevice, button: u32, is_press: bool);
    fn ei_device_scroll_discrete(device: *mut EiDevice, x: i32, y: i32);
    fn ei_device_scroll_stop(device: *mut EiDevice, stop_x: bool, stop_y: bool);
    fn ei_device_text_utf8_with_length(
        device: *mut EiDevice,
        text: *const libc::c_char,
        length: usize,
    );
}

#[link(name = "xkbcommon")]
unsafe extern "C" {
    fn xkb_context_new(flags: libc::c_uint) -> *mut XkbContext;
    fn xkb_context_unref(context: *mut XkbContext);
    fn xkb_keymap_new_from_names(
        context: *mut XkbContext,
        names: *const XkbRuleNames,
        flags: libc::c_uint,
    ) -> *mut XkbKeymapRaw;
    fn xkb_keymap_unref(keymap: *mut XkbKeymapRaw);
    fn xkb_keymap_min_keycode(keymap: *mut XkbKeymapRaw) -> u32;
    fn xkb_keymap_max_keycode(keymap: *mut XkbKeymapRaw) -> u32;
    fn xkb_keymap_key_get_syms_by_level(
        keymap: *mut XkbKeymapRaw,
        key: u32,
        layout: u32,
        level: u32,
        syms_out: *mut *const u32,
    ) -> libc::c_int;
    fn xkb_utf32_to_keysym(ucs: u32) -> u32;
}

#[repr(C)]
pub struct XkbContext {
    _private: [u8; 0],
}

#[repr(C)]
pub struct XkbKeymapRaw {
    _private: [u8; 0],
}

#[repr(C)]
struct XkbRuleNames {
    rules: *const libc::c_char,
    model: *const libc::c_char,
    layout: *const libc::c_char,
    variant: *const libc::c_char,
    options: *const libc::c_char,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct XkbKeymapNames<'a> {
    pub rules: Option<&'a str>,
    pub model: Option<&'a str>,
    pub layout: Option<&'a str>,
    pub variant: Option<&'a str>,
    pub options: Option<&'a str>,
}

impl<'a> XkbKeymapNames<'a> {
    pub fn us_pc105() -> Self {
        Self {
            rules: None,
            model: Some("pc105"),
            layout: Some("us"),
            variant: None,
            options: Some(""),
        }
    }
}

pub struct XkbKeymap {
    keymap: NonNull<XkbKeymapRaw>,
    context: NonNull<XkbContext>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XkbKeyStroke {
    pub evdev_keycode: u16,
    pub shift: bool,
}

impl XkbKeymap {
    pub fn new_from_names(names: XkbKeymapNames<'_>) -> Result<Self> {
        let rules = optional_cstring("rules", names.rules)?;
        let model = optional_cstring("model", names.model)?;
        let layout = optional_cstring("layout", names.layout)?;
        let variant = optional_cstring("variant", names.variant)?;
        let options = optional_cstring("options", names.options)?;
        let names = XkbRuleNames {
            rules: optional_cstring_ptr(&rules),
            model: optional_cstring_ptr(&model),
            layout: optional_cstring_ptr(&layout),
            variant: optional_cstring_ptr(&variant),
            options: optional_cstring_ptr(&options),
        };

        // SAFETY: flags value 0 is the documented "no flags" value.
        let context = unsafe { xkb_context_new(0) };
        let context = NonNull::new(context).ok_or(EisPlanError::CreateXkbContext)?;

        // SAFETY: `context` is valid, and every pointer in `names` is either NULL or points to
        // a NUL-terminated CString that lives until the call returns.
        let keymap = unsafe { xkb_keymap_new_from_names(context.as_ptr(), &names, 0) };
        let Some(keymap) = NonNull::new(keymap) else {
            // SAFETY: `context` was returned by xkb_context_new and has not been unref'd yet.
            unsafe { xkb_context_unref(context.as_ptr()) };
            return Err(EisPlanError::CreateXkbKeymap);
        };

        Ok(Self { keymap, context })
    }

    pub fn evdev_keycode_for_keysym_level0(&self, keysym: u32) -> Option<u16> {
        // SAFETY: `self.keymap` is a valid keymap for the lifetime of this wrapper.
        let min = unsafe { xkb_keymap_min_keycode(self.keymap.as_ptr()) };
        // SAFETY: `self.keymap` is a valid keymap for the lifetime of this wrapper.
        let max = unsafe { xkb_keymap_max_keycode(self.keymap.as_ptr()) };
        for xkb_keycode in min..=max {
            if self.keycode_level0_contains_keysym(xkb_keycode, keysym)
                && let Some(evdev_keycode) = xkb_keycode_to_evdev(xkb_keycode)
            {
                return Some(evdev_keycode);
            }
        }
        None
    }

    pub fn keystroke_for_keysym(&self, keysym: u32) -> Option<XkbKeyStroke> {
        // Level 0 is unshifted and level 1 is the conventional Shift level for
        // the simple keyboard symbols supported by the agent-seat slice.
        for (level, shift) in [(0, false), (1, true)] {
            let min = unsafe { xkb_keymap_min_keycode(self.keymap.as_ptr()) };
            let max = unsafe { xkb_keymap_max_keycode(self.keymap.as_ptr()) };
            for xkb_keycode in min..=max {
                if self.keycode_level_contains_keysym(xkb_keycode, level, keysym)
                    && let Some(evdev_keycode) = xkb_keycode_to_evdev(xkb_keycode)
                {
                    return Some(XkbKeyStroke {
                        evdev_keycode,
                        shift,
                    });
                }
            }
        }
        None
    }

    fn keycode_level0_contains_keysym(&self, keycode: u32, keysym: u32) -> bool {
        self.keycode_level_contains_keysym(keycode, 0, keysym)
    }

    fn keycode_level_contains_keysym(&self, keycode: u32, level: u32, keysym: u32) -> bool {
        let mut syms = ptr::null();
        // SAFETY: `self.keymap` is valid; `syms` is an out-pointer owned by xkbcommon for this
        // keymap, and we only read `count` elements before the next xkbcommon call.
        let count = unsafe {
            xkb_keymap_key_get_syms_by_level(self.keymap.as_ptr(), keycode, 0, level, &mut syms)
        };
        if count <= 0 || syms.is_null() {
            return false;
        }
        // SAFETY: xkbcommon returned `count` symbols at `syms` for this keymap lookup.
        let syms = unsafe { std::slice::from_raw_parts(syms, count as usize) };
        syms.contains(&keysym)
    }
}

impl Drop for XkbKeymap {
    fn drop(&mut self) {
        // SAFETY: both pointers are owned by this wrapper and unref'd exactly once.
        unsafe {
            xkb_keymap_unref(self.keymap.as_ptr());
            xkb_context_unref(self.context.as_ptr());
        }
    }
}

fn optional_cstring(field: &'static str, value: Option<&str>) -> Result<Option<CString>> {
    value
        .map(|value| CString::new(value).map_err(|_| EisPlanError::InvalidXkbKeymapName { field }))
        .transpose()
}

fn optional_cstring_ptr(value: &Option<CString>) -> *const libc::c_char {
    value.as_ref().map_or(ptr::null(), |value| value.as_ptr())
}

pub fn xkb_keycode_to_evdev(xkb_keycode: u32) -> Option<u16> {
    let evdev_keycode = xkb_keycode.checked_sub(XKB_EVDEV_OFFSET)?;
    u16::try_from(evdev_keycode).ok()
}

struct RetainedLibeiDevice {
    id: String,
    device: NonNull<EiDevice>,
}

impl RetainedLibeiDevice {
    unsafe fn from_event_device(device: *mut EiDevice) -> Option<Self> {
        let device = NonNull::new(device)?;
        // SAFETY: `device` is a non-null borrowed pointer from the current libei event.
        let retained = unsafe { ei_device_ref(device.as_ptr()) };
        let retained = NonNull::new(retained)?;
        Some(Self {
            id: libei_device_id(device.as_ptr()),
            device: retained,
        })
    }
}

impl Drop for RetainedLibeiDevice {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns one ref obtained through `ei_device_ref`.
        unsafe { ei_device_unref(self.device.as_ptr()) };
    }
}

pub struct LibeiSenderContext {
    context: NonNull<Ei>,
    retained_devices: Vec<RetainedLibeiDevice>,
}

// SAFETY: the wrapper owns the libei sender context and retained device refs,
// and all public operations require `&mut self` for dispatch or event
// emission. Moving the wrapper between daemon worker threads is acceptable as
// long as callers continue to serialize access, which the daemon session store
// does with a mutex. This does not make concurrent libei access safe.
unsafe impl Send for LibeiSenderContext {}

impl LibeiSenderContext {
    pub fn from_owned_fd(
        fd: OwnedFd,
        client_name: &str,
    ) -> std::result::Result<Self, LibeiConnectionError> {
        let client_name =
            CString::new(client_name).map_err(|_| LibeiConnectionError::InteriorNulClientName)?;
        // SAFETY: passing a null user-data pointer is permitted by libei.
        let context = unsafe { ei_new_sender(ptr::null_mut()) };
        let context = NonNull::new(context).ok_or(LibeiConnectionError::CreateSenderContext)?;

        // SAFETY: `context` is a new sender context, and `client_name` is NUL-terminated.
        unsafe { ei_configure_name(context.as_ptr(), client_name.as_ptr()) };
        let raw_fd = fd.into_raw_fd();
        // SAFETY: `context` is valid; libei takes ownership of `raw_fd`.
        let setup_result = unsafe { ei_setup_backend_fd(context.as_ptr(), raw_fd) };
        if setup_result < 0 {
            // SAFETY: `context` was created by libei and must be released on setup failure.
            unsafe { ei_unref(context.as_ptr()) };
            return Err(LibeiConnectionError::SetupBackendFd {
                errno: -setup_result,
            });
        }

        Ok(Self {
            context,
            retained_devices: Vec::new(),
        })
    }

    pub fn event_fd(&self) -> RawFd {
        // SAFETY: `context` is valid for this wrapper's lifetime.
        unsafe { ei_get_fd(self.context.as_ptr()) }
    }

    pub fn dispatch_pending(&mut self) -> Vec<LibeiEventSnapshot> {
        self.dispatch_pending_with_bindings(&[])
    }

    pub fn dispatch_pending_for_plan(&mut self, plan: &EisActionPlan) -> Vec<LibeiEventSnapshot> {
        self.dispatch_pending_with_bindings(&plan.required_capabilities)
    }

    pub fn dispatch_pending_with_bindings(
        &mut self,
        requested_capabilities: &[EisCapability],
    ) -> Vec<LibeiEventSnapshot> {
        // SAFETY: `context` is valid for this wrapper's lifetime.
        unsafe { ei_dispatch(self.context.as_ptr()) };

        let mut snapshots = Vec::new();
        loop {
            // SAFETY: `context` is valid; NULL means no pending events.
            let event = unsafe { ei_get_event(self.context.as_ptr()) };
            let Some(event) = NonNull::new(event) else {
                break;
            };

            let snapshot = libei_event_snapshot(event.as_ptr(), requested_capabilities);
            self.update_retained_devices(event.as_ptr(), &snapshot);
            snapshots.push(snapshot);
            // SAFETY: each event returned by ei_get_event must be unref'd once.
            unsafe { ei_event_unref(event.as_ptr()) };
        }
        snapshots
    }

    fn update_retained_devices(&mut self, event: *mut EiEvent, snapshot: &LibeiEventSnapshot) {
        match snapshot {
            LibeiEventSnapshot::DeviceResumed(_) => {
                // SAFETY: libei permits retrieving the borrowed device for device events.
                let device = unsafe { ei_event_get_device(event) };
                // SAFETY: the event is still alive for this call, so the borrowed device can be ref'd.
                if let Some(retained) = unsafe { RetainedLibeiDevice::from_event_device(device) } {
                    self.retained_devices
                        .retain(|existing| existing.id != retained.id);
                    self.retained_devices.push(retained);
                }
            }
            LibeiEventSnapshot::DevicePaused { device_id }
            | LibeiEventSnapshot::DeviceRemoved { device_id } => {
                self.retained_devices
                    .retain(|retained| retained.id != device_id.as_str());
            }
            LibeiEventSnapshot::Disconnect | LibeiEventSnapshot::SeatRemoved => {
                self.retained_devices.clear();
            }
            LibeiEventSnapshot::Connect
            | LibeiEventSnapshot::SeatAdded { .. }
            | LibeiEventSnapshot::DeviceAdded(_)
            | LibeiEventSnapshot::Other { .. } => {}
        }
    }
}

impl EisEventSource for LibeiSenderContext {
    fn event_fd(&self) -> RawFd {
        LibeiSenderContext::event_fd(self)
    }

    fn dispatch_pending(&mut self) -> Vec<LibeiEventSnapshot> {
        LibeiSenderContext::dispatch_pending(self)
    }

    fn dispatch_pending_for_plan(&mut self, plan: &EisActionPlan) -> Vec<LibeiEventSnapshot> {
        LibeiSenderContext::dispatch_pending_for_plan(self, plan)
    }
}

impl Drop for LibeiSenderContext {
    fn drop(&mut self) {
        self.retained_devices.clear();
        // SAFETY: `context` is owned by this wrapper and released exactly once.
        unsafe { ei_unref(self.context.as_ptr()) };
    }
}

impl EisSelectedDeviceExecutor for LibeiSenderContext {
    type Error = LibeiExecutionError;

    fn apply_plan_to_selected_device(
        &mut self,
        selection: &EisDeviceSelection,
        plan: &EisActionPlan,
    ) -> std::result::Result<(), Self::Error> {
        let device = self
            .retained_devices
            .iter()
            .find(|retained| retained.id == selection.device_id)
            .map(|retained| retained.device)
            .ok_or_else(|| LibeiExecutionError::DeviceNotRetained {
                device_id: selection.device_id.clone(),
            })?;
        // SAFETY: `context` is live for this wrapper, and `device` is a retained
        // non-null libei device reference owned by this wrapper.
        let mut sink = unsafe { LibeiDeviceSink::from_raw(self.context.as_ptr(), device.as_ptr()) }
            .ok_or_else(|| LibeiExecutionError::DeviceNotRetained {
                device_id: selection.device_id.clone(),
            })?;
        apply_plan_to_sink(plan, &mut sink).map_err(LibeiExecutionError::Sink)
    }
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

fn libei_event_snapshot(
    event: *mut EiEvent,
    requested_capabilities: &[EisCapability],
) -> LibeiEventSnapshot {
    let event_type = LibeiEventType::from_raw(unsafe { ei_event_get_type(event) });
    match event_type {
        LibeiEventType::Connect => LibeiEventSnapshot::Connect,
        LibeiEventType::Disconnect => LibeiEventSnapshot::Disconnect,
        LibeiEventType::SeatAdded => {
            // SAFETY: libei permits retrieving the seat for seat events; NULL is handled.
            let seat = unsafe { ei_event_get_seat(event) };
            let capabilities = libei_seat_capabilities(seat);
            let bound_capabilities =
                bind_available_seat_capabilities(seat, requested_capabilities, &capabilities);
            LibeiEventSnapshot::SeatAdded {
                capabilities,
                bound_capabilities,
            }
        }
        LibeiEventType::SeatRemoved => LibeiEventSnapshot::SeatRemoved,
        LibeiEventType::DeviceAdded => {
            // SAFETY: libei permits retrieving the device for device events; NULL is handled.
            let device = unsafe { ei_event_get_device(event) };
            libei_device_info(device, false).map_or(
                LibeiEventSnapshot::Other { event_type: 5 },
                LibeiEventSnapshot::DeviceAdded,
            )
        }
        LibeiEventType::DeviceRemoved => {
            // SAFETY: libei permits retrieving the device for device events; NULL is handled.
            let device = unsafe { ei_event_get_device(event) };
            LibeiEventSnapshot::DeviceRemoved {
                device_id: libei_device_id(device),
            }
        }
        LibeiEventType::DevicePaused => {
            // SAFETY: libei permits retrieving the device for device events; NULL is handled.
            let device = unsafe { ei_event_get_device(event) };
            LibeiEventSnapshot::DevicePaused {
                device_id: libei_device_id(device),
            }
        }
        LibeiEventType::DeviceResumed => {
            // SAFETY: libei permits retrieving the device for device events; NULL is handled.
            let device = unsafe { ei_event_get_device(event) };
            libei_device_info(device, true).map_or(
                LibeiEventSnapshot::Other { event_type: 8 },
                LibeiEventSnapshot::DeviceResumed,
            )
        }
        LibeiEventType::Other(event_type) => LibeiEventSnapshot::Other { event_type },
    }
}

fn libei_seat_capabilities(seat: *mut EiSeat) -> Vec<EisCapability> {
    let Some(seat) = NonNull::new(seat) else {
        return Vec::new();
    };

    known_capabilities()
        .into_iter()
        .filter(|capability| unsafe {
            ei_seat_has_capability(seat.as_ptr(), capability_to_libei(*capability))
        })
        .collect()
}

fn bind_available_seat_capabilities(
    seat: *mut EiSeat,
    requested_capabilities: &[EisCapability],
    available_capabilities: &[EisCapability],
) -> Vec<EisCapability> {
    if NonNull::new(seat).is_none() {
        return Vec::new();
    }

    let capabilities = unique_capabilities(requested_capabilities)
        .into_iter()
        .filter(|capability| available_capabilities.contains(capability))
        .collect::<Vec<_>>();

    // SAFETY: `seat` is a non-null seat borrowed from a SeatAdded event. The
    // variadic call is terminated with a NULL sentinel and only includes libei
    // capability constants.
    unsafe { bind_seat_capability_slice(seat, &capabilities) };
    capabilities
}

fn unique_capabilities(capabilities: &[EisCapability]) -> Vec<EisCapability> {
    let mut unique = Vec::new();
    for capability in capabilities {
        if !unique.contains(capability) {
            unique.push(*capability);
        }
    }
    unique
}

unsafe fn bind_seat_capability_slice(seat: *mut EiSeat, capabilities: &[EisCapability]) {
    let null = ptr::null::<std::ffi::c_void>();
    match capabilities {
        [] => {}
        [a] => unsafe { ei_seat_bind_capabilities(seat, capability_to_libei(*a), null) },
        [a, b] => unsafe {
            ei_seat_bind_capabilities(seat, capability_to_libei(*a), capability_to_libei(*b), null)
        },
        [a, b, c] => unsafe {
            ei_seat_bind_capabilities(
                seat,
                capability_to_libei(*a),
                capability_to_libei(*b),
                capability_to_libei(*c),
                null,
            )
        },
        [a, b, c, d] => unsafe {
            ei_seat_bind_capabilities(
                seat,
                capability_to_libei(*a),
                capability_to_libei(*b),
                capability_to_libei(*c),
                capability_to_libei(*d),
                null,
            )
        },
        _ => unreachable!("Seatgeist only models four EIS capabilities today"),
    }
}

fn libei_device_info(device: *mut EiDevice, resumed: bool) -> Option<EisDeviceInfo> {
    let device = NonNull::new(device)?;
    let device_ptr = device.as_ptr();
    let id = libei_device_id(device_ptr);
    let name = libei_device_name(device_ptr);
    let kind = libei_device_kind(device_ptr);
    let capabilities = libei_device_capabilities(device_ptr);
    let regions = if kind == EisDeviceKind::Virtual {
        libei_device_regions(device_ptr)
    } else {
        Vec::new()
    };

    Some(EisDeviceInfo {
        id,
        name,
        kind,
        resumed,
        capabilities,
        regions,
    })
}

fn libei_device_id(device: *mut EiDevice) -> String {
    format!("{device:p}")
}

fn libei_device_name(device: *mut EiDevice) -> Option<String> {
    // SAFETY: libei returns NULL or a NUL-terminated borrowed string for the device lifetime.
    let name = unsafe { ei_device_get_name(device) };
    NonNull::new(name.cast_mut()).map(|name| {
        // SAFETY: non-null libei device names are valid C strings.
        unsafe { CStr::from_ptr(name.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    })
}

fn libei_device_kind(device: *mut EiDevice) -> EisDeviceKind {
    // SAFETY: `device` is a non-null libei device pointer from an event.
    match unsafe { ei_device_get_type(device) } {
        EI_DEVICE_TYPE_VIRTUAL => EisDeviceKind::Virtual,
        EI_DEVICE_TYPE_PHYSICAL => EisDeviceKind::Physical,
        other => EisDeviceKind::Unknown(other),
    }
}

fn libei_device_capabilities(device: *mut EiDevice) -> Vec<EisCapability> {
    known_capabilities()
        .into_iter()
        .filter(|capability| unsafe {
            ei_device_has_capability(device, capability_to_libei(*capability))
        })
        .collect()
}

fn libei_device_regions(device: *mut EiDevice) -> Vec<EisRegion> {
    let mut regions = Vec::new();
    for index in 0.. {
        // SAFETY: indexes are queried until libei reports NULL.
        let region = unsafe { ei_device_get_region(device, index) };
        let Some(region) = NonNull::new(region) else {
            break;
        };
        regions.push(libei_region(region.as_ptr()));
    }
    regions
}

fn libei_region(region: *mut EiRegion) -> EisRegion {
    // SAFETY: `region` is a non-null libei region pointer borrowed from a device.
    unsafe {
        EisRegion {
            x: f64::from(ei_region_get_x(region)),
            y: f64::from(ei_region_get_y(region)),
            width: f64::from(ei_region_get_width(region)),
            height: f64::from(ei_region_get_height(region)),
            scale: ei_region_get_physical_scale(region),
        }
    }
}

fn known_capabilities() -> [EisCapability; 5] {
    [
        EisCapability::PointerAbsolute,
        EisCapability::Keyboard,
        EisCapability::Button,
        EisCapability::Scroll,
        EisCapability::Text,
    ]
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

    fn keyboard_key(
        &mut self,
        keycode: u32,
        is_press: bool,
    ) -> std::result::Result<(), Self::Error> {
        // SAFETY: `device` is guaranteed valid by `from_raw` for this sink's lifetime.
        unsafe { ei_device_keyboard_key(self.device.as_ptr(), keycode, is_press) };
        Ok(())
    }

    fn text_keysym(&mut self, keysym: u32, is_press: bool) -> std::result::Result<(), Self::Error> {
        // SAFETY: `device` is guaranteed valid by `from_raw` for this sink's lifetime.
        unsafe { ei_device_text_keysym(self.device.as_ptr(), keysym, is_press) };
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

pub fn unicode_char_to_keysym(character: char) -> Result<u32> {
    // SAFETY: `xkb_utf32_to_keysym` is a pure conversion for one Unicode scalar value.
    let keysym = unsafe { xkb_utf32_to_keysym(u32::from(character)) };
    if keysym == 0 {
        return Err(EisPlanError::UnmappedKeysym { character });
    }
    Ok(keysym)
}

pub fn plan_text_as_keysyms(sequence: u32, text: &str) -> Result<EisActionPlan> {
    if text.is_empty() {
        return Err(EisPlanError::EmptyKeysymText);
    }
    let keysyms = text
        .chars()
        .map(unicode_char_to_keysym)
        .collect::<Result<Vec<_>>>()?;
    plan_text_keysyms(sequence, &keysyms)
}

pub fn plan_text_keysyms(sequence: u32, keysyms: &[u32]) -> Result<EisActionPlan> {
    if keysyms.is_empty() {
        return Err(EisPlanError::EmptyKeysymText);
    }

    let mut events = Vec::with_capacity(keysyms.len() * 3 + 2);
    events.push(EisEvent::StartEmulating { sequence });
    for keysym in keysyms {
        events.push(EisEvent::TextKeysym {
            keysym: *keysym,
            is_press: true,
        });
        events.push(EisEvent::TextKeysym {
            keysym: *keysym,
            is_press: false,
        });
        events.push(EisEvent::Frame);
    }
    events.push(EisEvent::StopEmulating);

    Ok(EisActionPlan {
        required_capabilities: vec![EisCapability::Text],
        events,
    })
}

pub fn plan_key_combo_evdev(sequence: u32, keycodes: &[u16]) -> Result<EisActionPlan> {
    if keycodes.is_empty() {
        return Err(EisPlanError::EmptyKeyCombo);
    }

    let mut events = Vec::with_capacity(keycodes.len() * 2 + 4);
    events.push(EisEvent::StartEmulating { sequence });
    for keycode in keycodes {
        events.push(EisEvent::KeyboardKey {
            keycode: u32::from(*keycode),
            is_press: true,
        });
    }
    events.push(EisEvent::Frame);
    for keycode in keycodes.iter().rev() {
        events.push(EisEvent::KeyboardKey {
            keycode: u32::from(*keycode),
            is_press: false,
        });
    }
    events.push(EisEvent::Frame);
    events.push(EisEvent::StopEmulating);

    Ok(EisActionPlan {
        required_capabilities: vec![EisCapability::Keyboard],
        events,
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
    use libseatgeist::CoordinateSpace;
    use std::collections::VecDeque;

    use super::*;

    #[derive(Default)]
    struct MockEventSource {
        event_fd: RawFd,
        pending_batches: VecDeque<Vec<LibeiEventSnapshot>>,
        plan_batches: VecDeque<Vec<LibeiEventSnapshot>>,
        executed_plans: Vec<MockExecutedPlan>,
    }

    #[derive(Debug, Clone, PartialEq)]
    struct MockExecutedPlan {
        selection: EisDeviceSelection,
        events: Vec<EisEvent>,
    }

    #[derive(Debug, Clone, Error, PartialEq, Eq)]
    enum MockExecutionError {
        #[error("mock executor rejected selected device")]
        RejectedDevice,
    }

    impl MockEventSource {
        fn with_event_fd(event_fd: RawFd) -> Self {
            Self {
                event_fd,
                ..Self::default()
            }
        }

        fn push_pending(&mut self, snapshots: Vec<LibeiEventSnapshot>) {
            self.pending_batches.push_back(snapshots);
        }

        fn push_plan(&mut self, snapshots: Vec<LibeiEventSnapshot>) {
            self.plan_batches.push_back(snapshots);
        }
    }

    impl EisEventSource for MockEventSource {
        fn event_fd(&self) -> RawFd {
            self.event_fd
        }

        fn dispatch_pending(&mut self) -> Vec<LibeiEventSnapshot> {
            self.pending_batches.pop_front().unwrap_or_default()
        }

        fn dispatch_pending_for_plan(&mut self, _plan: &EisActionPlan) -> Vec<LibeiEventSnapshot> {
            self.plan_batches.pop_front().unwrap_or_default()
        }
    }

    impl EisSelectedDeviceExecutor for MockEventSource {
        type Error = MockExecutionError;

        fn apply_plan_to_selected_device(
            &mut self,
            selection: &EisDeviceSelection,
            plan: &EisActionPlan,
        ) -> std::result::Result<(), Self::Error> {
            if selection.device_id == "rejected" {
                return Err(MockExecutionError::RejectedDevice);
            }
            self.executed_plans.push(MockExecutedPlan {
                selection: selection.clone(),
                events: plan.events.clone(),
            });
            Ok(())
        }
    }

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

        fn keyboard_key(
            &mut self,
            keycode: u32,
            is_press: bool,
        ) -> std::result::Result<(), Self::Error> {
            self.calls.push(format!("key:{keycode}:{is_press}"));
            Ok(())
        }

        fn text_keysym(
            &mut self,
            keysym: u32,
            is_press: bool,
        ) -> std::result::Result<(), Self::Error> {
            self.calls.push(format!("keysym:{keysym}:{is_press}"));
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

        fn keyboard_key(
            &mut self,
            _keycode: u32,
            _is_press: bool,
        ) -> std::result::Result<(), Self::Error> {
            Ok(())
        }

        fn text_keysym(
            &mut self,
            _keysym: u32,
            _is_press: bool,
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
    fn maps_libei_event_types() {
        assert_eq!(LibeiEventType::from_raw(1), LibeiEventType::Connect);
        assert_eq!(LibeiEventType::from_raw(2), LibeiEventType::Disconnect);
        assert_eq!(LibeiEventType::from_raw(3), LibeiEventType::SeatAdded);
        assert_eq!(LibeiEventType::from_raw(4), LibeiEventType::SeatRemoved);
        assert_eq!(LibeiEventType::from_raw(5), LibeiEventType::DeviceAdded);
        assert_eq!(LibeiEventType::from_raw(6), LibeiEventType::DeviceRemoved);
        assert_eq!(LibeiEventType::from_raw(7), LibeiEventType::DevicePaused);
        assert_eq!(LibeiEventType::from_raw(8), LibeiEventType::DeviceResumed);
        assert_eq!(LibeiEventType::from_raw(999), LibeiEventType::Other(999));
    }

    #[test]
    fn maps_libei_capability_bits_to_plan_capabilities() {
        assert_eq!(
            capability_to_libei(EisCapability::PointerAbsolute),
            EI_CAP_POINTER_ABSOLUTE
        );
        assert_eq!(
            capability_to_libei(EisCapability::Keyboard),
            EI_CAP_KEYBOARD
        );
        assert_eq!(capability_to_libei(EisCapability::Button), EI_CAP_BUTTON);
        assert_eq!(capability_to_libei(EisCapability::Scroll), EI_CAP_SCROLL);
        assert_eq!(capability_to_libei(EisCapability::Text), EI_CAP_TEXT);
        assert_eq!(
            capabilities_from_libei_bits(
                EI_CAP_POINTER_ABSOLUTE | EI_CAP_KEYBOARD | EI_CAP_BUTTON | EI_CAP_TEXT
            ),
            vec![
                EisCapability::PointerAbsolute,
                EisCapability::Keyboard,
                EisCapability::Button,
                EisCapability::Text,
            ]
        );
    }

    #[test]
    fn unique_capabilities_preserves_first_seen_order() {
        assert_eq!(
            unique_capabilities(&[
                EisCapability::Text,
                EisCapability::Button,
                EisCapability::Text,
                EisCapability::Scroll,
                EisCapability::Button,
            ]),
            vec![
                EisCapability::Text,
                EisCapability::Button,
                EisCapability::Scroll,
            ]
        );
    }

    #[test]
    fn seat_binding_returns_empty_for_missing_seat() {
        assert_eq!(
            bind_available_seat_capabilities(
                ptr::null_mut(),
                &[EisCapability::Text],
                &[EisCapability::Text],
            ),
            Vec::<EisCapability>::new()
        );
    }

    #[test]
    fn runtime_state_tracks_seat_bindings_and_selects_resumed_devices() {
        let text_plan = plan_text_utf8(1, "hello").expect("text plan");
        let mut state = EisRuntimeState::new();

        state.apply_snapshots(
            [
                LibeiEventSnapshot::Connect,
                LibeiEventSnapshot::SeatAdded {
                    capabilities: vec![EisCapability::Text, EisCapability::Button],
                    bound_capabilities: vec![EisCapability::Text],
                },
                LibeiEventSnapshot::DeviceAdded(device(
                    "text",
                    false,
                    vec![EisCapability::Text],
                    vec![],
                )),
                LibeiEventSnapshot::DeviceResumed(device(
                    "text",
                    true,
                    vec![EisCapability::Text],
                    vec![],
                )),
            ]
            .iter(),
        );

        assert!(state.connected());
        assert_eq!(
            state.seat_capabilities(),
            &[EisCapability::Text, EisCapability::Button]
        );
        assert_eq!(state.bound_capabilities(), &[EisCapability::Text]);
        assert_eq!(state.devices().len(), 1);
        assert!(state.devices()[0].resumed);
        assert_eq!(
            state.select_device_for_plan(&text_plan).expect("selection"),
            EisDeviceSelection {
                device_id: "text".to_string(),
                device_name: Some("device text".to_string()),
                matched_region: None,
            }
        );
    }

    #[test]
    fn runtime_state_applies_pause_remove_and_disconnect_events() {
        let text_plan = plan_text_utf8(1, "hello").expect("text plan");
        let mut state = EisRuntimeState::new();
        state.apply_snapshots(
            [
                LibeiEventSnapshot::Connect,
                LibeiEventSnapshot::SeatAdded {
                    capabilities: vec![EisCapability::Text],
                    bound_capabilities: vec![EisCapability::Text],
                },
                LibeiEventSnapshot::DeviceResumed(device(
                    "text",
                    true,
                    vec![EisCapability::Text],
                    vec![],
                )),
                LibeiEventSnapshot::DevicePaused {
                    device_id: "text".to_string(),
                },
            ]
            .iter(),
        );

        assert_eq!(
            state.select_device_for_plan(&text_plan),
            Err(EisDeviceSelectionError::NoCapableResumedDevice)
        );
        assert!(!state.devices()[0].resumed);

        state.apply_snapshot(&LibeiEventSnapshot::DeviceRemoved {
            device_id: "text".to_string(),
        });
        assert!(state.devices().is_empty());

        state.apply_snapshot(&LibeiEventSnapshot::DeviceResumed(device(
            "text",
            true,
            vec![EisCapability::Text],
            vec![],
        )));
        state.apply_snapshot(&LibeiEventSnapshot::Disconnect);
        assert!(!state.connected());
        assert!(state.seat_capabilities().is_empty());
        assert!(state.bound_capabilities().is_empty());
        assert!(state.devices().is_empty());
    }

    #[test]
    fn runtime_state_keeps_absolute_pointer_region_selection() {
        let plan = plan_pointer_click_absolute(1, point(75.0, 80.0), PointerButton::Left, 1)
            .expect("click plan");
        let mut state = EisRuntimeState::new();
        state.apply_snapshots(
            [
                LibeiEventSnapshot::Connect,
                LibeiEventSnapshot::SeatAdded {
                    capabilities: vec![EisCapability::PointerAbsolute, EisCapability::Button],
                    bound_capabilities: vec![EisCapability::PointerAbsolute, EisCapability::Button],
                },
                LibeiEventSnapshot::DeviceResumed(device(
                    "pointer",
                    true,
                    vec![EisCapability::PointerAbsolute, EisCapability::Button],
                    vec![region(50.0, 50.0, 100.0, 100.0)],
                )),
            ]
            .iter(),
        );

        assert_eq!(
            state.select_device_for_plan(&plan).expect("selection"),
            EisDeviceSelection {
                device_id: "pointer".to_string(),
                device_name: Some("device pointer".to_string()),
                matched_region: Some(region(50.0, 50.0, 100.0, 100.0)),
            }
        );
    }

    #[test]
    fn session_runtime_dispatches_pending_events_into_state() {
        let mut source = MockEventSource::with_event_fd(42);
        source.push_pending(vec![
            LibeiEventSnapshot::Connect,
            LibeiEventSnapshot::SeatAdded {
                capabilities: vec![EisCapability::Text],
                bound_capabilities: Vec::new(),
            },
        ]);
        let mut runtime = EisSessionRuntime::new(source);

        let snapshots = runtime.dispatch_pending();

        assert_eq!(runtime.event_fd(), 42);
        assert_eq!(snapshots.len(), 2);
        assert!(runtime.state().connected());
        assert_eq!(runtime.state().seat_capabilities(), &[EisCapability::Text]);
        assert!(runtime.state().devices().is_empty());
    }

    #[test]
    fn session_runtime_refreshes_for_plan_and_reports_selection() {
        let plan = plan_pointer_click_absolute(1, point(20.0, 20.0), PointerButton::Left, 1)
            .expect("click plan");
        let mut source = MockEventSource::default();
        source.push_plan(vec![
            LibeiEventSnapshot::SeatAdded {
                capabilities: vec![EisCapability::PointerAbsolute, EisCapability::Button],
                bound_capabilities: vec![EisCapability::PointerAbsolute, EisCapability::Button],
            },
            LibeiEventSnapshot::DeviceResumed(device(
                "pointer",
                true,
                vec![EisCapability::PointerAbsolute, EisCapability::Button],
                vec![region(0.0, 0.0, 100.0, 100.0)],
            )),
        ]);
        let mut runtime = EisSessionRuntime::new(source);

        let readiness = runtime.refresh_for_plan(&plan);

        assert_eq!(readiness.snapshots.len(), 2);
        assert_eq!(
            readiness.selection.expect("selection"),
            EisDeviceSelection {
                device_id: "pointer".to_string(),
                device_name: Some("device pointer".to_string()),
                matched_region: Some(region(0.0, 0.0, 100.0, 100.0)),
            }
        );
        assert_eq!(
            runtime.state().bound_capabilities(),
            &[EisCapability::PointerAbsolute, EisCapability::Button]
        );
    }

    #[test]
    fn session_runtime_reports_not_ready_without_resumed_device() {
        let plan = plan_text_utf8(1, "hello").expect("text plan");
        let mut source = MockEventSource::default();
        source.push_plan(vec![LibeiEventSnapshot::SeatAdded {
            capabilities: vec![EisCapability::Text],
            bound_capabilities: vec![EisCapability::Text],
        }]);
        let mut runtime = EisSessionRuntime::new(source);

        let readiness = runtime.refresh_for_plan(&plan);

        assert_eq!(
            readiness.selection,
            Err(EisDeviceSelectionError::NoCapableResumedDevice)
        );
        assert!(runtime.state().devices().is_empty());
    }

    #[test]
    fn execution_readiness_requires_connected_session() {
        let plan = plan_text_utf8(1, "hello").expect("text plan");
        let mut state = EisRuntimeState::new();
        state.apply_snapshots(
            [
                LibeiEventSnapshot::SeatAdded {
                    capabilities: vec![EisCapability::Text],
                    bound_capabilities: vec![EisCapability::Text],
                },
                LibeiEventSnapshot::DeviceResumed(device(
                    "text",
                    true,
                    vec![EisCapability::Text],
                    vec![],
                )),
            ]
            .iter(),
        );

        assert_eq!(
            state.execution_readiness_for_plan(&plan),
            Err(EisExecutionReadinessError::NotConnected)
        );
    }

    #[test]
    fn execution_readiness_requires_bound_capabilities() {
        let plan = plan_pointer_click_absolute(1, point(25.0, 25.0), PointerButton::Left, 1)
            .expect("click plan");
        let mut state = EisRuntimeState::new();
        state.apply_snapshots(
            [
                LibeiEventSnapshot::Connect,
                LibeiEventSnapshot::SeatAdded {
                    capabilities: vec![EisCapability::PointerAbsolute, EisCapability::Button],
                    bound_capabilities: vec![EisCapability::PointerAbsolute],
                },
                LibeiEventSnapshot::DeviceResumed(device(
                    "pointer",
                    true,
                    vec![EisCapability::PointerAbsolute, EisCapability::Button],
                    vec![region(0.0, 0.0, 100.0, 100.0)],
                )),
            ]
            .iter(),
        );

        assert_eq!(
            state.execution_readiness_for_plan(&plan),
            Err(EisExecutionReadinessError::MissingBoundCapabilities {
                missing: vec![EisCapability::Button],
            })
        );
    }

    #[test]
    fn session_runtime_reports_execution_readiness() {
        let plan = plan_pointer_click_absolute(1, point(20.0, 20.0), PointerButton::Left, 1)
            .expect("click plan");
        let mut source = MockEventSource::default();
        source.push_plan(vec![
            LibeiEventSnapshot::Connect,
            LibeiEventSnapshot::SeatAdded {
                capabilities: vec![EisCapability::PointerAbsolute, EisCapability::Button],
                bound_capabilities: vec![EisCapability::PointerAbsolute, EisCapability::Button],
            },
            LibeiEventSnapshot::DeviceResumed(device(
                "pointer",
                true,
                vec![EisCapability::PointerAbsolute, EisCapability::Button],
                vec![region(0.0, 0.0, 100.0, 100.0)],
            )),
        ]);
        let mut runtime = EisSessionRuntime::new(source);

        let readiness = runtime.refresh_execution_readiness(&plan);

        assert_eq!(readiness.snapshots.len(), 3);
        assert_eq!(
            readiness.selection.expect("selection"),
            EisDeviceSelection {
                device_id: "pointer".to_string(),
                device_name: Some("device pointer".to_string()),
                matched_region: Some(region(0.0, 0.0, 100.0, 100.0)),
            }
        );
    }

    #[test]
    fn session_runtime_executes_plan_only_after_readiness() {
        let plan = plan_text_utf8(1, "hello").expect("text plan");
        let mut source = MockEventSource::default();
        source.push_plan(vec![
            LibeiEventSnapshot::Connect,
            LibeiEventSnapshot::SeatAdded {
                capabilities: vec![EisCapability::Text],
                bound_capabilities: vec![EisCapability::Text],
            },
            LibeiEventSnapshot::DeviceResumed(device(
                "text",
                true,
                vec![EisCapability::Text],
                vec![],
            )),
        ]);
        let mut runtime = EisSessionRuntime::new(source);

        let report = runtime.execute_ready_plan(&plan).expect("plan executes");

        assert_eq!(report.snapshots.len(), 3);
        assert_eq!(report.selection.device_id, "text");
        assert_eq!(runtime.source().executed_plans.len(), 1);
        assert_eq!(
            runtime.source().executed_plans[0].selection.device_id,
            "text"
        );
        assert_eq!(runtime.source().executed_plans[0].events, plan.events);
    }

    #[test]
    fn session_runtime_does_not_execute_without_readiness() {
        let plan = plan_text_utf8(1, "hello").expect("text plan");
        let mut source = MockEventSource::default();
        source.push_plan(vec![
            LibeiEventSnapshot::SeatAdded {
                capabilities: vec![EisCapability::Text],
                bound_capabilities: vec![EisCapability::Text],
            },
            LibeiEventSnapshot::DeviceResumed(device(
                "text",
                true,
                vec![EisCapability::Text],
                vec![],
            )),
        ]);
        let mut runtime = EisSessionRuntime::new(source);

        let err = runtime
            .execute_ready_plan(&plan)
            .expect_err("readiness failure prevents execution");

        assert_eq!(
            err,
            EisPlanExecutionError::Readiness(EisExecutionReadinessError::NotConnected)
        );
        assert!(runtime.source().executed_plans.is_empty());
    }

    #[test]
    fn session_runtime_reports_executor_failure() {
        let plan = plan_text_utf8(1, "hello").expect("text plan");
        let mut source = MockEventSource::default();
        source.push_plan(vec![
            LibeiEventSnapshot::Connect,
            LibeiEventSnapshot::SeatAdded {
                capabilities: vec![EisCapability::Text],
                bound_capabilities: vec![EisCapability::Text],
            },
            LibeiEventSnapshot::DeviceResumed(device(
                "rejected",
                true,
                vec![EisCapability::Text],
                vec![],
            )),
        ]);
        let mut runtime = EisSessionRuntime::new(source);

        let err = runtime
            .execute_ready_plan(&plan)
            .expect_err("executor failure is surfaced");

        assert_eq!(
            err,
            EisPlanExecutionError::Executor(MockExecutionError::RejectedDevice)
        );
        assert!(runtime.source().executed_plans.is_empty());
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
    fn plans_xkb_text_keysyms_with_press_release_frames() {
        let plan = plan_text_keysyms(7, &[0x61, 0x20ac]).expect("keysym plan");

        assert_eq!(plan.required_capabilities, vec![EisCapability::Text]);
        assert_eq!(
            plan.events,
            vec![
                EisEvent::StartEmulating { sequence: 7 },
                EisEvent::TextKeysym {
                    keysym: 0x61,
                    is_press: true,
                },
                EisEvent::TextKeysym {
                    keysym: 0x61,
                    is_press: false,
                },
                EisEvent::Frame,
                EisEvent::TextKeysym {
                    keysym: 0x20ac,
                    is_press: true,
                },
                EisEvent::TextKeysym {
                    keysym: 0x20ac,
                    is_press: false,
                },
                EisEvent::Frame,
                EisEvent::StopEmulating,
            ]
        );
        assert_eq!(
            plan_text_keysyms(1, &[]),
            Err(EisPlanError::EmptyKeysymText)
        );
    }

    #[test]
    fn maps_unicode_text_to_xkb_keysyms() {
        assert_eq!(unicode_char_to_keysym('a').expect("ascii keysym"), 0x61);
        assert_eq!(unicode_char_to_keysym('€').expect("euro keysym"), 0x20ac);

        let plan = plan_text_as_keysyms(9, "a€").expect("text maps to keysyms");
        assert!(matches!(
            plan.events.as_slice(),
            [
                EisEvent::StartEmulating { sequence: 9 },
                EisEvent::TextKeysym {
                    keysym: 0x61,
                    is_press: true,
                },
                EisEvent::TextKeysym {
                    keysym: 0x61,
                    is_press: false,
                },
                EisEvent::Frame,
                EisEvent::TextKeysym {
                    keysym: 0x20ac,
                    is_press: true,
                },
                EisEvent::TextKeysym {
                    keysym: 0x20ac,
                    is_press: false,
                },
                EisEvent::Frame,
                EisEvent::StopEmulating,
            ]
        ));
        assert_eq!(
            plan_text_as_keysyms(1, ""),
            Err(EisPlanError::EmptyKeysymText)
        );
    }

    #[test]
    fn maps_us_xkb_level0_keysyms_to_evdev_codes() {
        let keymap = XkbKeymap::new_from_names(XkbKeymapNames::us_pc105()).expect("us xkb keymap");

        assert_eq!(xkb_keycode_to_evdev(38), Some(30));
        assert_eq!(xkb_keycode_to_evdev(7), None);
        assert_eq!(keymap.evdev_keycode_for_keysym_level0(0x61), Some(30));
        assert_eq!(keymap.evdev_keycode_for_keysym_level0(0x6c), Some(38));
        assert_eq!(keymap.evdev_keycode_for_keysym_level0(0x20), Some(57));
        assert_eq!(keymap.evdev_keycode_for_keysym_level0(u32::MAX), None);
        assert_eq!(
            keymap.keystroke_for_keysym(0x61),
            Some(XkbKeyStroke {
                evdev_keycode: 30,
                shift: false,
            })
        );
        assert_eq!(
            keymap.keystroke_for_keysym(0x41),
            Some(XkbKeyStroke {
                evdev_keycode: 30,
                shift: true,
            })
        );
        assert_eq!(keymap.keystroke_for_keysym(u32::MAX), None);
    }

    #[test]
    fn rejects_interior_nul_xkb_keymap_names() {
        assert_eq!(
            XkbKeymap::new_from_names(XkbKeymapNames {
                layout: Some("u\0s"),
                ..XkbKeymapNames::us_pc105()
            })
            .err(),
            Some(EisPlanError::InvalidXkbKeymapName { field: "layout" })
        );
    }

    #[test]
    fn plans_key_combo_with_evdev_codes() {
        let plan = plan_key_combo_evdev(8, &[29, 42, 38]).expect("key combo plan");

        assert_eq!(plan.required_capabilities, vec![EisCapability::Keyboard]);
        assert_eq!(
            plan.events,
            vec![
                EisEvent::StartEmulating { sequence: 8 },
                EisEvent::KeyboardKey {
                    keycode: 29,
                    is_press: true
                },
                EisEvent::KeyboardKey {
                    keycode: 42,
                    is_press: true
                },
                EisEvent::KeyboardKey {
                    keycode: 38,
                    is_press: true
                },
                EisEvent::Frame,
                EisEvent::KeyboardKey {
                    keycode: 38,
                    is_press: false
                },
                EisEvent::KeyboardKey {
                    keycode: 42,
                    is_press: false
                },
                EisEvent::KeyboardKey {
                    keycode: 29,
                    is_press: false
                },
                EisEvent::Frame,
                EisEvent::StopEmulating,
            ]
        );
        assert_eq!(
            plan_key_combo_evdev(1, &[]),
            Err(EisPlanError::EmptyKeyCombo)
        );
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
    fn applies_keysym_plan_to_sender_sink_in_order() {
        let plan = plan_text_keysyms(43, &[0x61]).expect("keysym plan");
        let mut sink = RecordingSink::default();

        apply_plan_to_sink(&plan, &mut sink).expect("apply keysym plan");

        assert_eq!(
            sink.calls,
            vec![
                "start:43",
                "keysym:97:true",
                "keysym:97:false",
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
