use std::fmt;
use std::path::PathBuf;

pub const DESKTOP_BUS_NAME: &str = "org.freedesktop.portal.Desktop";
pub const DESKTOP_OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";
pub const SCREENSHOT_INTERFACE: &str = "org.freedesktop.portal.Screenshot";
pub const REQUEST_INTERFACE: &str = "org.freedesktop.portal.Request";
pub const SCREENSHOT_METHOD: &str = "Screenshot";
pub const RESPONSE_SIGNAL: &str = "Response";
pub const REQUEST_PATH_PREFIX: &str = "/org/freedesktop/portal/desktop/request";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortalContractError {
    EmptyHandleToken,
    InvalidHandleToken(String),
    InvalidSenderName(String),
    UnknownScreenshotTarget(u32),
    UnknownResponseCode(u32),
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
            Self::UnknownScreenshotTarget(target) => {
                write!(
                    formatter,
                    "unknown portal screenshot target value: {target}"
                )
            }
            Self::UnknownResponseCode(code) => {
                write!(formatter, "unknown portal request response code: {code}")
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
            target: Some(PortalScreenshotTarget::Screen),
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
