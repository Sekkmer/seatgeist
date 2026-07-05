use std::collections::HashMap;
use std::fmt;
use std::future::poll_fn;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;
use zbus::export::futures_core::Stream;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

pub const DESKTOP_BUS_NAME: &str = "org.freedesktop.portal.Desktop";
pub const DESKTOP_OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";
pub const SCREENSHOT_INTERFACE: &str = "org.freedesktop.portal.Screenshot";
pub const REMOTE_DESKTOP_INTERFACE: &str = "org.freedesktop.portal.RemoteDesktop";
pub const REQUEST_INTERFACE: &str = "org.freedesktop.portal.Request";
pub const SESSION_INTERFACE: &str = "org.freedesktop.portal.Session";
pub const SCREENSHOT_METHOD: &str = "Screenshot";
pub const CREATE_SESSION_METHOD: &str = "CreateSession";
pub const SELECT_DEVICES_METHOD: &str = "SelectDevices";
pub const START_METHOD: &str = "Start";
pub const CONNECT_TO_EIS_METHOD: &str = "ConnectToEIS";
pub const RESPONSE_SIGNAL: &str = "Response";
pub const REQUEST_PATH_PREFIX: &str = "/org/freedesktop/portal/desktop/request";
pub const SESSION_PATH_PREFIX: &str = "/org/freedesktop/portal/desktop/session";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortalContractError {
    EmptyHandleToken,
    InvalidHandleToken(String),
    InvalidSenderName(String),
    UnknownRemoteDesktopDeviceTypes(u32),
    UnknownPersistMode(u32),
    UnknownScreenshotTarget(u32),
    UnknownResponseCode(u32),
    MissingSessionHandle,
    MissingScreenshotUri,
    InvalidFileDescriptor(RawFd),
    UnsupportedUri(String),
    InvalidPercentEncoding(String),
    Transport(String),
}

impl fmt::Display for PortalContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyHandleToken => write!(formatter, "portal handle_token must be non-empty"),
            Self::InvalidHandleToken(token) => {
                write!(
                    formatter,
                    "portal handle_token is not a valid object path element: {token}"
                )
            }
            Self::InvalidSenderName(sender) => {
                write!(formatter, "portal sender unique name is invalid: {sender}")
            }
            Self::UnknownRemoteDesktopDeviceTypes(types) => {
                write!(
                    formatter,
                    "unknown portal remote desktop device type bits: {types:#x}"
                )
            }
            Self::UnknownPersistMode(mode) => {
                write!(
                    formatter,
                    "unknown portal remote desktop persist mode: {mode}"
                )
            }
            Self::UnknownScreenshotTarget(target) => {
                write!(
                    formatter,
                    "unknown portal screenshot target value: {target}"
                )
            }
            Self::UnknownResponseCode(code) => {
                write!(formatter, "unknown portal request response code: {code}")
            }
            Self::MissingSessionHandle => {
                write!(
                    formatter,
                    "portal remote desktop response omitted session_handle"
                )
            }
            Self::MissingScreenshotUri => {
                write!(formatter, "portal screenshot response omitted uri")
            }
            Self::InvalidFileDescriptor(fd) => {
                write!(formatter, "portal returned invalid file descriptor: {fd}")
            }
            Self::UnsupportedUri(uri) => {
                write!(formatter, "unsupported portal screenshot uri: {uri}")
            }
            Self::InvalidPercentEncoding(uri) => {
                write!(formatter, "invalid percent-encoding in portal uri: {uri}")
            }
            Self::Transport(message) => write!(formatter, "portal transport failed: {message}"),
        }
    }
}

impl std::error::Error for PortalContractError {}

pub type Result<T> = std::result::Result<T, PortalContractError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteDesktopDeviceTypes(u32);

impl RemoteDesktopDeviceTypes {
    pub const KEYBOARD: Self = Self(1);
    pub const POINTER: Self = Self(2);
    pub const TOUCHSCREEN: Self = Self(4);
    pub const ALL: Self = Self(Self::KEYBOARD.0 | Self::POINTER.0 | Self::TOUCHSCREEN.0);

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn keyboard_pointer() -> Self {
        Self(Self::KEYBOARD.0 | Self::POINTER.0)
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn validate(self) -> Result<()> {
        if self.0 != 0 && self.0 & !Self::ALL.0 == 0 {
            return Ok(());
        }
        Err(PortalContractError::UnknownRemoteDesktopDeviceTypes(self.0))
    }
}

impl TryFrom<u32> for RemoteDesktopDeviceTypes {
    type Error = PortalContractError;

    fn try_from(value: u32) -> Result<Self> {
        let types = Self(value);
        types.validate()?;
        Ok(types)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteDesktopPersistMode {
    DoNotPersist,
    ApplicationLifetime,
    ExplicitlyRevoked,
}

impl RemoteDesktopPersistMode {
    pub const fn value(self) -> u32 {
        match self {
            Self::DoNotPersist => 0,
            Self::ApplicationLifetime => 1,
            Self::ExplicitlyRevoked => 2,
        }
    }
}

impl TryFrom<u32> for RemoteDesktopPersistMode {
    type Error = PortalContractError;

    fn try_from(value: u32) -> Result<Self> {
        match value {
            0 => Ok(Self::DoNotPersist),
            1 => Ok(Self::ApplicationLifetime),
            2 => Ok(Self::ExplicitlyRevoked),
            other => Err(PortalContractError::UnknownPersistMode(other)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortalScreenshotTarget {
    Screen,
    Window,
    Area,
    ActiveWindow,
}

impl PortalScreenshotTarget {
    pub const fn value(self) -> u32 {
        match self {
            Self::Screen => 1,
            Self::Window => 2,
            Self::Area => 4,
            Self::ActiveWindow => 8,
        }
    }
}

impl TryFrom<u32> for PortalScreenshotTarget {
    type Error = PortalContractError;

    fn try_from(value: u32) -> Result<Self> {
        match value {
            1 => Ok(Self::Screen),
            2 => Ok(Self::Window),
            4 => Ok(Self::Area),
            8 => Ok(Self::ActiveWindow),
            other => Err(PortalContractError::UnknownScreenshotTarget(other)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortalResponseCode {
    Success,
    Cancelled,
    Other,
}

impl TryFrom<u32> for PortalResponseCode {
    type Error = PortalContractError;

    fn try_from(value: u32) -> Result<Self> {
        match value {
            0 => Ok(Self::Success),
            1 => Ok(Self::Cancelled),
            2 => Ok(Self::Other),
            other => Err(PortalContractError::UnknownResponseCode(other)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalScreenshotOptions {
    pub parent_window: String,
    pub handle_token: String,
    pub modal: bool,
    pub interactive: bool,
    pub target: Option<PortalScreenshotTarget>,
}

impl PortalScreenshotOptions {
    pub fn new(handle_token: impl Into<String>) -> Self {
        Self {
            parent_window: String::new(),
            handle_token: handle_token.into(),
            modal: true,
            interactive: false,
            target: None,
        }
    }

    pub fn validate(&self) -> Result<()> {
        validate_handle_token(&self.handle_token)
    }

    pub fn vardict_entry_count(&self) -> usize {
        3 + usize::from(self.target.is_some())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusctlPortalCall {
    pub program: &'static str,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalCreateSessionOptions {
    pub handle_token: String,
    pub session_handle_token: String,
}

impl PortalCreateSessionOptions {
    pub fn new(handle_token: impl Into<String>, session_handle_token: impl Into<String>) -> Self {
        Self {
            handle_token: handle_token.into(),
            session_handle_token: session_handle_token.into(),
        }
    }

    pub fn validate(&self) -> Result<()> {
        validate_handle_token(&self.handle_token)?;
        validate_handle_token(&self.session_handle_token)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalSelectDevicesOptions {
    pub handle_token: String,
    pub types: Option<RemoteDesktopDeviceTypes>,
    pub restore_token: Option<String>,
    pub persist_mode: Option<RemoteDesktopPersistMode>,
}

impl PortalSelectDevicesOptions {
    pub fn new(handle_token: impl Into<String>) -> Self {
        Self {
            handle_token: handle_token.into(),
            types: None,
            restore_token: None,
            persist_mode: None,
        }
    }

    pub fn validate(&self) -> Result<()> {
        validate_handle_token(&self.handle_token)?;
        if let Some(types) = self.types {
            types.validate()?;
        }
        Ok(())
    }

    pub fn vardict_entry_count(&self) -> usize {
        1 + usize::from(self.types.is_some())
            + usize::from(self.restore_token.is_some())
            + usize::from(self.persist_mode.is_some())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalStartOptions {
    pub parent_window: String,
    pub handle_token: String,
}

impl PortalStartOptions {
    pub fn new(handle_token: impl Into<String>) -> Self {
        Self {
            parent_window: String::new(),
            handle_token: handle_token.into(),
        }
    }

    pub fn validate(&self) -> Result<()> {
        validate_handle_token(&self.handle_token)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortalConnectToEisOptions;

impl PortalConnectToEisOptions {
    pub const fn new() -> Self {
        Self
    }

    pub const fn vardict_entry_count(self) -> usize {
        0
    }
}

impl Default for PortalConnectToEisOptions {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalRemoteDesktopSession {
    pub expected_session_path: String,
    pub actual_session_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalRemoteDesktopStart {
    pub devices: RemoteDesktopDeviceTypes,
    pub clipboard_enabled: bool,
    pub restore_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalRemoteDesktopRequestResponse {
    pub response: PortalResponseCode,
    pub session_handle: Option<String>,
    pub devices: Option<u32>,
    pub clipboard_enabled: Option<bool>,
    pub restore_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalRemoteDesktopOptions {
    pub create_session: PortalCreateSessionOptions,
    pub select_devices: PortalSelectDevicesOptions,
    pub start: PortalStartOptions,
}

impl PortalRemoteDesktopOptions {
    pub fn new(
        create_handle_token: impl Into<String>,
        session_handle_token: impl Into<String>,
        select_handle_token: impl Into<String>,
        start_handle_token: impl Into<String>,
    ) -> Self {
        Self {
            create_session: PortalCreateSessionOptions::new(
                create_handle_token,
                session_handle_token,
            ),
            select_devices: PortalSelectDevicesOptions::new(select_handle_token),
            start: PortalStartOptions::new(start_handle_token),
        }
    }

    pub fn validate(&self) -> Result<()> {
        self.create_session.validate()?;
        self.select_devices.validate()?;
        self.start.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalRemoteDesktopSessionStart {
    pub create_request_path: String,
    pub select_request_path: String,
    pub start_request_path: String,
    pub session: PortalRemoteDesktopSession,
    pub start: PortalRemoteDesktopStart,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalRemoteDesktopEisConnection {
    pub session_handle: String,
    pub fd: RawFd,
}

#[derive(Debug)]
pub struct PortalRemoteDesktopOwnedEisConnection {
    pub session_handle: String,
    pub fd: OwnedFd,
}

#[derive(Debug)]
pub struct PortalRemoteDesktopEisSession {
    pub session_start: PortalRemoteDesktopSessionStart,
    pub eis: PortalRemoteDesktopOwnedEisConnection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalRequestResponse {
    pub response: PortalResponseCode,
    pub uri: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalScreenshotCapture {
    pub expected_handle_path: String,
    pub actual_handle_path: String,
    pub uri: String,
    pub path: PathBuf,
}

pub trait PortalScreenshotTransport {
    fn unique_sender_name(&mut self) -> Result<String>;
    fn call_screenshot(&mut self, options: &PortalScreenshotOptions) -> Result<String>;
    fn wait_for_response(&mut self, handle_path: &str) -> Result<PortalRequestResponse>;
}

pub trait PortalRemoteDesktopTransport {
    fn unique_sender_name(&mut self) -> Result<String>;
    fn call_create_session(&mut self, options: &PortalCreateSessionOptions) -> Result<String>;
    fn call_select_devices(
        &mut self,
        session_handle: &str,
        options: &PortalSelectDevicesOptions,
    ) -> Result<String>;
    fn call_start(&mut self, session_handle: &str, options: &PortalStartOptions) -> Result<String>;
    fn call_connect_to_eis(
        &mut self,
        session_handle: &str,
        options: &PortalConnectToEisOptions,
    ) -> Result<RawFd>;
    fn wait_for_response(
        &mut self,
        handle_path: &str,
    ) -> Result<PortalRemoteDesktopRequestResponse>;
}

pub fn screenshot_busctl_call(options: &PortalScreenshotOptions) -> Result<BusctlPortalCall> {
    options.validate()?;
    let mut args = vec![
        "--user".to_string(),
        "call".to_string(),
        DESKTOP_BUS_NAME.to_string(),
        DESKTOP_OBJECT_PATH.to_string(),
        SCREENSHOT_INTERFACE.to_string(),
        SCREENSHOT_METHOD.to_string(),
        "sa{sv}".to_string(),
        options.parent_window.clone(),
        options.vardict_entry_count().to_string(),
        "handle_token".to_string(),
        "s".to_string(),
        options.handle_token.clone(),
        "modal".to_string(),
        "b".to_string(),
        options.modal.to_string(),
        "interactive".to_string(),
        "b".to_string(),
        options.interactive.to_string(),
    ];

    if let Some(target) = options.target {
        args.push("target".to_string());
        args.push("u".to_string());
        args.push(target.value().to_string());
    }

    Ok(BusctlPortalCall {
        program: "busctl",
        args,
    })
}

pub fn create_remote_desktop_session_busctl_call(
    options: &PortalCreateSessionOptions,
) -> Result<BusctlPortalCall> {
    options.validate()?;
    Ok(BusctlPortalCall {
        program: "busctl",
        args: vec![
            "--user".to_string(),
            "call".to_string(),
            DESKTOP_BUS_NAME.to_string(),
            DESKTOP_OBJECT_PATH.to_string(),
            REMOTE_DESKTOP_INTERFACE.to_string(),
            CREATE_SESSION_METHOD.to_string(),
            "a{sv}".to_string(),
            "2".to_string(),
            "handle_token".to_string(),
            "s".to_string(),
            options.handle_token.clone(),
            "session_handle_token".to_string(),
            "s".to_string(),
            options.session_handle_token.clone(),
        ],
    })
}

pub fn select_remote_desktop_devices_busctl_call(
    session_handle: &str,
    options: &PortalSelectDevicesOptions,
) -> Result<BusctlPortalCall> {
    validate_session_path(session_handle)?;
    options.validate()?;
    let mut args = vec![
        "--user".to_string(),
        "call".to_string(),
        DESKTOP_BUS_NAME.to_string(),
        DESKTOP_OBJECT_PATH.to_string(),
        REMOTE_DESKTOP_INTERFACE.to_string(),
        SELECT_DEVICES_METHOD.to_string(),
        "oa{sv}".to_string(),
        session_handle.to_string(),
        options.vardict_entry_count().to_string(),
        "handle_token".to_string(),
        "s".to_string(),
        options.handle_token.clone(),
    ];
    if let Some(types) = options.types {
        args.push("types".to_string());
        args.push("u".to_string());
        args.push(types.bits().to_string());
    }
    if let Some(restore_token) = &options.restore_token {
        args.push("restore_token".to_string());
        args.push("s".to_string());
        args.push(restore_token.clone());
    }
    if let Some(persist_mode) = options.persist_mode {
        args.push("persist_mode".to_string());
        args.push("u".to_string());
        args.push(persist_mode.value().to_string());
    }
    Ok(BusctlPortalCall {
        program: "busctl",
        args,
    })
}

pub fn start_remote_desktop_busctl_call(
    session_handle: &str,
    options: &PortalStartOptions,
) -> Result<BusctlPortalCall> {
    validate_session_path(session_handle)?;
    options.validate()?;
    Ok(BusctlPortalCall {
        program: "busctl",
        args: vec![
            "--user".to_string(),
            "call".to_string(),
            DESKTOP_BUS_NAME.to_string(),
            DESKTOP_OBJECT_PATH.to_string(),
            REMOTE_DESKTOP_INTERFACE.to_string(),
            START_METHOD.to_string(),
            "osa{sv}".to_string(),
            session_handle.to_string(),
            options.parent_window.clone(),
            "1".to_string(),
            "handle_token".to_string(),
            "s".to_string(),
            options.handle_token.clone(),
        ],
    })
}

pub fn connect_remote_desktop_eis_busctl_call(
    session_handle: &str,
    options: &PortalConnectToEisOptions,
) -> Result<BusctlPortalCall> {
    validate_session_path(session_handle)?;
    Ok(BusctlPortalCall {
        program: "busctl",
        args: vec![
            "--user".to_string(),
            "call".to_string(),
            DESKTOP_BUS_NAME.to_string(),
            DESKTOP_OBJECT_PATH.to_string(),
            REMOTE_DESKTOP_INTERFACE.to_string(),
            CONNECT_TO_EIS_METHOD.to_string(),
            "oa{sv}".to_string(),
            session_handle.to_string(),
            options.vardict_entry_count().to_string(),
        ],
    })
}

pub fn request_response_match_rule(handle_path: &str) -> String {
    format!(
        "type='signal',sender='{DESKTOP_BUS_NAME}',path='{handle_path}',interface='{REQUEST_INTERFACE}',member='{RESPONSE_SIGNAL}'"
    )
}

pub fn request_screenshot<T>(
    transport: &mut T,
    options: &PortalScreenshotOptions,
) -> Result<Option<PortalScreenshotCapture>>
where
    T: PortalScreenshotTransport,
{
    options.validate()?;
    let sender = transport.unique_sender_name()?;
    let expected_handle_path = expected_request_path(&sender, &options.handle_token)?;
    let actual_handle_path = transport.call_screenshot(options)?;
    validate_request_path(&actual_handle_path)?;
    let response = transport.wait_for_response(&actual_handle_path)?;
    let Some(uri) = parse_screenshot_uri(response.response, response.uri.as_deref())? else {
        return Ok(None);
    };
    let path = file_uri_to_path(&uri)?;
    Ok(Some(PortalScreenshotCapture {
        expected_handle_path,
        actual_handle_path,
        uri,
        path,
    }))
}

pub fn request_remote_desktop_session<T>(
    transport: &mut T,
    options: &PortalRemoteDesktopOptions,
) -> Result<Option<PortalRemoteDesktopSessionStart>>
where
    T: PortalRemoteDesktopTransport,
{
    options.validate()?;
    let sender = transport.unique_sender_name()?;
    let expected_session_path =
        expected_session_path(&sender, &options.create_session.session_handle_token)?;

    let create_request_path = transport.call_create_session(&options.create_session)?;
    validate_request_path(&create_request_path)?;
    let create_response = transport.wait_for_response(&create_request_path)?;
    let Some(session) = parse_remote_desktop_session_handle(
        create_response.response,
        create_response.session_handle.as_deref(),
        &expected_session_path,
    )?
    else {
        return Ok(None);
    };

    let select_request_path =
        transport.call_select_devices(&session.actual_session_path, &options.select_devices)?;
    validate_request_path(&select_request_path)?;
    let select_response = transport.wait_for_response(&select_request_path)?;
    if !parse_remote_desktop_select_response(select_response.response) {
        return Ok(None);
    }

    let start_request_path = transport.call_start(&session.actual_session_path, &options.start)?;
    validate_request_path(&start_request_path)?;
    let start_response = transport.wait_for_response(&start_request_path)?;
    let Some(start) = parse_remote_desktop_start_response(
        start_response.response,
        start_response.devices,
        start_response.clipboard_enabled,
        start_response.restore_token.as_deref(),
    )?
    else {
        return Ok(None);
    };

    Ok(Some(PortalRemoteDesktopSessionStart {
        create_request_path,
        select_request_path,
        start_request_path,
        session,
        start,
    }))
}

pub fn connect_remote_desktop_eis<T>(
    transport: &mut T,
    session_handle: &str,
    options: &PortalConnectToEisOptions,
) -> Result<PortalRemoteDesktopEisConnection>
where
    T: PortalRemoteDesktopTransport,
{
    validate_session_path(session_handle)?;
    let fd = transport.call_connect_to_eis(session_handle, options)?;
    if fd < 0 {
        return Err(PortalContractError::InvalidFileDescriptor(fd));
    }
    Ok(PortalRemoteDesktopEisConnection {
        session_handle: session_handle.to_string(),
        fd,
    })
}

pub async fn connect_remote_desktop_eis_zbus(
    session_handle: &str,
    options: &PortalConnectToEisOptions,
) -> Result<PortalRemoteDesktopOwnedEisConnection> {
    validate_session_path(session_handle)?;
    let connection = zbus::Connection::session()
        .await
        .map_err(|err| PortalContractError::Transport(format!("connect session bus: {err}")))?;
    connect_remote_desktop_eis_on_connection(&connection, session_handle, options).await
}

pub async fn request_screenshot_zbus(
    options: &PortalScreenshotOptions,
    response_timeout: Duration,
) -> Result<Option<PortalScreenshotCapture>> {
    options.validate()?;
    let connection = zbus::Connection::session()
        .await
        .map_err(|err| PortalContractError::Transport(format!("connect session bus: {err}")))?;
    let sender = connection
        .unique_name()
        .ok_or_else(|| {
            PortalContractError::Transport(
                "session bus did not assign a unique sender name".to_string(),
            )
        })?
        .as_str()
        .to_string();
    let expected_handle_path = expected_request_path(&sender, &options.handle_token)?;
    let request_proxy = request_proxy_for_path(&connection, expected_handle_path.clone()).await?;
    let mut response_stream = request_proxy
        .receive_signal(RESPONSE_SIGNAL)
        .await
        .map_err(|err| {
            PortalContractError::Transport(format!("subscribe expected Request response: {err}"))
        })?;

    let actual_handle_path = call_screenshot_zbus(&connection, options).await?;
    validate_request_path(&actual_handle_path)?;
    if actual_handle_path != expected_handle_path {
        let request_proxy = request_proxy_for_path(&connection, actual_handle_path.clone()).await?;
        response_stream = request_proxy
            .receive_signal(RESPONSE_SIGNAL)
            .await
            .map_err(|err| {
                PortalContractError::Transport(format!(
                    "subscribe returned Request response: {err}"
                ))
            })?;
    }

    let response = wait_for_zbus_response(&mut response_stream, response_timeout).await?;
    let Some(uri) = parse_screenshot_uri(response.response, response.uri.as_deref())? else {
        return Ok(None);
    };
    let path = file_uri_to_path(&uri)?;
    Ok(Some(PortalScreenshotCapture {
        expected_handle_path,
        actual_handle_path,
        uri,
        path,
    }))
}

pub async fn request_remote_desktop_session_zbus(
    options: &PortalRemoteDesktopOptions,
    response_timeout: Duration,
) -> Result<Option<PortalRemoteDesktopSessionStart>> {
    options.validate()?;
    let connection = zbus::Connection::session()
        .await
        .map_err(|err| PortalContractError::Transport(format!("connect session bus: {err}")))?;
    request_remote_desktop_session_on_connection(&connection, options, response_timeout).await
}

pub async fn request_remote_desktop_eis_zbus(
    options: &PortalRemoteDesktopOptions,
    connect_options: &PortalConnectToEisOptions,
    response_timeout: Duration,
) -> Result<Option<PortalRemoteDesktopEisSession>> {
    options.validate()?;
    let connection = zbus::Connection::session()
        .await
        .map_err(|err| PortalContractError::Transport(format!("connect session bus: {err}")))?;
    let Some(session_start) =
        request_remote_desktop_session_on_connection(&connection, options, response_timeout)
            .await?
    else {
        return Ok(None);
    };
    let eis = connect_remote_desktop_eis_on_connection(
        &connection,
        &session_start.session.actual_session_path,
        connect_options,
    )
    .await?;

    Ok(Some(PortalRemoteDesktopEisSession { session_start, eis }))
}

async fn request_remote_desktop_session_on_connection(
    connection: &zbus::Connection,
    options: &PortalRemoteDesktopOptions,
    response_timeout: Duration,
) -> Result<Option<PortalRemoteDesktopSessionStart>> {
    let sender = connection
        .unique_name()
        .ok_or_else(|| {
            PortalContractError::Transport(
                "session bus did not assign a unique sender name".to_string(),
            )
        })?
        .as_str()
        .to_string();
    let expected_session_path =
        expected_session_path(&sender, &options.create_session.session_handle_token)?;

    let expected_create_request_path =
        expected_request_path(&sender, &options.create_session.handle_token)?;
    let mut create_response_stream =
        subscribe_request_response(connection, expected_create_request_path.clone()).await?;
    let create_request_path = call_remote_desktop_create_session_zbus(connection, options).await?;
    validate_request_path(&create_request_path)?;
    if create_request_path != expected_create_request_path {
        create_response_stream =
            subscribe_request_response(connection, create_request_path.clone()).await?;
    }
    let create_response =
        wait_for_remote_desktop_zbus_response(&mut create_response_stream, response_timeout)
            .await?;
    let Some(session) = parse_remote_desktop_session_handle(
        create_response.response,
        create_response.session_handle.as_deref(),
        &expected_session_path,
    )?
    else {
        return Ok(None);
    };

    let expected_select_request_path =
        expected_request_path(&sender, &options.select_devices.handle_token)?;
    let mut select_response_stream =
        subscribe_request_response(connection, expected_select_request_path.clone()).await?;
    let select_request_path =
        call_remote_desktop_select_devices_zbus(connection, &session.actual_session_path, options)
            .await?;
    validate_request_path(&select_request_path)?;
    if select_request_path != expected_select_request_path {
        select_response_stream =
            subscribe_request_response(connection, select_request_path.clone()).await?;
    }
    let select_response =
        wait_for_remote_desktop_zbus_response(&mut select_response_stream, response_timeout)
            .await?;
    if !parse_remote_desktop_select_response(select_response.response) {
        return Ok(None);
    }

    let expected_start_request_path = expected_request_path(&sender, &options.start.handle_token)?;
    let mut start_response_stream =
        subscribe_request_response(connection, expected_start_request_path.clone()).await?;
    let start_request_path =
        call_remote_desktop_start_zbus(connection, &session.actual_session_path, options).await?;
    validate_request_path(&start_request_path)?;
    if start_request_path != expected_start_request_path {
        start_response_stream =
            subscribe_request_response(connection, start_request_path.clone()).await?;
    }
    let start_response =
        wait_for_remote_desktop_zbus_response(&mut start_response_stream, response_timeout).await?;
    let Some(start) = parse_remote_desktop_start_response(
        start_response.response,
        start_response.devices,
        start_response.clipboard_enabled,
        start_response.restore_token.as_deref(),
    )?
    else {
        return Ok(None);
    };

    Ok(Some(PortalRemoteDesktopSessionStart {
        create_request_path,
        select_request_path,
        start_request_path,
        session,
        start,
    }))
}

async fn connect_remote_desktop_eis_on_connection(
    connection: &zbus::Connection,
    session_handle: &str,
    options: &PortalConnectToEisOptions,
) -> Result<PortalRemoteDesktopOwnedEisConnection> {
    let fd = call_remote_desktop_connect_to_eis_zbus(connection, session_handle, options).await?;
    if fd.as_raw_fd() < 0 {
        return Err(PortalContractError::InvalidFileDescriptor(fd.as_raw_fd()));
    }
    Ok(PortalRemoteDesktopOwnedEisConnection {
        session_handle: session_handle.to_string(),
        fd,
    })
}

async fn request_proxy_for_path(
    connection: &zbus::Connection,
    handle_path: String,
) -> Result<zbus::Proxy<'static>> {
    zbus::Proxy::new_owned(
        connection.clone(),
        DESKTOP_BUS_NAME.to_string(),
        handle_path,
        REQUEST_INTERFACE.to_string(),
    )
    .await
    .map_err(|err| PortalContractError::Transport(format!("create Request proxy: {err}")))
}

async fn subscribe_request_response(
    connection: &zbus::Connection,
    handle_path: String,
) -> Result<zbus::proxy::SignalStream<'static>> {
    request_proxy_for_path(connection, handle_path)
        .await?
        .receive_signal(RESPONSE_SIGNAL)
        .await
        .map_err(|err| PortalContractError::Transport(format!("subscribe Request response: {err}")))
}

async fn call_screenshot_zbus(
    connection: &zbus::Connection,
    options: &PortalScreenshotOptions,
) -> Result<String> {
    let portal_proxy = zbus::Proxy::new(
        connection,
        DESKTOP_BUS_NAME,
        DESKTOP_OBJECT_PATH,
        SCREENSHOT_INTERFACE,
    )
    .await
    .map_err(|err| PortalContractError::Transport(format!("create Screenshot proxy: {err}")))?;
    let mut vardict = HashMap::<&str, Value<'_>>::new();
    vardict.insert("handle_token", Value::new(options.handle_token.as_str()));
    vardict.insert("modal", Value::new(options.modal));
    vardict.insert("interactive", Value::new(options.interactive));
    if let Some(target) = options.target {
        vardict.insert("target", Value::new(target.value()));
    }
    let handle: OwnedObjectPath = portal_proxy
        .call(
            SCREENSHOT_METHOD,
            &(options.parent_window.as_str(), vardict),
        )
        .await
        .map_err(|err| PortalContractError::Transport(format!("call Screenshot: {err}")))?;
    Ok(handle.to_string())
}

async fn remote_desktop_proxy(connection: &zbus::Connection) -> Result<zbus::Proxy<'_>> {
    zbus::Proxy::new(
        connection,
        DESKTOP_BUS_NAME,
        DESKTOP_OBJECT_PATH,
        REMOTE_DESKTOP_INTERFACE,
    )
    .await
    .map_err(|err| PortalContractError::Transport(format!("create RemoteDesktop proxy: {err}")))
}

async fn call_remote_desktop_create_session_zbus(
    connection: &zbus::Connection,
    options: &PortalRemoteDesktopOptions,
) -> Result<String> {
    let portal_proxy = remote_desktop_proxy(connection).await?;
    let mut vardict = HashMap::<&str, Value<'_>>::new();
    vardict.insert(
        "handle_token",
        Value::new(options.create_session.handle_token.as_str()),
    );
    vardict.insert(
        "session_handle_token",
        Value::new(options.create_session.session_handle_token.as_str()),
    );
    let handle: OwnedObjectPath = portal_proxy
        .call(CREATE_SESSION_METHOD, &(vardict))
        .await
        .map_err(|err| PortalContractError::Transport(format!("call CreateSession: {err}")))?;
    Ok(handle.to_string())
}

async fn call_remote_desktop_select_devices_zbus(
    connection: &zbus::Connection,
    session_handle: &str,
    options: &PortalRemoteDesktopOptions,
) -> Result<String> {
    let portal_proxy = remote_desktop_proxy(connection).await?;
    let session_handle = OwnedObjectPath::try_from(session_handle.to_string())
        .map_err(|err| PortalContractError::Transport(format!("invalid session handle: {err}")))?;
    let mut vardict = HashMap::<&str, Value<'_>>::new();
    vardict.insert(
        "handle_token",
        Value::new(options.select_devices.handle_token.as_str()),
    );
    if let Some(types) = options.select_devices.types {
        vardict.insert("types", Value::new(types.bits()));
    }
    if let Some(restore_token) = &options.select_devices.restore_token {
        vardict.insert("restore_token", Value::new(restore_token.as_str()));
    }
    if let Some(persist_mode) = options.select_devices.persist_mode {
        vardict.insert("persist_mode", Value::new(persist_mode.value()));
    }
    let handle: OwnedObjectPath = portal_proxy
        .call(SELECT_DEVICES_METHOD, &(session_handle, vardict))
        .await
        .map_err(|err| PortalContractError::Transport(format!("call SelectDevices: {err}")))?;
    Ok(handle.to_string())
}

async fn call_remote_desktop_start_zbus(
    connection: &zbus::Connection,
    session_handle: &str,
    options: &PortalRemoteDesktopOptions,
) -> Result<String> {
    let portal_proxy = remote_desktop_proxy(connection).await?;
    let session_handle = OwnedObjectPath::try_from(session_handle.to_string())
        .map_err(|err| PortalContractError::Transport(format!("invalid session handle: {err}")))?;
    let mut vardict = HashMap::<&str, Value<'_>>::new();
    vardict.insert(
        "handle_token",
        Value::new(options.start.handle_token.as_str()),
    );
    let handle: OwnedObjectPath = portal_proxy
        .call(
            START_METHOD,
            &(
                session_handle,
                options.start.parent_window.as_str(),
                vardict,
            ),
        )
        .await
        .map_err(|err| PortalContractError::Transport(format!("call Start: {err}")))?;
    Ok(handle.to_string())
}

async fn call_remote_desktop_connect_to_eis_zbus(
    connection: &zbus::Connection,
    session_handle: &str,
    options: &PortalConnectToEisOptions,
) -> Result<OwnedFd> {
    validate_session_path(session_handle)?;
    let portal_proxy = remote_desktop_proxy(connection).await?;
    let session_handle = OwnedObjectPath::try_from(session_handle.to_string())
        .map_err(|err| PortalContractError::Transport(format!("invalid session handle: {err}")))?;
    let vardict = HashMap::<&str, Value<'_>>::with_capacity(options.vardict_entry_count());
    let fd: zbus::zvariant::OwnedFd = portal_proxy
        .call(CONNECT_TO_EIS_METHOD, &(session_handle, vardict))
        .await
        .map_err(|err| PortalContractError::Transport(format!("call ConnectToEIS: {err}")))?;
    Ok(fd.into())
}

async fn wait_for_zbus_response(
    response_stream: &mut zbus::proxy::SignalStream<'_>,
    response_timeout: Duration,
) -> Result<PortalRequestResponse> {
    let message = tokio::time::timeout(
        response_timeout,
        poll_fn(|context| Pin::new(&mut *response_stream).poll_next(context)),
    )
    .await
    .map_err(|_| {
        PortalContractError::Transport(format!(
            "timed out waiting {}ms for portal Request response",
            response_timeout.as_millis()
        ))
    })?
    .ok_or_else(|| {
        PortalContractError::Transport("portal Request response stream ended".to_string())
    })?;
    let (response, results): (u32, HashMap<String, OwnedValue>) =
        message.body().deserialize().map_err(|err| {
            PortalContractError::Transport(format!("decode Request response signal: {err}"))
        })?;
    let uri = results
        .get("uri")
        .map(|value| String::try_from(&**value))
        .transpose()
        .map_err(|err| {
            PortalContractError::Transport(format!("decode screenshot uri result: {err}"))
        })?;
    Ok(PortalRequestResponse {
        response: PortalResponseCode::try_from(response)?,
        uri,
    })
}

async fn wait_for_remote_desktop_zbus_response(
    response_stream: &mut zbus::proxy::SignalStream<'_>,
    response_timeout: Duration,
) -> Result<PortalRemoteDesktopRequestResponse> {
    let message = tokio::time::timeout(
        response_timeout,
        poll_fn(|context| Pin::new(&mut *response_stream).poll_next(context)),
    )
    .await
    .map_err(|_| {
        PortalContractError::Transport(format!(
            "timed out waiting {}ms for portal Request response",
            response_timeout.as_millis()
        ))
    })?
    .ok_or_else(|| {
        PortalContractError::Transport("portal Request response stream ended".to_string())
    })?;
    let (response, results): (u32, HashMap<String, OwnedValue>) =
        message.body().deserialize().map_err(|err| {
            PortalContractError::Transport(format!("decode Request response signal: {err}"))
        })?;

    Ok(PortalRemoteDesktopRequestResponse {
        response: PortalResponseCode::try_from(response)?,
        session_handle: owned_string_result(&results, "session_handle")?,
        devices: owned_u32_result(&results, "devices")?,
        clipboard_enabled: owned_bool_result(&results, "clipboard_enabled")?,
        restore_token: owned_string_result(&results, "restore_token")?,
    })
}

fn owned_string_result(results: &HashMap<String, OwnedValue>, key: &str) -> Result<Option<String>> {
    results
        .get(key)
        .map(|value| String::try_from(&**value))
        .transpose()
        .map_err(|err| PortalContractError::Transport(format!("decode portal {key} result: {err}")))
}

fn owned_u32_result(results: &HashMap<String, OwnedValue>, key: &str) -> Result<Option<u32>> {
    results
        .get(key)
        .map(|value| u32::try_from(&**value))
        .transpose()
        .map_err(|err| PortalContractError::Transport(format!("decode portal {key} result: {err}")))
}

fn owned_bool_result(results: &HashMap<String, OwnedValue>, key: &str) -> Result<Option<bool>> {
    results
        .get(key)
        .map(|value| bool::try_from(&**value))
        .transpose()
        .map_err(|err| PortalContractError::Transport(format!("decode portal {key} result: {err}")))
}

pub fn expected_request_path(sender_unique_name: &str, handle_token: &str) -> Result<String> {
    validate_handle_token(handle_token)?;
    let sender = sender_unique_name.trim();
    if !sender.starts_with(':') || sender.len() == 1 {
        return Err(PortalContractError::InvalidSenderName(
            sender_unique_name.to_string(),
        ));
    }
    let sender_path_element = sender[1..].replace('.', "_");
    validate_handle_token(&sender_path_element)?;
    Ok(format!(
        "{REQUEST_PATH_PREFIX}/{sender_path_element}/{handle_token}"
    ))
}

pub fn expected_session_path(
    sender_unique_name: &str,
    session_handle_token: &str,
) -> Result<String> {
    validate_handle_token(session_handle_token)?;
    let sender = sender_unique_name.trim();
    if !sender.starts_with(':') || sender.len() == 1 {
        return Err(PortalContractError::InvalidSenderName(
            sender_unique_name.to_string(),
        ));
    }
    let sender_path_element = sender[1..].replace('.', "_");
    validate_handle_token(&sender_path_element)?;
    Ok(format!(
        "{SESSION_PATH_PREFIX}/{sender_path_element}/{session_handle_token}"
    ))
}

pub fn validate_request_path(path: &str) -> Result<()> {
    let Some(rest) = path.strip_prefix(REQUEST_PATH_PREFIX) else {
        return Err(PortalContractError::Transport(format!(
            "request handle path is outside portal request namespace: {path}"
        )));
    };
    let rest = rest
        .strip_prefix('/')
        .ok_or_else(|| PortalContractError::Transport(format!("malformed request path: {path}")))?;
    let mut parts = rest.split('/');
    let sender = parts.next().ok_or_else(|| {
        PortalContractError::Transport(format!("missing sender in request path: {path}"))
    })?;
    let token = parts.next().ok_or_else(|| {
        PortalContractError::Transport(format!("missing token in request path: {path}"))
    })?;
    if parts.next().is_some() {
        return Err(PortalContractError::Transport(format!(
            "request path has too many elements: {path}"
        )));
    }
    validate_handle_token(sender)?;
    validate_handle_token(token)
}

pub fn validate_session_path(path: &str) -> Result<()> {
    validate_portal_object_path(path, SESSION_PATH_PREFIX, "session")
}

fn validate_portal_object_path(path: &str, prefix: &str, label: &str) -> Result<()> {
    let Some(rest) = path.strip_prefix(prefix) else {
        return Err(PortalContractError::Transport(format!(
            "{label} handle path is outside portal {label} namespace: {path}"
        )));
    };
    let rest = rest
        .strip_prefix('/')
        .ok_or_else(|| PortalContractError::Transport(format!("malformed {label} path: {path}")))?;
    let mut parts = rest.split('/');
    let sender = parts.next().ok_or_else(|| {
        PortalContractError::Transport(format!("missing sender in {label} path: {path}"))
    })?;
    let token = parts.next().ok_or_else(|| {
        PortalContractError::Transport(format!("missing token in {label} path: {path}"))
    })?;
    if parts.next().is_some() {
        return Err(PortalContractError::Transport(format!(
            "{label} path has too many elements: {path}"
        )));
    }
    validate_handle_token(sender)?;
    validate_handle_token(token)
}

pub fn parse_screenshot_uri(
    response: PortalResponseCode,
    uri: Option<&str>,
) -> Result<Option<String>> {
    match response {
        PortalResponseCode::Success => uri
            .filter(|value| !value.trim().is_empty())
            .map(|value| Ok(Some(value.to_string())))
            .unwrap_or(Err(PortalContractError::MissingScreenshotUri)),
        PortalResponseCode::Cancelled | PortalResponseCode::Other => Ok(None),
    }
}

pub fn parse_remote_desktop_session_handle(
    response: PortalResponseCode,
    session_handle: Option<&str>,
    expected_session_path: &str,
) -> Result<Option<PortalRemoteDesktopSession>> {
    match response {
        PortalResponseCode::Success => {
            let actual_session_path = session_handle
                .filter(|value| !value.trim().is_empty())
                .ok_or(PortalContractError::MissingSessionHandle)?;
            validate_session_path(actual_session_path)?;
            validate_session_path(expected_session_path)?;
            Ok(Some(PortalRemoteDesktopSession {
                expected_session_path: expected_session_path.to_string(),
                actual_session_path: actual_session_path.to_string(),
            }))
        }
        PortalResponseCode::Cancelled | PortalResponseCode::Other => Ok(None),
    }
}

pub fn parse_remote_desktop_select_response(response: PortalResponseCode) -> bool {
    matches!(response, PortalResponseCode::Success)
}

pub fn parse_remote_desktop_start_response(
    response: PortalResponseCode,
    devices: Option<u32>,
    clipboard_enabled: Option<bool>,
    restore_token: Option<&str>,
) -> Result<Option<PortalRemoteDesktopStart>> {
    match response {
        PortalResponseCode::Success => {
            let devices = RemoteDesktopDeviceTypes::try_from(devices.unwrap_or(0))?;
            Ok(Some(PortalRemoteDesktopStart {
                devices,
                clipboard_enabled: clipboard_enabled.unwrap_or(false),
                restore_token: restore_token.map(str::to_string),
            }))
        }
        PortalResponseCode::Cancelled | PortalResponseCode::Other => Ok(None),
    }
}

pub fn file_uri_to_path(uri: &str) -> Result<PathBuf> {
    let Some(rest) = uri.strip_prefix("file://") else {
        return Err(PortalContractError::UnsupportedUri(uri.to_string()));
    };
    if rest.is_empty() {
        return Err(PortalContractError::UnsupportedUri(uri.to_string()));
    }
    if !rest.starts_with('/') {
        return Err(PortalContractError::UnsupportedUri(uri.to_string()));
    }
    Ok(PathBuf::from(percent_decode(rest, uri)?))
}

pub fn validate_handle_token(token: &str) -> Result<()> {
    if token.is_empty() {
        return Err(PortalContractError::EmptyHandleToken);
    }
    if token
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Ok(());
    }
    Err(PortalContractError::InvalidHandleToken(token.to_string()))
}

fn percent_decode(input: &str, original_uri: &str) -> Result<String> {
    let mut bytes = Vec::with_capacity(input.len());
    let raw = input.as_bytes();
    let mut index = 0;
    while index < raw.len() {
        if raw[index] == b'%' {
            if index + 2 >= raw.len() {
                return Err(PortalContractError::InvalidPercentEncoding(
                    original_uri.to_string(),
                ));
            }
            let high = hex_value(raw[index + 1]).ok_or_else(|| {
                PortalContractError::InvalidPercentEncoding(original_uri.to_string())
            })?;
            let low = hex_value(raw[index + 2]).ok_or_else(|| {
                PortalContractError::InvalidPercentEncoding(original_uri.to_string())
            })?;
            bytes.push((high << 4) | low);
            index += 3;
        } else {
            bytes.push(raw[index]);
            index += 1;
        }
    }
    String::from_utf8(bytes)
        .map_err(|_| PortalContractError::InvalidPercentEncoding(original_uri.to_string()))
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct MockScreenshotTransport {
        sender: String,
        returned_handle: String,
        response: PortalRequestResponse,
        called_options: Option<PortalScreenshotOptions>,
        waited_handle: Option<String>,
    }

    impl MockScreenshotTransport {
        fn success() -> Self {
            Self {
                sender: ":1.42".to_string(),
                returned_handle: "/org/freedesktop/portal/desktop/request/1_42/seatgeist_abc"
                    .to_string(),
                response: PortalRequestResponse {
                    response: PortalResponseCode::Success,
                    uri: Some("file:///run/user/1000/doc/abc/screen%20shot.png".to_string()),
                },
                called_options: None,
                waited_handle: None,
            }
        }
    }

    impl PortalScreenshotTransport for MockScreenshotTransport {
        fn unique_sender_name(&mut self) -> Result<String> {
            Ok(self.sender.clone())
        }

        fn call_screenshot(&mut self, options: &PortalScreenshotOptions) -> Result<String> {
            self.called_options = Some(options.clone());
            Ok(self.returned_handle.clone())
        }

        fn wait_for_response(&mut self, handle_path: &str) -> Result<PortalRequestResponse> {
            self.waited_handle = Some(handle_path.to_string());
            Ok(self.response.clone())
        }
    }

    #[derive(Debug)]
    struct MockRemoteDesktopTransport {
        sender: String,
        create_handle: String,
        select_handle: String,
        start_handle: String,
        responses: Vec<PortalRemoteDesktopRequestResponse>,
        calls: Vec<String>,
        waited_handles: Vec<String>,
    }

    impl MockRemoteDesktopTransport {
        fn success() -> Self {
            Self {
                sender: ":1.42".to_string(),
                create_handle: "/org/freedesktop/portal/desktop/request/1_42/seatgeist_create"
                    .to_string(),
                select_handle: "/org/freedesktop/portal/desktop/request/1_42/seatgeist_select"
                    .to_string(),
                start_handle: "/org/freedesktop/portal/desktop/request/1_42/seatgeist_start"
                    .to_string(),
                responses: vec![
                    PortalRemoteDesktopRequestResponse {
                        response: PortalResponseCode::Success,
                        session_handle: Some(
                            "/org/freedesktop/portal/desktop/session/1_42/seatgeist_session"
                                .to_string(),
                        ),
                        devices: None,
                        clipboard_enabled: None,
                        restore_token: None,
                    },
                    PortalRemoteDesktopRequestResponse {
                        response: PortalResponseCode::Success,
                        session_handle: None,
                        devices: None,
                        clipboard_enabled: None,
                        restore_token: None,
                    },
                    PortalRemoteDesktopRequestResponse {
                        response: PortalResponseCode::Success,
                        session_handle: None,
                        devices: Some(RemoteDesktopDeviceTypes::keyboard_pointer().bits()),
                        clipboard_enabled: Some(true),
                        restore_token: Some("restore_next".to_string()),
                    },
                ],
                calls: Vec::new(),
                waited_handles: Vec::new(),
            }
        }
    }

    impl PortalRemoteDesktopTransport for MockRemoteDesktopTransport {
        fn unique_sender_name(&mut self) -> Result<String> {
            Ok(self.sender.clone())
        }

        fn call_create_session(&mut self, options: &PortalCreateSessionOptions) -> Result<String> {
            self.calls.push(format!(
                "create:{}:{}",
                options.handle_token, options.session_handle_token
            ));
            Ok(self.create_handle.clone())
        }

        fn call_select_devices(
            &mut self,
            session_handle: &str,
            options: &PortalSelectDevicesOptions,
        ) -> Result<String> {
            self.calls.push(format!(
                "select:{session_handle}:{}:{}",
                options.handle_token,
                options
                    .types
                    .map(RemoteDesktopDeviceTypes::bits)
                    .unwrap_or(0)
            ));
            Ok(self.select_handle.clone())
        }

        fn call_start(
            &mut self,
            session_handle: &str,
            options: &PortalStartOptions,
        ) -> Result<String> {
            self.calls.push(format!(
                "start:{session_handle}:{}:{}",
                options.handle_token, options.parent_window
            ));
            Ok(self.start_handle.clone())
        }

        fn call_connect_to_eis(
            &mut self,
            session_handle: &str,
            _options: &PortalConnectToEisOptions,
        ) -> Result<RawFd> {
            self.calls.push(format!("connect-eis:{session_handle}"));
            Ok(42)
        }

        fn wait_for_response(
            &mut self,
            handle_path: &str,
        ) -> Result<PortalRemoteDesktopRequestResponse> {
            self.waited_handles.push(handle_path.to_string());
            Ok(self.responses.remove(0))
        }
    }

    #[test]
    fn validates_handle_tokens_as_object_path_elements() {
        assert!(validate_handle_token("seatgeist_123").is_ok());
        assert!(matches!(
            validate_handle_token(""),
            Err(PortalContractError::EmptyHandleToken)
        ));
        assert!(matches!(
            validate_handle_token("bad-token"),
            Err(PortalContractError::InvalidHandleToken(_))
        ));
        assert!(matches!(
            validate_handle_token("bad/token"),
            Err(PortalContractError::InvalidHandleToken(_))
        ));
    }

    #[test]
    fn models_remote_desktop_device_bitmasks() -> Result<()> {
        let types = RemoteDesktopDeviceTypes::keyboard_pointer();
        assert_eq!(types.bits(), 3);
        assert!(types.contains(RemoteDesktopDeviceTypes::KEYBOARD));
        assert!(types.contains(RemoteDesktopDeviceTypes::POINTER));
        assert!(!types.contains(RemoteDesktopDeviceTypes::TOUCHSCREEN));
        assert_eq!(
            RemoteDesktopDeviceTypes::try_from(7)?,
            RemoteDesktopDeviceTypes::ALL
        );
        assert!(matches!(
            RemoteDesktopDeviceTypes::try_from(0),
            Err(PortalContractError::UnknownRemoteDesktopDeviceTypes(0))
        ));
        assert!(matches!(
            RemoteDesktopDeviceTypes::try_from(8),
            Err(PortalContractError::UnknownRemoteDesktopDeviceTypes(8))
        ));
        Ok(())
    }

    #[test]
    fn models_remote_desktop_persist_modes() -> Result<()> {
        assert_eq!(RemoteDesktopPersistMode::DoNotPersist.value(), 0);
        assert_eq!(
            RemoteDesktopPersistMode::try_from(2)?,
            RemoteDesktopPersistMode::ExplicitlyRevoked
        );
        assert!(matches!(
            RemoteDesktopPersistMode::try_from(3),
            Err(PortalContractError::UnknownPersistMode(3))
        ));
        Ok(())
    }

    #[test]
    fn default_options_omit_version_three_target_for_compatibility() {
        let options = PortalScreenshotOptions::new("seatgeist_abc");
        assert_eq!(options.target, None);
        assert_eq!(options.vardict_entry_count(), 3);
    }

    #[test]
    fn validates_request_handle_paths() {
        assert!(
            validate_request_path("/org/freedesktop/portal/desktop/request/1_42/seatgeist_abc")
                .is_ok()
        );
        assert!(matches!(
            validate_request_path("/org/freedesktop/portal/desktop/request/1-42/seatgeist_abc"),
            Err(PortalContractError::InvalidHandleToken(_))
        ));
        assert!(matches!(
            validate_request_path("/not/portal/request"),
            Err(PortalContractError::Transport(_))
        ));
    }

    #[test]
    fn validates_session_handle_paths() {
        assert!(
            validate_session_path("/org/freedesktop/portal/desktop/session/1_42/seatgeist_abc")
                .is_ok()
        );
        assert!(matches!(
            validate_session_path("/org/freedesktop/portal/desktop/session/1-42/seatgeist_abc"),
            Err(PortalContractError::InvalidHandleToken(_))
        ));
        assert!(matches!(
            validate_session_path("/org/freedesktop/portal/desktop/request/1_42/seatgeist_abc"),
            Err(PortalContractError::Transport(_))
        ));
    }

    #[test]
    fn builds_expected_request_path_from_sender_and_token() -> Result<()> {
        assert_eq!(
            expected_request_path(":1.42", "seatgeist_abc")?,
            "/org/freedesktop/portal/desktop/request/1_42/seatgeist_abc"
        );
        assert!(matches!(
            expected_request_path("1.42", "seatgeist_abc"),
            Err(PortalContractError::InvalidSenderName(_))
        ));
        Ok(())
    }

    #[test]
    fn builds_expected_session_path_from_sender_and_token() -> Result<()> {
        assert_eq!(
            expected_session_path(":1.42", "seatgeist_session")?,
            "/org/freedesktop/portal/desktop/session/1_42/seatgeist_session"
        );
        assert!(matches!(
            expected_session_path("1.42", "seatgeist_session"),
            Err(PortalContractError::InvalidSenderName(_))
        ));
        Ok(())
    }

    #[test]
    fn builds_screenshot_busctl_call_with_vardict_options() -> Result<()> {
        let mut options = PortalScreenshotOptions::new("seatgeist_abc");
        options.modal = false;
        options.interactive = true;
        options.target = Some(PortalScreenshotTarget::ActiveWindow);

        let call = screenshot_busctl_call(&options)?;
        assert_eq!(call.program, "busctl");
        assert_eq!(
            call.args,
            vec![
                "--user",
                "call",
                DESKTOP_BUS_NAME,
                DESKTOP_OBJECT_PATH,
                SCREENSHOT_INTERFACE,
                SCREENSHOT_METHOD,
                "sa{sv}",
                "",
                "4",
                "handle_token",
                "s",
                "seatgeist_abc",
                "modal",
                "b",
                "false",
                "interactive",
                "b",
                "true",
                "target",
                "u",
                "8",
            ]
        );
        Ok(())
    }

    #[test]
    fn builds_remote_desktop_create_session_busctl_call() -> Result<()> {
        let options = PortalCreateSessionOptions::new("seatgeist_create", "seatgeist_session");
        let call = create_remote_desktop_session_busctl_call(&options)?;
        assert_eq!(call.program, "busctl");
        assert_eq!(
            call.args,
            vec![
                "--user",
                "call",
                DESKTOP_BUS_NAME,
                DESKTOP_OBJECT_PATH,
                REMOTE_DESKTOP_INTERFACE,
                CREATE_SESSION_METHOD,
                "a{sv}",
                "2",
                "handle_token",
                "s",
                "seatgeist_create",
                "session_handle_token",
                "s",
                "seatgeist_session",
            ]
        );
        Ok(())
    }

    #[test]
    fn builds_remote_desktop_select_devices_busctl_call() -> Result<()> {
        let session = "/org/freedesktop/portal/desktop/session/1_42/seatgeist_session";
        let mut options = PortalSelectDevicesOptions::new("seatgeist_select");
        options.types = Some(RemoteDesktopDeviceTypes::keyboard_pointer());
        options.restore_token = Some("restore_once".to_string());
        options.persist_mode = Some(RemoteDesktopPersistMode::ApplicationLifetime);

        let call = select_remote_desktop_devices_busctl_call(session, &options)?;
        assert_eq!(
            call.args,
            vec![
                "--user",
                "call",
                DESKTOP_BUS_NAME,
                DESKTOP_OBJECT_PATH,
                REMOTE_DESKTOP_INTERFACE,
                SELECT_DEVICES_METHOD,
                "oa{sv}",
                session,
                "4",
                "handle_token",
                "s",
                "seatgeist_select",
                "types",
                "u",
                "3",
                "restore_token",
                "s",
                "restore_once",
                "persist_mode",
                "u",
                "1",
            ]
        );
        Ok(())
    }

    #[test]
    fn builds_remote_desktop_start_busctl_call() -> Result<()> {
        let session = "/org/freedesktop/portal/desktop/session/1_42/seatgeist_session";
        let mut options = PortalStartOptions::new("seatgeist_start");
        options.parent_window = "wayland:app-window".to_string();

        let call = start_remote_desktop_busctl_call(session, &options)?;
        assert_eq!(
            call.args,
            vec![
                "--user",
                "call",
                DESKTOP_BUS_NAME,
                DESKTOP_OBJECT_PATH,
                REMOTE_DESKTOP_INTERFACE,
                START_METHOD,
                "osa{sv}",
                session,
                "wayland:app-window",
                "1",
                "handle_token",
                "s",
                "seatgeist_start",
            ]
        );
        Ok(())
    }

    #[test]
    fn builds_remote_desktop_connect_to_eis_busctl_call() -> Result<()> {
        let session = "/org/freedesktop/portal/desktop/session/1_42/seatgeist_session";
        let options = PortalConnectToEisOptions::new();

        let call = connect_remote_desktop_eis_busctl_call(session, &options)?;
        assert_eq!(
            call.args,
            vec![
                "--user",
                "call",
                DESKTOP_BUS_NAME,
                DESKTOP_OBJECT_PATH,
                REMOTE_DESKTOP_INTERFACE,
                CONNECT_TO_EIS_METHOD,
                "oa{sv}",
                session,
                "0",
            ]
        );
        assert!(connect_remote_desktop_eis_busctl_call("/not/a/session", &options).is_err());
        Ok(())
    }

    #[test]
    fn runs_remote_desktop_session_lifecycle_with_transport() -> Result<()> {
        let mut transport = MockRemoteDesktopTransport::success();
        let mut options = PortalRemoteDesktopOptions::new(
            "seatgeist_create",
            "seatgeist_session",
            "seatgeist_select",
            "seatgeist_start",
        );
        options.select_devices.types = Some(RemoteDesktopDeviceTypes::keyboard_pointer());
        options.start.parent_window = "wayland:test-window".to_string();

        let result = request_remote_desktop_session(&mut transport, &options)?
            .expect("session should start");
        assert_eq!(
            result.session.expected_session_path,
            "/org/freedesktop/portal/desktop/session/1_42/seatgeist_session"
        );
        assert_eq!(
            result.session.actual_session_path,
            "/org/freedesktop/portal/desktop/session/1_42/seatgeist_session"
        );
        assert_eq!(
            result.start.devices,
            RemoteDesktopDeviceTypes::keyboard_pointer()
        );
        assert!(result.start.clipboard_enabled);
        assert_eq!(result.start.restore_token.as_deref(), Some("restore_next"));
        assert_eq!(
            transport.waited_handles,
            vec![
                "/org/freedesktop/portal/desktop/request/1_42/seatgeist_create",
                "/org/freedesktop/portal/desktop/request/1_42/seatgeist_select",
                "/org/freedesktop/portal/desktop/request/1_42/seatgeist_start",
            ]
        );
        assert_eq!(
            transport.calls,
            vec![
                "create:seatgeist_create:seatgeist_session",
                "select:/org/freedesktop/portal/desktop/session/1_42/seatgeist_session:seatgeist_select:3",
                "start:/org/freedesktop/portal/desktop/session/1_42/seatgeist_session:seatgeist_start:wayland:test-window",
            ]
        );
        Ok(())
    }

    #[test]
    fn connects_remote_desktop_session_to_eis_with_transport() -> Result<()> {
        let mut transport = MockRemoteDesktopTransport::success();
        let session = "/org/freedesktop/portal/desktop/session/1_42/seatgeist_session";
        let connection =
            connect_remote_desktop_eis(&mut transport, session, &PortalConnectToEisOptions)?;

        assert_eq!(connection.session_handle, session);
        assert_eq!(connection.fd, 42);
        assert_eq!(transport.calls, vec![format!("connect-eis:{session}")]);
        assert!(
            connect_remote_desktop_eis(
                &mut transport,
                "/org/freedesktop/portal/desktop/request/1_42/seatgeist_session",
                &PortalConnectToEisOptions,
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn stops_remote_desktop_lifecycle_when_select_is_cancelled() -> Result<()> {
        let mut transport = MockRemoteDesktopTransport::success();
        transport.responses[1].response = PortalResponseCode::Cancelled;
        let options = PortalRemoteDesktopOptions::new(
            "seatgeist_create",
            "seatgeist_session",
            "seatgeist_select",
            "seatgeist_start",
        );

        assert!(request_remote_desktop_session(&mut transport, &options)?.is_none());
        assert_eq!(transport.calls.len(), 2);
        assert_eq!(
            transport.waited_handles,
            vec![
                "/org/freedesktop/portal/desktop/request/1_42/seatgeist_create",
                "/org/freedesktop/portal/desktop/request/1_42/seatgeist_select",
            ]
        );
        Ok(())
    }

    #[test]
    fn maps_portal_response_codes_and_screenshot_uri() -> Result<()> {
        assert_eq!(
            PortalResponseCode::try_from(0)?,
            PortalResponseCode::Success
        );
        assert_eq!(
            PortalResponseCode::try_from(1)?,
            PortalResponseCode::Cancelled
        );
        assert_eq!(PortalResponseCode::try_from(2)?, PortalResponseCode::Other);
        assert!(matches!(
            PortalResponseCode::try_from(3),
            Err(PortalContractError::UnknownResponseCode(3))
        ));
        assert_eq!(
            parse_screenshot_uri(PortalResponseCode::Success, Some("file:///tmp/shot.png"))?,
            Some("file:///tmp/shot.png".to_string())
        );
        assert!(matches!(
            parse_screenshot_uri(PortalResponseCode::Success, None),
            Err(PortalContractError::MissingScreenshotUri)
        ));
        assert_eq!(
            parse_screenshot_uri(PortalResponseCode::Cancelled, Some("file:///tmp/shot.png"))?,
            None
        );
        Ok(())
    }

    #[test]
    fn parses_remote_desktop_session_response() -> Result<()> {
        let expected = "/org/freedesktop/portal/desktop/session/1_42/seatgeist_session";
        let session = parse_remote_desktop_session_handle(
            PortalResponseCode::Success,
            Some(expected),
            expected,
        )?
        .expect("session is returned");
        assert_eq!(session.expected_session_path, expected);
        assert_eq!(session.actual_session_path, expected);
        assert!(
            parse_remote_desktop_session_handle(PortalResponseCode::Cancelled, None, expected)?
                .is_none()
        );
        assert!(matches!(
            parse_remote_desktop_session_handle(PortalResponseCode::Success, None, expected),
            Err(PortalContractError::MissingSessionHandle)
        ));
        Ok(())
    }

    #[test]
    fn parses_remote_desktop_start_response() -> Result<()> {
        let start = parse_remote_desktop_start_response(
            PortalResponseCode::Success,
            Some(3),
            Some(true),
            Some("restore_next"),
        )?
        .expect("start response is returned");
        assert!(start.devices.contains(RemoteDesktopDeviceTypes::KEYBOARD));
        assert!(start.devices.contains(RemoteDesktopDeviceTypes::POINTER));
        assert!(start.clipboard_enabled);
        assert_eq!(start.restore_token.as_deref(), Some("restore_next"));
        assert!(
            parse_remote_desktop_start_response(PortalResponseCode::Other, None, None, None)?
                .is_none()
        );
        assert!(matches!(
            parse_remote_desktop_start_response(PortalResponseCode::Success, Some(8), None, None),
            Err(PortalContractError::UnknownRemoteDesktopDeviceTypes(8))
        ));
        Ok(())
    }

    #[test]
    fn request_screenshot_runs_lifecycle_and_decodes_result() -> Result<()> {
        let mut transport = MockScreenshotTransport::success();
        let options = PortalScreenshotOptions::new("seatgeist_abc");

        let capture = request_screenshot(&mut transport, &options)?.expect("capture succeeds");

        assert_eq!(
            capture.expected_handle_path,
            "/org/freedesktop/portal/desktop/request/1_42/seatgeist_abc"
        );
        assert_eq!(capture.actual_handle_path, capture.expected_handle_path);
        assert_eq!(
            capture.uri,
            "file:///run/user/1000/doc/abc/screen%20shot.png"
        );
        assert_eq!(
            capture.path,
            PathBuf::from("/run/user/1000/doc/abc/screen shot.png")
        );
        assert_eq!(transport.called_options.as_ref(), Some(&options));
        assert_eq!(
            transport.waited_handle.as_deref(),
            Some("/org/freedesktop/portal/desktop/request/1_42/seatgeist_abc")
        );
        Ok(())
    }

    #[test]
    fn request_screenshot_uses_returned_handle_when_portal_differs_from_expected() -> Result<()> {
        let mut transport = MockScreenshotTransport::success();
        transport.returned_handle =
            "/org/freedesktop/portal/desktop/request/compat/seatgeist_abc".to_string();

        let capture = request_screenshot(
            &mut transport,
            &PortalScreenshotOptions::new("seatgeist_abc"),
        )?
        .expect("capture succeeds");

        assert_eq!(
            capture.expected_handle_path,
            "/org/freedesktop/portal/desktop/request/1_42/seatgeist_abc"
        );
        assert_eq!(capture.actual_handle_path, transport.returned_handle);
        assert_eq!(
            transport.waited_handle.as_deref(),
            Some("/org/freedesktop/portal/desktop/request/compat/seatgeist_abc")
        );
        Ok(())
    }

    #[test]
    fn request_screenshot_returns_none_when_cancelled() -> Result<()> {
        let mut transport = MockScreenshotTransport::success();
        transport.response = PortalRequestResponse {
            response: PortalResponseCode::Cancelled,
            uri: None,
        };

        assert_eq!(
            request_screenshot(
                &mut transport,
                &PortalScreenshotOptions::new("seatgeist_abc")
            )?,
            None
        );
        Ok(())
    }

    #[test]
    fn request_screenshot_rejects_invalid_returned_handle() {
        let mut transport = MockScreenshotTransport::success();
        transport.returned_handle = "/invalid/request/path".to_string();

        assert!(matches!(
            request_screenshot(
                &mut transport,
                &PortalScreenshotOptions::new("seatgeist_abc")
            ),
            Err(PortalContractError::Transport(_))
        ));
    }

    #[test]
    fn decodes_file_uris_for_portal_results() -> Result<()> {
        assert_eq!(
            file_uri_to_path("file:///run/user/1000/doc/abc/screen%20shot.png")?,
            PathBuf::from("/run/user/1000/doc/abc/screen shot.png")
        );
        assert_eq!(
            file_uri_to_path("file:///tmp/a%2Bb.png")?,
            PathBuf::from("/tmp/a+b.png")
        );
        assert!(matches!(
            file_uri_to_path("https://example.invalid/shot.png"),
            Err(PortalContractError::UnsupportedUri(_))
        ));
        assert!(matches!(
            file_uri_to_path("file://relative/path.png"),
            Err(PortalContractError::UnsupportedUri(_))
        ));
        assert!(matches!(
            file_uri_to_path("file:///tmp/%xx.png"),
            Err(PortalContractError::InvalidPercentEncoding(_))
        ));
        Ok(())
    }

    #[test]
    fn builds_request_response_match_rule() {
        let rule = request_response_match_rule(
            "/org/freedesktop/portal/desktop/request/1_42/seatgeist_abc",
        );
        assert!(rule.contains("sender='org.freedesktop.portal.Desktop'"));
        assert!(rule.contains("interface='org.freedesktop.portal.Request'"));
        assert!(rule.contains("member='Response'"));
    }
}
