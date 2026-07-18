use std::{
    os::fd::{OwnedFd, RawFd},
    time::Duration,
};

use crate::{
    PortalContractError, PortalCreateSessionOptions, PortalRemoteDesktopSession,
    PortalResponseCode, PortalStartOptions, RemoteDesktopPersistMode, Result,
    close_screen_cast_session_on_connection, expected_session_path,
    parse_remote_desktop_session_handle, validate_handle_token, validate_request_path,
    validate_session_path, wait_for_session_closed_signal,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenCastSourceTypes(u32);

impl ScreenCastSourceTypes {
    pub const MONITOR: Self = Self(1);
    pub const WINDOW: Self = Self(2);
    pub const VIRTUAL: Self = Self(4);
    pub const ALL: Self = Self(Self::MONITOR.0 | Self::WINDOW.0 | Self::VIRTUAL.0);

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn validate(self) -> Result<()> {
        if self.0 != 0 && self.0 & !Self::ALL.0 == 0 {
            return Ok(());
        }
        Err(PortalContractError::UnknownScreenCastSourceTypes(self.0))
    }
}

impl TryFrom<u32> for ScreenCastSourceTypes {
    type Error = PortalContractError;

    fn try_from(value: u32) -> Result<Self> {
        let types = Self(value);
        types.validate()?;
        Ok(types)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenCastCursorMode {
    Hidden,
    Embedded,
    Metadata,
}

impl ScreenCastCursorMode {
    pub const fn value(self) -> u32 {
        match self {
            Self::Hidden => 1,
            Self::Embedded => 2,
            Self::Metadata => 4,
        }
    }
}

impl TryFrom<u32> for ScreenCastCursorMode {
    type Error = PortalContractError;

    fn try_from(value: u32) -> Result<Self> {
        match value {
            1 => Ok(Self::Hidden),
            2 => Ok(Self::Embedded),
            4 => Ok(Self::Metadata),
            other => Err(PortalContractError::UnknownScreenCastCursorMode(other)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalSelectSourcesOptions {
    pub handle_token: String,
    pub types: ScreenCastSourceTypes,
    pub multiple: bool,
    pub cursor_mode: ScreenCastCursorMode,
    pub restore_token: Option<String>,
    pub persist_mode: RemoteDesktopPersistMode,
}

impl PortalSelectSourcesOptions {
    pub fn new(handle_token: impl Into<String>, types: ScreenCastSourceTypes) -> Self {
        Self {
            handle_token: handle_token.into(),
            types,
            multiple: false,
            cursor_mode: ScreenCastCursorMode::Hidden,
            restore_token: None,
            persist_mode: RemoteDesktopPersistMode::DoNotPersist,
        }
    }

    pub fn validate(&self) -> Result<()> {
        validate_handle_token(&self.handle_token)?;
        self.types.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalScreenCastOptions {
    pub create_session: PortalCreateSessionOptions,
    pub select_sources: PortalSelectSourcesOptions,
    pub start: PortalStartOptions,
}

impl PortalScreenCastOptions {
    pub fn new_for_source(
        create_handle_token: impl Into<String>,
        session_handle_token: impl Into<String>,
        select_handle_token: impl Into<String>,
        start_handle_token: impl Into<String>,
        source_types: ScreenCastSourceTypes,
    ) -> Self {
        Self {
            create_session: PortalCreateSessionOptions::new(
                create_handle_token,
                session_handle_token,
            ),
            select_sources: PortalSelectSourcesOptions::new(select_handle_token, source_types),
            start: PortalStartOptions::new(start_handle_token),
        }
    }

    pub fn new_window(
        create_handle_token: impl Into<String>,
        session_handle_token: impl Into<String>,
        select_handle_token: impl Into<String>,
        start_handle_token: impl Into<String>,
    ) -> Self {
        Self::new_for_source(
            create_handle_token,
            session_handle_token,
            select_handle_token,
            start_handle_token,
            ScreenCastSourceTypes::WINDOW,
        )
    }

    pub fn validate(&self) -> Result<()> {
        self.create_session.validate()?;
        self.select_sources.validate()?;
        self.start.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalScreenCastStream {
    pub node_id: u32,
    pub id: Option<String>,
    pub position: Option<(i32, i32)>,
    pub size: Option<(i32, i32)>,
    pub source_type: Option<ScreenCastSourceTypes>,
    pub mapping_id: Option<String>,
    pub pipewire_serial: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalScreenCastRequestResponse {
    pub response: PortalResponseCode,
    pub session_handle: Option<String>,
    pub streams: Option<Vec<PortalScreenCastStream>>,
    pub restore_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalScreenCastSessionStart {
    pub create_request_path: String,
    pub select_request_path: String,
    pub start_request_path: String,
    pub session: PortalRemoteDesktopSession,
    pub streams: Vec<PortalScreenCastStream>,
    pub restore_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalScreenCastPipeWireConnection {
    pub session_handle: String,
    pub fd: RawFd,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalScreenCastSessionConnection {
    pub session_start: PortalScreenCastSessionStart,
    pub pipewire: PortalScreenCastPipeWireConnection,
}

pub struct PortalScreenCastOwnedSession {
    pub session_start: PortalScreenCastSessionStart,
    pub(crate) pipewire_fd: Option<OwnedFd>,
    pub(crate) connection: zbus::Connection,
    pub(crate) closed_stream: zbus::proxy::SignalStream<'static>,
}

impl PortalScreenCastOwnedSession {
    pub fn take_pipewire_fd(&mut self) -> Result<OwnedFd> {
        self.pipewire_fd.take().ok_or_else(|| {
            PortalContractError::Transport(
                "ScreenCast PipeWire descriptor was already taken".to_string(),
            )
        })
    }

    pub async fn close(&self) -> Result<()> {
        close_screen_cast_session_on_connection(
            &self.connection,
            &self.session_start.session.actual_session_path,
        )
        .await
    }

    pub async fn wait_closed(&mut self, timeout: Duration) -> Result<bool> {
        wait_for_session_closed_signal(&mut self.closed_stream, timeout).await
    }
}

pub trait PortalScreenCastTransport {
    fn unique_sender_name(&mut self) -> Result<String>;
    fn call_create_session(&mut self, options: &PortalCreateSessionOptions) -> Result<String>;
    fn call_select_sources(
        &mut self,
        session_handle: &str,
        options: &PortalSelectSourcesOptions,
    ) -> Result<String>;
    fn call_start(&mut self, session_handle: &str, options: &PortalStartOptions) -> Result<String>;
    fn wait_for_response(&mut self, handle_path: &str) -> Result<PortalScreenCastRequestResponse>;
    fn call_open_pipewire_remote(&mut self, session_handle: &str) -> Result<RawFd>;
    fn close_session(&mut self, session_handle: &str) -> Result<()>;
}

pub fn request_screen_cast_session<T>(
    transport: &mut T,
    options: &PortalScreenCastOptions,
) -> Result<Option<PortalScreenCastSessionStart>>
where
    T: PortalScreenCastTransport,
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

    let result = (|| {
        let select_request_path =
            transport.call_select_sources(&session.actual_session_path, &options.select_sources)?;
        validate_request_path(&select_request_path)?;
        let select_response = transport.wait_for_response(&select_request_path)?;
        if select_response.response != PortalResponseCode::Success {
            return Ok(None);
        }

        let start_request_path =
            transport.call_start(&session.actual_session_path, &options.start)?;
        validate_request_path(&start_request_path)?;
        let start_response = transport.wait_for_response(&start_request_path)?;
        let Some((streams, restore_token)) = parse_screen_cast_start_response(start_response)?
        else {
            return Ok(None);
        };

        Ok(Some(PortalScreenCastSessionStart {
            create_request_path,
            select_request_path,
            start_request_path,
            session: session.clone(),
            streams,
            restore_token,
        }))
    })();

    if !matches!(result, Ok(Some(_))) {
        let _ = transport.close_session(&session.actual_session_path);
    }
    result
}

pub fn request_screen_cast_pipewire<T>(
    transport: &mut T,
    options: &PortalScreenCastOptions,
) -> Result<Option<PortalScreenCastSessionConnection>>
where
    T: PortalScreenCastTransport,
{
    let Some(session_start) = request_screen_cast_session(transport, options)? else {
        return Ok(None);
    };
    let session_handle = session_start.session.actual_session_path.clone();
    match open_screen_cast_pipewire_remote(transport, &session_handle) {
        Ok(pipewire) => Ok(Some(PortalScreenCastSessionConnection {
            session_start,
            pipewire,
        })),
        Err(err) => {
            let _ = transport.close_session(&session_handle);
            Err(err)
        }
    }
}

pub fn open_screen_cast_pipewire_remote<T>(
    transport: &mut T,
    session_handle: &str,
) -> Result<PortalScreenCastPipeWireConnection>
where
    T: PortalScreenCastTransport,
{
    validate_session_path(session_handle)?;
    let fd = transport.call_open_pipewire_remote(session_handle)?;
    if fd < 0 {
        return Err(PortalContractError::InvalidFileDescriptor(fd));
    }
    Ok(PortalScreenCastPipeWireConnection {
        session_handle: session_handle.to_string(),
        fd,
    })
}

pub(crate) fn parse_screen_cast_start_response(
    response: PortalScreenCastRequestResponse,
) -> Result<Option<(Vec<PortalScreenCastStream>, Option<String>)>> {
    match response.response {
        PortalResponseCode::Success => {
            let streams = response
                .streams
                .filter(|streams| !streams.is_empty())
                .ok_or(PortalContractError::MissingScreenCastStreams)?;
            Ok(Some((streams, response.restore_token)))
        }
        PortalResponseCode::Cancelled | PortalResponseCode::Other => Ok(None),
    }
}
