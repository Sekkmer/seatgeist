use std::{
    io::Write,
    process::{Command, Stdio},
};

use anyhow::{Result, bail};
use async_trait::async_trait;
use libseatgeist::{
    ActionResult, ClipboardBackendStatus, ClipboardGetRequest, ClipboardText, SeatgeistError,
};
use seatgeist_backend::ClipboardBackend as ClipboardBackendTrait;
use uuid::Uuid;

use crate::commands::exists as command_exists;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackendKind {
    WlClipboard,
    KdeKlipper,
}

impl BackendKind {
    fn name(self) -> &'static str {
        match self {
            Self::WlClipboard => "wl-clipboard",
            Self::KdeKlipper => "kde-klipper",
        }
    }

    fn adapter(self) -> Box<dyn ClipboardBackendTrait> {
        match self {
            Self::WlClipboard => Box::new(WlClipboardBackend),
            Self::KdeKlipper => Box::new(KdeKlipperBackend),
        }
    }
}

#[derive(Debug)]
struct WlClipboardBackend;

#[async_trait]
impl ClipboardBackendTrait for WlClipboardBackend {
    async fn get_text(&self) -> seatgeist_backend::Result<Option<String>> {
        let output = Command::new("wl-paste")
            .arg("--no-newline")
            .output()
            .map_err(|err| SeatgeistError::Io(format!("run wl-paste clipboard backend: {err}")))?;
        if !output.status.success() {
            return Err(SeatgeistError::BackendUnavailable(format!(
                "wl-paste clipboard backend exited with status {}",
                output.status
            )));
        }
        String::from_utf8(output.stdout)
            .map(Some)
            .map_err(|err| SeatgeistError::Io(format!("clipboard text is not valid UTF-8: {err}")))
    }

    async fn set_text(&self, text: &str) -> seatgeist_backend::Result<()> {
        let mut child = Command::new("wl-copy")
            .arg("--type")
            .arg("text/plain;charset=utf-8")
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|err| SeatgeistError::Io(format!("start wl-copy clipboard backend: {err}")))?;
        child
            .stdin
            .as_mut()
            .ok_or_else(|| SeatgeistError::Io("wl-copy stdin is unavailable".to_string()))?
            .write_all(text.as_bytes())
            .map_err(|err| SeatgeistError::Io(format!("write text to wl-copy: {err}")))?;
        let status = child.wait().map_err(|err| {
            SeatgeistError::Io(format!("wait for wl-copy clipboard backend: {err}"))
        })?;
        if !status.success() {
            return Err(SeatgeistError::BackendUnavailable(format!(
                "wl-copy clipboard backend exited with status {status}"
            )));
        }
        Ok(())
    }
}

#[derive(Debug)]
struct KdeKlipperBackend;

#[async_trait]
impl ClipboardBackendTrait for KdeKlipperBackend {
    async fn get_text(&self) -> seatgeist_backend::Result<Option<String>> {
        let output = Command::new("qdbus6")
            .args([
                "org.kde.klipper",
                "/klipper",
                "org.kde.klipper.klipper.getClipboardContents",
            ])
            .output()
            .map_err(|err| {
                SeatgeistError::Io(format!("run KDE Klipper clipboard read backend: {err}"))
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(SeatgeistError::BackendUnavailable(format!(
                "KDE Klipper clipboard read backend exited with status {}: {stderr}",
                output.status
            )));
        }
        let mut text = String::from_utf8(output.stdout).map_err(|err| {
            SeatgeistError::Io(format!("KDE Klipper clipboard text is not UTF-8: {err}"))
        })?;
        if text.ends_with('\n') {
            text.pop();
        }
        Ok(Some(text))
    }

    async fn set_text(&self, text: &str) -> seatgeist_backend::Result<()> {
        let status = Command::new("qdbus6")
            .args([
                "org.kde.klipper",
                "/klipper",
                "org.kde.klipper.klipper.setClipboardContents",
                text,
            ])
            .status()
            .map_err(|err| {
                SeatgeistError::Io(format!("run KDE Klipper clipboard write backend: {err}"))
            })?;
        if !status.success() {
            return Err(SeatgeistError::BackendUnavailable(format!(
                "KDE Klipper clipboard write backend exited with status {status}"
            )));
        }
        Ok(())
    }
}

pub(super) async fn get_text(request: ClipboardGetRequest) -> Result<ClipboardText> {
    if request.max_bytes == Some(0) {
        bail!("clipboard max_bytes must be greater than zero");
    }
    let backend = read_backend()
        .ok_or_else(|| anyhow::anyhow!("no clipboard text read backend is available"))?;
    let text = backend
        .adapter()
        .get_text()
        .await
        .map_err(|err| anyhow::anyhow!(err))?
        .unwrap_or_default();
    Ok(bound_text(
        text,
        request.max_bytes,
        backend.name().to_string(),
    ))
}

pub(super) async fn set_text(text: &str) -> Result<ActionResult> {
    let backend = write_backend()
        .ok_or_else(|| anyhow::anyhow!("no clipboard text write backend is available"))?;
    backend
        .adapter()
        .set_text(text)
        .await
        .map_err(|err| anyhow::anyhow!(err))?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,
        screenshot: None,
        message: Some(format!(
            "set clipboard text length={} backend={}",
            text.len(),
            backend.name()
        )),
    })
}

pub(super) fn status() -> ClipboardBackendStatus {
    status_from_availability(
        command_exists("wl-paste"),
        command_exists("wl-copy"),
        kde_klipper_available(),
    )
}

pub(super) fn available() -> bool {
    read_backend().is_some() && write_backend().is_some()
}

fn status_from_availability(
    wl_paste_available: bool,
    wl_copy_available: bool,
    kde_klipper_available: bool,
) -> ClipboardBackendStatus {
    let read_backend = read_backend_from_availability(wl_paste_available, kde_klipper_available)
        .map(|backend| backend.name().to_string());
    let write_backend = write_backend_from_availability(wl_copy_available, kde_klipper_available)
        .map(|backend| backend.name().to_string());
    let setup_hint = setup_hint(
        read_backend.as_deref(),
        write_backend.as_deref(),
        wl_paste_available,
        wl_copy_available,
        kde_klipper_available,
    );
    ClipboardBackendStatus {
        wl_paste_available,
        wl_copy_available,
        kde_klipper_available,
        read_backend,
        write_backend,
        setup_hint,
    }
}

fn setup_hint(
    read_backend: Option<&str>,
    write_backend: Option<&str>,
    wl_paste_available: bool,
    wl_copy_available: bool,
    kde_klipper_available: bool,
) -> String {
    match (read_backend, write_backend) {
        (Some(read), Some(write)) if read == write => {
            format!("clipboard text read/write backends are available through {read}")
        }
        (Some(read), Some(write)) => {
            format!("clipboard text read backend={read} write backend={write}")
        }
        (Some(read), None) => format!(
            "clipboard text read backend={read}; install wl-copy or enable KDE Klipper DBus for writes"
        ),
        (None, Some(write)) => format!(
            "clipboard text write backend={write}; install wl-paste or enable KDE Klipper DBus for reads"
        ),
        (None, None) if !wl_paste_available && !wl_copy_available && !kde_klipper_available => {
            "no clipboard text backend is available; install wl-clipboard or run inside a KDE session with Klipper DBus".to_string()
        }
        (None, None) => {
            "clipboard backend probes are partially visible but no complete text read/write path is available".to_string()
        }
    }
}

fn bound_text(mut text: String, max_bytes: Option<usize>, backend: String) -> ClipboardText {
    let original_bytes = text.len();
    let Some(max_bytes) = max_bytes else {
        return ClipboardText {
            text,
            truncated: false,
            original_bytes,
            backend,
        };
    };
    if original_bytes <= max_bytes {
        return ClipboardText {
            text,
            truncated: false,
            original_bytes,
            backend,
        };
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    ClipboardText {
        text,
        truncated: true,
        original_bytes,
        backend,
    }
}

fn read_backend() -> Option<BackendKind> {
    read_backend_from_availability(command_exists("wl-paste"), kde_klipper_available())
}

fn write_backend() -> Option<BackendKind> {
    write_backend_from_availability(command_exists("wl-copy"), kde_klipper_available())
}

fn read_backend_from_availability(
    wl_paste_available: bool,
    kde_klipper_available: bool,
) -> Option<BackendKind> {
    if wl_paste_available {
        Some(BackendKind::WlClipboard)
    } else if kde_klipper_available {
        Some(BackendKind::KdeKlipper)
    } else {
        None
    }
}

fn write_backend_from_availability(
    wl_copy_available: bool,
    kde_klipper_available: bool,
) -> Option<BackendKind> {
    if wl_copy_available {
        Some(BackendKind::WlClipboard)
    } else if kde_klipper_available {
        Some(BackendKind::KdeKlipper)
    } else {
        None
    }
}

fn kde_klipper_available() -> bool {
    Command::new("qdbus6")
        .args(["org.kde.klipper", "/klipper"])
        .output()
        .is_ok_and(|output| output.status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concrete_adapters_implement_the_shared_trait() {
        fn assert_backend<T: ClipboardBackendTrait>() {}
        assert_backend::<WlClipboardBackend>();
        assert_backend::<KdeKlipperBackend>();
    }

    #[test]
    fn bounds_text_on_utf8_boundary_and_supports_unbounded_reads() {
        let bounded = bound_text("abécd".to_string(), Some(4), "test".to_string());
        assert_eq!(bounded.text, "abé");
        assert!(bounded.truncated);
        assert_eq!(bounded.original_bytes, 6);
        assert_eq!(bounded.backend, "test");

        let unbounded = bound_text("hello".to_string(), None, "test".to_string());
        assert_eq!(unbounded.text, "hello");
        assert!(!unbounded.truncated);
        assert_eq!(unbounded.original_bytes, 5);
    }

    #[test]
    fn selection_prefers_wayland_then_klipper() {
        assert_eq!(
            read_backend_from_availability(true, true),
            Some(BackendKind::WlClipboard)
        );
        assert_eq!(
            read_backend_from_availability(false, true),
            Some(BackendKind::KdeKlipper)
        );
        assert_eq!(read_backend_from_availability(false, false), None);
        assert_eq!(
            write_backend_from_availability(true, true),
            Some(BackendKind::WlClipboard)
        );
        assert_eq!(
            write_backend_from_availability(false, true),
            Some(BackendKind::KdeKlipper)
        );
        assert_eq!(write_backend_from_availability(false, false), None);
    }

    #[test]
    fn status_and_setup_hint_report_backend_shape() {
        let status = status_from_availability(true, false, true);
        assert_eq!(status.read_backend.as_deref(), Some("wl-clipboard"));
        assert_eq!(status.write_backend.as_deref(), Some("kde-klipper"));
        assert!(status.setup_hint.contains("read backend=wl-clipboard"));
        assert!(status.setup_hint.contains("write backend=kde-klipper"));

        let missing = status_from_availability(false, false, false);
        assert!(missing.setup_hint.contains("no clipboard text backend"));
        assert!(missing.setup_hint.contains("wl-clipboard"));
    }
}
