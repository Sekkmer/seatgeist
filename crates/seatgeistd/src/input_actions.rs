use anyhow::{Result, bail};
use libseatgeist::{
    ActionResult, ClickPointerRequest, CoordinateSpace, DragPointerRequest, KeyComboRequest,
    MovePointerRequest, PageZoomOperation, PageZoomRequest, Point, ScrollPointerRequest,
    TypeTextRequest, WindowInfo,
};
use seatgeist_backend::{
    ScreenBackend, TargetedInputBackend, TargetedInputContext, TargetedInputDelivery, WindowBackend,
};
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

pub(crate) async fn agent_type_text(
    request: TypeTextRequest,
    context: &TargetedInputContext,
    target: &WindowInfo,
    backend: &dyn TargetedInputBackend,
) -> Result<ActionResult> {
    validate_text(&request.text)?;
    let chords = {
        let keymap =
            seatgeist_eis::XkbKeymap::new_from_names(seatgeist_eis::XkbKeymapNames::us_pc105())
                .map_err(|error| anyhow::anyhow!(error))?;
        request
            .text
            .chars()
            .map(|character| {
                let keysym = seatgeist_eis::unicode_char_to_keysym(character)
                    .map_err(|error| anyhow::anyhow!(error))?;
                let stroke = keymap.keystroke_for_keysym(keysym).ok_or_else(|| {
                    anyhow::anyhow!(
                        "character {character:?} is not available on the agent-seat US keymap"
                    )
                })?;
                Ok(if stroke.shift {
                    vec![42, stroke.evdev_keycode]
                } else {
                    vec![stroke.evdev_keycode]
                })
            })
            .collect::<Result<Vec<_>>>()?
    };
    let char_count = chords.len();
    let delivery = backend
        .key_sequence(context, target, &chords)
        .await
        .map_err(anyhow::Error::msg)?;
    Ok(agent_result(
        delivery,
        format!(
            "typed text length={char_count} target_window={} routing=independent keymap=us",
            target.id
        ),
    ))
}

pub(crate) async fn agent_key_combo(
    request: KeyComboRequest,
    context: &TargetedInputContext,
    target: &WindowInfo,
    backend: &dyn TargetedInputBackend,
) -> Result<ActionResult> {
    validate_combo(&request.combo)?;
    let settings = XkbKeymapSettings {
        model: Some("pc105".to_string()),
        layout: Some("us".to_string()),
        options: Some(String::new()),
        ..XkbKeymapSettings::default()
    };
    let codes = crate::eis_key_combo::codes(&request.combo, &settings)?;
    let key_count = codes.len();
    let delivery = backend
        .key_combo(context, target, &codes)
        .await
        .map_err(anyhow::Error::msg)?;
    Ok(agent_result(
        delivery,
        format!(
            "sent key combo keys={key_count} target_window={} routing=independent",
            target.id
        ),
    ))
}

pub(crate) async fn agent_move_pointer(
    request: MovePointerRequest,
    context: &TargetedInputContext,
    target: &WindowInfo,
    backend: &dyn TargetedInputBackend,
) -> Result<ActionResult> {
    validate_agent_point(target, request.point)?;
    let point = request.point;
    let delivery = backend
        .move_pointer(context, target, point)
        .await
        .map_err(anyhow::Error::msg)?;
    Ok(agent_result(
        delivery,
        format!(
            "moved pointer x={:.0} y={:.0} space=WindowLocal target_window={} routing=independent",
            point.x, point.y, target.id
        ),
    ))
}

pub(crate) async fn agent_click_pointer(
    request: ClickPointerRequest,
    context: &TargetedInputContext,
    target: &WindowInfo,
    backend: &dyn TargetedInputBackend,
) -> Result<ActionResult> {
    validate_clicks(request.clicks)?;
    validate_agent_point(target, request.point)?;
    let point = request.point;
    let delivery = backend
        .click(context, target, point, request.button, request.clicks)
        .await
        .map_err(anyhow::Error::msg)?;
    Ok(agent_result(
        delivery,
        format!(
            "clicked pointer button={:?} clicks={} x={:.0} y={:.0} space=WindowLocal target_window={} routing=independent",
            request.button, request.clicks, point.x, point.y, target.id
        ),
    ))
}

pub(crate) async fn agent_drag_pointer(
    request: DragPointerRequest,
    context: &TargetedInputContext,
    target: &WindowInfo,
    backend: &dyn TargetedInputBackend,
) -> Result<ActionResult> {
    validate_drag(request.duration_ms, request.from.space, request.to.space)?;
    validate_agent_point(target, request.from)?;
    validate_agent_point(target, request.to)?;
    let delivery = backend
        .drag(context, target, request.from, request.to, request.button)
        .await
        .map_err(anyhow::Error::msg)?;
    Ok(agent_result(
        delivery,
        format!(
            "dragged pointer button={:?} from={:.0},{:.0} to={:.0},{:.0} duration_ms={} space=WindowLocal target_window={} routing=independent",
            request.button,
            request.from.x,
            request.from.y,
            request.to.x,
            request.to.y,
            request.duration_ms,
            target.id
        ),
    ))
}

pub(crate) async fn agent_scroll_pointer(
    request: ScrollPointerRequest,
    context: &TargetedInputContext,
    target: &WindowInfo,
    backend: &dyn TargetedInputBackend,
) -> Result<ActionResult> {
    validate_scroll(request.vertical, request.horizontal)?;
    let delivery = backend
        .scroll(context, target, request.vertical, request.horizontal)
        .await
        .map_err(anyhow::Error::msg)?;
    Ok(agent_result(
        delivery,
        format!(
            "scrolled pointer vertical={} horizontal={} target_window={} routing=independent",
            request.vertical, request.horizontal, target.id
        ),
    ))
}

fn agent_result(delivery: TargetedInputDelivery, message: String) -> ActionResult {
    ActionResult {
        id: delivery.action_id,
        ok: true,
        observation: None,
        screenshot: None,
        message: Some(format!("{message} backend={}", delivery.backend)),
    }
}

fn validate_agent_point(target: &WindowInfo, point: Point) -> Result<()> {
    if point.space != CoordinateSpace::WindowLocal {
        bail!("kwin_agent_seat requires window_local pointer coordinates");
    }
    if !point.x.is_finite() || !point.y.is_finite() {
        bail!("agent-seat pointer coordinates must be finite");
    }
    let geometry = target
        .geometry
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("agent-seat target has no window geometry"))?;
    if point.x < 0.0
        || point.y < 0.0
        || point.x >= f64::from(geometry.width)
        || point.y >= f64::from(geometry.height)
    {
        bail!(
            "window_local pointer coordinate {},{} is outside target window {} {}x{}",
            point.x,
            point.y,
            target.id,
            geometry.width,
            geometry.height
        );
    }
    Ok(())
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
        InputBackendPreference::Auto
        | InputBackendPreference::KwinAgentSeat
        | InputBackendPreference::Uinput => XkbKeymapSettings::default(),
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
        InputBackendPreference::Auto
        | InputBackendPreference::KwinAgentSeat
        | InputBackendPreference::Uinput => XkbKeymapSettings::default(),
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
