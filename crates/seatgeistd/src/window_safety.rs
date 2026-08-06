use anyhow::{Context, Result, bail};
use libseatgeist::DaemonRequest;
use seatgeist_backend::WindowBackend;

use crate::observation::active_window_revision;
use crate::{
    AppPolicy, active_window_guard_for_request, interaction_session_id_for_request,
    is_control_safety_class, safety_class_for_request, target_window_guard_for_request,
};

pub(crate) async fn enforce_app_policy(
    window_backend: &dyn WindowBackend,
    app_policy: &AppPolicy,
    request: &DaemonRequest,
) -> Result<()> {
    if app_policy.allow.is_empty() && app_policy.deny.is_empty() {
        return Ok(());
    }
    if !is_control_safety_class(&safety_class_for_request(request)) {
        return Ok(());
    }
    if matches!(
        request,
        DaemonRequest::RemoteDesktopSessionProbe(_)
            | DaemonRequest::RemoteDesktopEisProbe(_)
            | DaemonRequest::RemoteDesktopEisStart(_)
    ) {
        // These requests may show a portal consent dialog or retain an input
        // transport, but they never direct an action at an application. Every
        // later keyboard/pointer action still passes this app-policy gate.
        return Ok(());
    }
    if target_window_guard_for_request(request).is_some() {
        // High-level semantic operations authorize the resolved destination
        // after AT-SPI resolution and before the side effect.
        return Ok(());
    }
    if interaction_session_id_for_request(request).is_some() {
        // Sticky raw actions authorize the re-resolved pinned destination
        // immediately before their focus lease and input side effect.
        return Ok(());
    }

    if let DaemonRequest::FocusWindow(request) = request {
        return enforce_explicit_window_app_policy(
            window_backend,
            app_policy,
            &request.window_id,
            "focus target",
        )
        .await;
    }
    if let DaemonRequest::CloseWindow(request) = request {
        return enforce_explicit_window_app_policy(
            window_backend,
            app_policy,
            &request.window_id,
            "close target",
        )
        .await;
    }
    if let DaemonRequest::ResizeWindow(request) = request {
        return enforce_explicit_window_app_policy(
            window_backend,
            app_policy,
            &request.window_id,
            "resize target",
        )
        .await;
    }
    if let DaemonRequest::MoveWindow(request) = request {
        return enforce_explicit_window_app_policy(
            window_backend,
            app_policy,
            &request.window_id,
            "move target",
        )
        .await;
    }
    if let DaemonRequest::LaunchWindow(request) = request {
        let desktop_entry = request
            .desktop_entry
            .trim()
            .strip_suffix(".desktop")
            .unwrap_or(request.desktop_entry.trim());
        return enforce_app_policy_for_app(app_policy, Some(desktop_entry), "launch target");
    }

    let window = window_backend
        .active_window()
        .await
        .map_err(anyhow::Error::msg)
        .context("app policy could not read active window")?
        .ok_or_else(|| anyhow::anyhow!("app policy requires an active window for control"))?;
    enforce_app_policy_for_app(app_policy, window.app_id.as_deref(), "active window")
}

async fn enforce_explicit_window_app_policy(
    window_backend: &dyn WindowBackend,
    app_policy: &AppPolicy,
    window_id: &str,
    context: &str,
) -> Result<()> {
    let windows = window_backend
        .list_windows()
        .await
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("app policy could not list {context}s"))?;
    let target = windows
        .iter()
        .find(|window| window.id == window_id)
        .ok_or_else(
            || anyhow::anyhow!("app policy could not find {context} window {window_id}",),
        )?;
    enforce_app_policy_for_app(app_policy, target.app_id.as_deref(), context)
}

pub(crate) fn enforce_app_policy_for_app(
    app_policy: &AppPolicy,
    app_id: Option<&str>,
    context: &str,
) -> Result<()> {
    let app_id = app_id
        .map(str::trim)
        .filter(|app_id| !app_id.is_empty())
        .ok_or_else(|| anyhow::anyhow!("app policy could not determine {context} app id"))?;

    if app_policy
        .deny
        .iter()
        .any(|denied| app_id_matches(denied, app_id))
    {
        bail!(
            "app policy denied control of protected application {app_id} for {context}; \
             do not retry with focus, keyboard, pointer, accessibility, capture targeting, \
             or another backend"
        );
    }
    if !app_policy.allow.is_empty()
        && !app_policy
            .allow
            .iter()
            .any(|allowed| app_id_matches(allowed, app_id))
    {
        bail!(
            "app policy did not allow control of application {app_id} for {context}; \
             do not retry through another backend"
        );
    }
    Ok(())
}

pub(crate) fn app_id_matches(policy_value: &str, app_id: &str) -> bool {
    policy_value.eq_ignore_ascii_case(app_id)
}

pub(crate) async fn enforce_active_window_guard(
    window_backend: &dyn WindowBackend,
    request: &DaemonRequest,
) -> Result<()> {
    if interaction_session_id_for_request(request).is_some() {
        return Ok(());
    }
    let Some(guard) = active_window_guard_for_request(request) else {
        return Ok(());
    };
    let window = window_backend
        .active_window()
        .await
        .map_err(anyhow::Error::msg)
        .context("active-window guard could not read active window")?
        .ok_or_else(|| anyhow::anyhow!("active-window guard failed: no active window"))?;

    if let Some(expected) = &guard.desktop_revision {
        let actual = active_window_revision(&Some(window.clone()));
        if actual != *expected {
            bail!("active-window guard failed: desktop revision changed");
        }
    }

    if let Some(expected) = &guard.expected_window_id
        && window.id != *expected
    {
        bail!(
            "active-window guard failed: expected window id {}, got {}",
            expected,
            window.id
        );
    }
    if let Some(expected) = &guard.expected_app_id
        && window.app_id.as_deref() != Some(expected.as_str())
    {
        bail!(
            "active-window guard failed: expected app id {}, got {}",
            expected,
            window.app_id.as_deref().unwrap_or("")
        );
    }
    if let Some(expected) = &guard.title_contains {
        let title = window.title.to_ascii_lowercase();
        let expected = expected.to_ascii_lowercase();
        if !title.contains(&expected) {
            bail!(
                "active-window guard failed: expected title containing {}, got {}",
                guard.title_contains.as_deref().unwrap_or(""),
                window.title
            );
        }
    }
    Ok(())
}
