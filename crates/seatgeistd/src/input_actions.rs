use anyhow::{Result, bail};
use libseatgeist::{
    ActionResult, ClickPointerRequest, DragPointerRequest, KeyComboRequest, MovePointerRequest,
    PageZoomOperation, PageZoomRequest, ScrollPointerRequest, TypeTextRequest,
};
use seatgeist_backend::{ScreenBackend, WindowBackend};
use uuid::Uuid;

use crate::{
    config::InputBackendPreference,
    input_execution,
    keymap::{Config as XkbKeymapConfig, Settings as XkbKeymapSettings},
    pointer_coordinates,
    portal_eis_session::PortalEisSessionStore,
};

pub(crate) fn type_text(
    request: TypeTextRequest,
    input_backend_preference: InputBackendPreference,
    portal_eis_session_store: &PortalEisSessionStore,
) -> Result<ActionResult> {
    validate_text(&request.text)?;
    let mut backend = input_execution::backend(input_backend_preference, portal_eis_session_store)?;
    backend.type_text(&request.text)?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,
        screenshot: None,
        message: Some(format!(
            "typed text length={} backend={}",
            request.text.chars().count(),
            backend.name()
        )),
    })
}

pub(crate) fn key_combo(
    request: KeyComboRequest,
    input_backend_preference: InputBackendPreference,
    xkb_keymap_config: &XkbKeymapConfig,
    portal_eis_session_store: &PortalEisSessionStore,
) -> Result<ActionResult> {
    validate_combo(&request.combo)?;
    let xkb_keymap_settings = match input_backend_preference {
        InputBackendPreference::PortalRemoteDesktop | InputBackendPreference::Libei => {
            crate::keymap::resolve(xkb_keymap_config).settings
        }
        InputBackendPreference::Auto | InputBackendPreference::Uinput => {
            XkbKeymapSettings::default()
        }
    };
    let mut backend = input_execution::backend_with_store(
        input_backend_preference,
        portal_eis_session_store,
        &xkb_keymap_settings,
    )?;
    let key_count = backend.key_combo(&request.combo)?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,
        screenshot: None,
        message: Some(format!(
            "sent key combo keys={key_count} backend={}",
            backend.name()
        )),
    })
}

pub(crate) async fn page_zoom(
    request: PageZoomRequest,
    window_backend: &dyn WindowBackend,
    input_backend_preference: InputBackendPreference,
    xkb_keymap_config: &XkbKeymapConfig,
    portal_eis_session_store: &PortalEisSessionStore,
) -> Result<ActionResult> {
    if request.guard.expected_window_id.is_none() {
        bail!("page zoom requires an expected active-window id guard");
    }
    if request.steps == 0 || request.steps > 20 {
        bail!("page zoom steps must be between 1 and 20");
    }
    let active = window_backend
        .active_window()
        .await
        .map_err(anyhow::Error::msg)?
        .ok_or_else(|| anyhow::anyhow!("page zoom requires an active browser window"))?;
    let app_id = active.app_id.as_deref().unwrap_or("");
    if !is_supported_browser_app(app_id) {
        bail!("page zoom supports Firefox and Chromium-family windows; active app is {app_id}");
    }

    let xkb_keymap_settings = match input_backend_preference {
        InputBackendPreference::PortalRemoteDesktop | InputBackendPreference::Libei => {
            crate::keymap::resolve(xkb_keymap_config).settings
        }
        InputBackendPreference::Auto | InputBackendPreference::Uinput => {
            XkbKeymapSettings::default()
        }
    };
    let mut backend = input_execution::backend_with_store(
        input_backend_preference,
        portal_eis_session_store,
        &xkb_keymap_settings,
    )?;
    let (combo, count) = match request.operation {
        PageZoomOperation::In => ("Ctrl+Equal", request.steps),
        PageZoomOperation::Out => ("Ctrl+Minus", request.steps),
        PageZoomOperation::Reset => ("Ctrl+0", 1),
    };
    for _ in 0..count {
        backend.key_combo(combo)?;
    }
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,
        screenshot: None,
        message: Some(format!(
            "page zoom operation={:?} steps={} backend={}",
            request.operation,
            count,
            backend.name()
        )),
    })
}

fn is_supported_browser_app(app_id: &str) -> bool {
    let app = app_id.trim().to_ascii_lowercase();
    ["firefox", "chromium", "chrome", "brave", "vivaldi"]
        .iter()
        .any(|browser| app.contains(browser))
}

pub(crate) async fn move_pointer(
    request: MovePointerRequest,
    window_backend: &dyn WindowBackend,
    screen_backend: &dyn ScreenBackend,
    input_backend_preference: InputBackendPreference,
    portal_eis_session_store: &PortalEisSessionStore,
) -> Result<ActionResult> {
    let context = pointer_coordinates::ResolutionContext::load(
        request.point.space,
        window_backend,
        screen_backend,
    )
    .await?;
    let point = context.resolve(request.point)?;
    let bounds = context.bounds();
    let mut backend = input_execution::backend(input_backend_preference, portal_eis_session_store)?;
    backend.move_pointer(point, bounds)?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,
        screenshot: None,
        message: Some(format!(
            "moved pointer x={:.0} y={:.0} space={:?} backend={}",
            point.x,
            point.y,
            request.point.space,
            backend.name()
        )),
    })
}

pub(crate) async fn click_pointer(
    request: ClickPointerRequest,
    window_backend: &dyn WindowBackend,
    screen_backend: &dyn ScreenBackend,
    input_backend_preference: InputBackendPreference,
    portal_eis_session_store: &PortalEisSessionStore,
) -> Result<ActionResult> {
    validate_clicks(request.clicks)?;
    let context = pointer_coordinates::ResolutionContext::load(
        request.point.space,
        window_backend,
        screen_backend,
    )
    .await?;
    let point = context.resolve(request.point)?;
    let bounds = context.bounds();
    let mut backend = input_execution::backend(input_backend_preference, portal_eis_session_store)?;
    backend.click_pointer(point, bounds, request.button, request.clicks)?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,
        screenshot: None,
        message: Some(format!(
            "clicked pointer button={:?} clicks={} x={:.0} y={:.0} space={:?} backend={}",
            request.button,
            request.clicks,
            point.x,
            point.y,
            request.point.space,
            backend.name()
        )),
    })
}

pub(crate) async fn drag_pointer(
    request: DragPointerRequest,
    window_backend: &dyn WindowBackend,
    screen_backend: &dyn ScreenBackend,
    input_backend_preference: InputBackendPreference,
    portal_eis_session_store: &PortalEisSessionStore,
) -> Result<ActionResult> {
    validate_drag(request.duration_ms, request.from.space, request.to.space)?;
    let context = pointer_coordinates::ResolutionContext::load(
        request.from.space,
        window_backend,
        screen_backend,
    )
    .await?;
    let from = context.resolve(request.from)?;
    let to = context.resolve(request.to)?;
    let bounds = context.bounds();
    let mut backend = input_execution::backend(input_backend_preference, portal_eis_session_store)?;
    backend.drag_pointer(from, to, bounds, request.button, request.duration_ms)?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,
        screenshot: None,
        message: Some(format!(
            "dragged pointer button={:?} from={:.0},{:.0} to={:.0},{:.0} duration_ms={} space={:?} backend={}",
            request.button,
            from.x,
            from.y,
            to.x,
            to.y,
            request.duration_ms,
            request.from.space,
            backend.name()
        )),
    })
}

pub(crate) async fn scroll_pointer(
    request: ScrollPointerRequest,
    screen_backend: &dyn ScreenBackend,
    input_backend_preference: InputBackendPreference,
    portal_eis_session_store: &PortalEisSessionStore,
) -> Result<ActionResult> {
    validate_scroll(request.vertical, request.horizontal)?;
    let bounds = pointer_coordinates::physical_bounds(screen_backend).await?;
    let mut backend = input_execution::backend(input_backend_preference, portal_eis_session_store)?;
    backend.scroll_pointer(request.vertical, request.horizontal, bounds)?;
    Ok(ActionResult {
        id: Uuid::new_v4(),
        ok: true,
        observation: None,
        screenshot: None,
        message: Some(format!(
            "scrolled pointer vertical={} horizontal={} backend={}",
            request.vertical,
            request.horizontal,
            backend.name()
        )),
    })
}

fn validate_text(text: &str) -> Result<()> {
    if text.is_empty() {
        bail!("text must be non-empty");
    }
    if text.chars().count() > 8192 {
        bail!("text must be at most 8192 characters");
    }
    Ok(())
}

fn validate_combo(combo: &str) -> Result<()> {
    if combo.trim().is_empty() {
        bail!("combo must be non-empty");
    }
    Ok(())
}

fn validate_clicks(clicks: u8) -> Result<()> {
    if clicks == 0 || clicks > 2 {
        bail!("clicks must be 1 or 2");
    }
    Ok(())
}

fn validate_drag(
    duration_ms: u64,
    from_space: libseatgeist::CoordinateSpace,
    to_space: libseatgeist::CoordinateSpace,
) -> Result<()> {
    if duration_ms > 10_000 {
        bail!("duration_ms must be at most 10000");
    }
    if from_space != to_space {
        bail!(
            "drag pointer coordinates must use one coordinate space, got {from_space:?} and {to_space:?}"
        );
    }
    Ok(())
}

fn validate_scroll(vertical: i32, horizontal: i32) -> Result<()> {
    if vertical == 0 && horizontal == 0 {
        bail!("scroll request must include a non-zero delta");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use libseatgeist::CoordinateSpace;

    use super::{
        is_supported_browser_app, validate_clicks, validate_combo, validate_drag, validate_scroll,
        validate_text,
    };

    #[test]
    fn keyboard_bounds_fail_before_backend_selection() {
        assert!(validate_text("").is_err());
        assert!(validate_text(&"x".repeat(8193)).is_err());
        assert!(validate_text(&"x".repeat(8192)).is_ok());
        assert!(validate_combo("  ").is_err());
        assert!(validate_combo("Ctrl+L").is_ok());
    }

    #[test]
    fn pointer_bounds_fail_before_coordinate_or_backend_reads() {
        assert!(validate_clicks(0).is_err());
        assert!(validate_clicks(3).is_err());
        assert!(validate_clicks(1).is_ok());
        assert!(validate_clicks(2).is_ok());
        assert!(
            validate_drag(
                10_001,
                CoordinateSpace::PhysicalPixel,
                CoordinateSpace::PhysicalPixel
            )
            .is_err()
        );
        assert!(
            validate_drag(
                10_000,
                CoordinateSpace::PhysicalPixel,
                CoordinateSpace::LogicalPixel
            )
            .is_err()
        );
        assert!(
            validate_drag(
                10_000,
                CoordinateSpace::PhysicalPixel,
                CoordinateSpace::PhysicalPixel
            )
            .is_ok()
        );
        assert!(validate_scroll(0, 0).is_err());
        assert!(validate_scroll(1, 0).is_ok());
    }

    #[test]
    fn page_zoom_is_limited_to_known_browser_app_ids() {
        assert!(is_supported_browser_app("org.mozilla.firefox"));
        assert!(is_supported_browser_app("google-chrome"));
        assert!(is_supported_browser_app("com.brave.Browser"));
        assert!(!is_supported_browser_app("org.kde.kate"));
    }
}
