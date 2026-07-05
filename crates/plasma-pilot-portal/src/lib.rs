use std::collections::HashMap;
use std::fmt;
use std::future::poll_fn;
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
                returned_handle: "/org/freedesktop/portal/desktop/request/1_42/plasma_pilot_abc"
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

    #[test]
    fn validates_handle_tokens_as_object_path_elements() {
        assert!(validate_handle_token("plasma_pilot_123").is_ok());
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
        let options = PortalScreenshotOptions::new("plasma_pilot_abc");
        assert_eq!(options.target, None);
        assert_eq!(options.vardict_entry_count(), 3);
    }

    #[test]
    fn validates_request_handle_paths() {
        assert!(
            validate_request_path("/org/freedesktop/portal/desktop/request/1_42/plasma_pilot_abc")
                .is_ok()
        );
        assert!(matches!(
            validate_request_path("/org/freedesktop/portal/desktop/request/1-42/plasma_pilot_abc"),
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
            validate_session_path("/org/freedesktop/portal/desktop/session/1_42/plasma_pilot_abc")
                .is_ok()
        );
        assert!(matches!(
            validate_session_path("/org/freedesktop/portal/desktop/session/1-42/plasma_pilot_abc"),
            Err(PortalContractError::InvalidHandleToken(_))
        ));
        assert!(matches!(
            validate_session_path("/org/freedesktop/portal/desktop/request/1_42/plasma_pilot_abc"),
            Err(PortalContractError::Transport(_))
        ));
    }

    #[test]
    fn builds_expected_request_path_from_sender_and_token() -> Result<()> {
        assert_eq!(
            expected_request_path(":1.42", "plasma_pilot_abc")?,
            "/org/freedesktop/portal/desktop/request/1_42/plasma_pilot_abc"
        );
        assert!(matches!(
            expected_request_path("1.42", "plasma_pilot_abc"),
            Err(PortalContractError::InvalidSenderName(_))
        ));
        Ok(())
    }

    #[test]
    fn builds_expected_session_path_from_sender_and_token() -> Result<()> {
        assert_eq!(
            expected_session_path(":1.42", "plasma_pilot_session")?,
            "/org/freedesktop/portal/desktop/session/1_42/plasma_pilot_session"
        );
        assert!(matches!(
            expected_session_path("1.42", "plasma_pilot_session"),
            Err(PortalContractError::InvalidSenderName(_))
        ));
        Ok(())
    }

    #[test]
    fn builds_screenshot_busctl_call_with_vardict_options() -> Result<()> {
        let mut options = PortalScreenshotOptions::new("plasma_pilot_abc");
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
                "plasma_pilot_abc",
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
        let options =
            PortalCreateSessionOptions::new("plasma_pilot_create", "plasma_pilot_session");
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
                "plasma_pilot_create",
                "session_handle_token",
                "s",
                "plasma_pilot_session",
            ]
        );
        Ok(())
    }

    #[test]
    fn builds_remote_desktop_select_devices_busctl_call() -> Result<()> {
        let session = "/org/freedesktop/portal/desktop/session/1_42/plasma_pilot_session";
        let mut options = PortalSelectDevicesOptions::new("plasma_pilot_select");
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
                "plasma_pilot_select",
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
        let session = "/org/freedesktop/portal/desktop/session/1_42/plasma_pilot_session";
        let mut options = PortalStartOptions::new("plasma_pilot_start");
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
                "plasma_pilot_start",
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
        let expected = "/org/freedesktop/portal/desktop/session/1_42/plasma_pilot_session";
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
        let options = PortalScreenshotOptions::new("plasma_pilot_abc");

        let capture = request_screenshot(&mut transport, &options)?.expect("capture succeeds");

        assert_eq!(
            capture.expected_handle_path,
            "/org/freedesktop/portal/desktop/request/1_42/plasma_pilot_abc"
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
            Some("/org/freedesktop/portal/desktop/request/1_42/plasma_pilot_abc")
        );
        Ok(())
    }

    #[test]
    fn request_screenshot_uses_returned_handle_when_portal_differs_from_expected() -> Result<()> {
        let mut transport = MockScreenshotTransport::success();
        transport.returned_handle =
            "/org/freedesktop/portal/desktop/request/compat/plasma_pilot_abc".to_string();

        let capture = request_screenshot(
            &mut transport,
            &PortalScreenshotOptions::new("plasma_pilot_abc"),
        )?
        .expect("capture succeeds");

        assert_eq!(
            capture.expected_handle_path,
            "/org/freedesktop/portal/desktop/request/1_42/plasma_pilot_abc"
        );
        assert_eq!(capture.actual_handle_path, transport.returned_handle);
        assert_eq!(
            transport.waited_handle.as_deref(),
            Some("/org/freedesktop/portal/desktop/request/compat/plasma_pilot_abc")
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
                &PortalScreenshotOptions::new("plasma_pilot_abc")
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
                &PortalScreenshotOptions::new("plasma_pilot_abc")
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
            "/org/freedesktop/portal/desktop/request/1_42/plasma_pilot_abc",
        );
        assert!(rule.contains("sender='org.freedesktop.portal.Desktop'"));
        assert!(rule.contains("interface='org.freedesktop.portal.Request'"));
        assert!(rule.contains("member='Response'"));
    }
}
