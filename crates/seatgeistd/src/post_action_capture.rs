use anyhow::{Result, bail};
use libseatgeist::{ActionResult, CaptureSnapshotRequest, DaemonRequest, PostActionImageOptions};

pub(crate) async fn validate(
    request: &DaemonRequest,
    image: &PostActionImageOptions,
    runtime: &super::DaemonRuntime,
) -> Result<()> {
    runtime
        .capture_session_store
        .require_active(&image.session_id)
        .await?;

    let interaction = runtime
        .interaction_session_store
        .status(&image.session_id)
        .await;
    validate_target_binding(request, &image.session_id, &interaction)?;
    let windows = runtime
        .window_backend
        .list_windows()
        .await
        .map_err(anyhow::Error::msg)?;
    let pinned = runtime
        .interaction_session_store
        .resolve(&image.session_id, &windows)
        .await?;
    super::enforce_app_policy_for_app(
        &runtime.app_policy,
        pinned.window.app_id.as_deref(),
        "post-action capture target",
    )?;
    Ok(())
}

fn validate_target_binding(
    request: &DaemonRequest,
    image_session_id: &str,
    interaction: &super::interaction::InteractionStatus,
) -> Result<()> {
    if !interaction.bound || interaction.session_id.as_deref() != Some(image_session_id) {
        bail!("post-action image requires a live pinned target for the named capture session");
    }
    let expected_window_id = if super::interaction_session_id_for_request(request)
        == Some(image_session_id)
    {
        interaction.window_id.as_deref()
    } else {
        expected_action_window_id(request)
    }
    .ok_or_else(|| {
        anyhow::anyhow!(
            "post-action image requires a sticky, target-window, focus, or exact active-window guard"
        )
    })?;
    if interaction.window_id.as_deref() != Some(expected_window_id) {
        bail!("post-action image capture session does not match the action target window");
    }
    Ok(())
}

pub(crate) async fn attach(
    image: &PostActionImageOptions,
    action: &mut ActionResult,
    runtime: &super::DaemonRuntime,
) {
    let interaction = runtime
        .interaction_session_store
        .status(&image.session_id)
        .await;
    let target_window_id = interaction.window_id.as_deref();
    if runtime
        .journal
        .record_post_action_capture_step(
            "interaction_post_action_capture_start",
            &image.session_id,
            action.id,
            target_window_id,
            true,
        )
        .is_err()
    {
        mark_unavailable(action);
        return;
    }
    let result = runtime
        .capture_session_store
        .snapshot(CaptureSnapshotRequest {
            session_id: image.session_id.clone(),
            output: image.output.clone(),
            max_edge: image.max_edge,
            timeout_ms: image.timeout_ms,
        })
        .await;
    let _ = runtime.journal.record_post_action_capture_step(
        "interaction_post_action_capture_finish",
        &image.session_id,
        action.id,
        target_window_id,
        result.is_ok(),
    );
    match result {
        Ok(frame) => {
            if let Some(observation) = action.observation.as_mut() {
                observation.screenshot_path = Some(frame.screenshot.path.display().to_string());
                observation.revision = Some(frame.revision);
            }
            action.screenshot = Some(frame.screenshot);
        }
        Err(_) => mark_unavailable(action),
    }
}

fn mark_unavailable(action: &mut ActionResult) {
    if let Some(observation) = action.observation.as_mut() {
        observation
            .issues
            .push("post_action_image_unavailable".to_string());
    }
    let message = action.message.take().unwrap_or_default();
    action.message = Some(format!("{message} post_action_image=unavailable"));
}

fn expected_action_window_id(request: &DaemonRequest) -> Option<&str> {
    super::target_window_guard_for_request(request)
        .map(|guard| guard.expected_window_id.as_str())
        .or_else(|| match request {
            DaemonRequest::FocusWindow(request) => Some(request.window_id.as_str()),
            DaemonRequest::ResizeWindow(request) => Some(request.window_id.as_str()),
            _ => super::active_window_guard_for_request(request)
                .and_then(|guard| guard.expected_window_id.as_deref()),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use libseatgeist::{TargetWindowGuard, TypeTextRequest};

    #[test]
    fn extracts_only_explicit_action_destinations() {
        let raw = DaemonRequest::TypeText(TypeTextRequest {
            text: "x".to_string(),
            guard: None,
            session_id: Some("capture-1".to_string()),
        });
        assert_eq!(expected_action_window_id(&raw), None);

        let semantic = DaemonRequest::ClickButton(libseatgeist::ClickButtonRequest {
            name: "Continue".to_string(),
            destructive: false,
            app: None,
            window_name_contains: None,
            max_nodes: 64,
            guard: None,
            target_guard: Some(TargetWindowGuard {
                expected_window_id: "window-1".to_string(),
                expected_app_id: None,
                expected_pid: None,
                title_contains: None,
            }),
        });
        assert_eq!(expected_action_window_id(&semantic), Some("window-1"));
    }

    #[test]
    fn image_session_must_match_the_explicit_action_destination() {
        let interaction = super::super::interaction::InteractionStatus {
            bound: true,
            session_id: Some("capture-1".to_string()),
            window_id: Some("window-1".to_string()),
            app_id: Some("org.mozilla.firefox".to_string()),
            pid: Some(42),
            expires_in_ms: Some(60_000),
            owner_tool: Some("seatgeist-mcp".to_string()),
            owner_pid: Some(7),
            owner_scope: Some("process".to_string()),
        };
        let raw = DaemonRequest::TypeText(TypeTextRequest {
            text: "x".to_string(),
            guard: None,
            session_id: Some("capture-1".to_string()),
        });
        validate_target_binding(&raw, "capture-1", &interaction)
            .expect("sticky raw action matches its pinned capture session");

        let wrong_target = DaemonRequest::FocusWindow(libseatgeist::FocusWindowRequest {
            window_id: "window-2".to_string(),
            guard: None,
        });
        let error = validate_target_binding(&wrong_target, "capture-1", &interaction)
            .expect_err("unrelated action target is rejected");
        assert!(error.to_string().contains("does not match"));

        let error = validate_target_binding(&raw, "capture-other", &interaction)
            .expect_err("unrelated session id is rejected");
        assert!(error.to_string().contains("live pinned target"));
    }
}
