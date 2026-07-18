use std::time::{Duration, Instant};

use anyhow::Result;
use libseatgeist::{
    AccessibilityNode, ActionSettleBackend, ActionSettleCondition, ActionSettleResult, Observation,
    PostActionOptions, WindowInfo,
};
use seatgeist_backend::{
    AccessibilityEvent, AccessibilityEventBackend, AccessibilityEventSubscription,
    AccessibilityEventTarget,
};
use sha2::{Digest, Sha256};

pub(crate) struct PreparedSemanticSettle {
    subscription: Option<Box<dyn AccessibilityEventSubscription>>,
    target_window: WindowInfo,
    target_node_id: String,
    before_revision: String,
    condition: ActionSettleCondition,
    timeout_ms: u64,
    interval_ms: u64,
    issues: Vec<String>,
}

pub(crate) async fn prepare(
    target: AccessibilityEventTarget,
    target_window: WindowInfo,
    options: Option<&PostActionOptions>,
) -> Result<Option<PreparedSemanticSettle>> {
    prepare_with_backend(
        &seatgeist_atspi::AtspiEventBackend,
        target,
        target_window,
        options,
    )
    .await
}

async fn prepare_with_backend(
    backend: &dyn AccessibilityEventBackend,
    target: AccessibilityEventTarget,
    target_window: WindowInfo,
    options: Option<&PostActionOptions>,
) -> Result<Option<PreparedSemanticSettle>> {
    let Some(options) = options.filter(|options| options.observe_after) else {
        return Ok(None);
    };
    let condition = match options.settle_condition {
        ActionSettleCondition::Auto => ActionSettleCondition::AccessibilityChange,
        ActionSettleCondition::AccessibilityChange | ActionSettleCondition::AnyChange => {
            options.settle_condition
        }
        _ => return Ok(None),
    };
    let before = seatgeist_atspi::node(&target.node_id, 0, 16).ok();
    let before_revision = target_revision(&target_window, before.as_ref());
    let (subscription, issues) = match backend.subscribe(target.clone()).await {
        Ok(subscription) => (Some(subscription), Vec::new()),
        Err(error) => {
            tracing::warn!(%error, "AT-SPI event subscription unavailable; using target polling");
            (
                None,
                vec!["atspi_event_subscription_unavailable".to_string()],
            )
        }
    };
    Ok(Some(PreparedSemanticSettle {
        subscription,
        target_window,
        target_node_id: target.node_id,
        before_revision,
        condition,
        timeout_ms: options.settle_timeout_ms,
        interval_ms: options.settle_interval_ms,
        issues,
    }))
}

pub(crate) async fn finish(prepared: PreparedSemanticSettle) -> Observation {
    let started = Instant::now();
    let mut issues = prepared.issues;
    let mut event = None;
    let used_subscription = prepared.subscription.is_some();
    if let Some(mut subscription) = prepared.subscription {
        match subscription
            .wait_for_event(Duration::from_millis(prepared.timeout_ms))
            .await
        {
            Ok(received) => event = received,
            Err(error) => {
                issues.push("atspi_event_wait_failed".to_string());
                tracing::warn!(%error, "AT-SPI target event wait failed");
            }
        }
        if let Err(error) = subscription.close().await {
            issues.push("atspi_event_deregister_failed".to_string());
            tracing::warn!(%error, "AT-SPI target event deregistration failed");
        }
    }
    let timeout = Duration::from_millis(prepared.timeout_ms);
    let interval = Duration::from_millis(prepared.interval_ms);
    let mut samples = 0_u32;
    let (target_accessibility, after_revision) = loop {
        let target_accessibility = read_target_node(&prepared.target_node_id, &mut issues);
        samples = samples.saturating_add(1);
        let revision = target_revision(&prepared.target_window, target_accessibility.as_ref());
        if used_subscription
            || event.is_some()
            || revision != prepared.before_revision
            || started.elapsed() >= timeout
        {
            break (target_accessibility, revision);
        }
        let remaining = timeout.saturating_sub(started.elapsed());
        tokio::time::sleep(interval.min(remaining)).await;
    };
    let settled = event.is_some() || after_revision != prepared.before_revision;
    let backend = if event.is_some() {
        ActionSettleBackend::AtspiEvent
    } else {
        ActionSettleBackend::TargetRead
    };
    let elapsed_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    Observation {
        active_window: None,
        target_window: Some(prepared.target_window),
        windows: Vec::new(),
        monitors: Vec::new(),
        focused_accessibility: None,
        target_accessibility,
        screenshot_path: None,
        revision: Some(after_revision.clone()),
        issues,
        settle: Some(ActionSettleResult {
            confirmation: if settled {
                libseatgeist::ActionConfirmation::Confirmed
            } else {
                libseatgeist::ActionConfirmation::UnconfirmedTimeout
            },
            condition: prepared.condition,
            backend,
            target_scoped: true,
            event: event.as_ref().map(event_name),
            settled,
            timed_out: !settled,
            timeout_ms: prepared.timeout_ms,
            interval_ms: prepared.interval_ms,
            samples,
            elapsed_ms,
            before_revision: Some(prepared.before_revision),
            after_revision,
        }),
    }
}

pub(crate) fn unchanged_observation(
    target_window: WindowInfo,
    mut target_accessibility: AccessibilityNode,
    options: &PostActionOptions,
) -> Observation {
    target_accessibility.value = None;
    target_accessibility.value_truncated = false;
    target_accessibility.children.clear();
    let revision = target_revision(&target_window, Some(&target_accessibility));
    Observation {
        active_window: None,
        target_window: Some(target_window),
        windows: Vec::new(),
        monitors: Vec::new(),
        focused_accessibility: None,
        target_accessibility: Some(target_accessibility),
        screenshot_path: None,
        revision: Some(revision.clone()),
        issues: Vec::new(),
        settle: Some(ActionSettleResult {
            confirmation: libseatgeist::ActionConfirmation::NotRequested,
            condition: ActionSettleCondition::None,
            backend: ActionSettleBackend::TargetRead,
            target_scoped: true,
            event: None,
            settled: true,
            timed_out: false,
            timeout_ms: options.settle_timeout_ms,
            interval_ms: options.settle_interval_ms,
            samples: 1,
            elapsed_ms: 0,
            before_revision: Some(revision.clone()),
            after_revision: revision,
        }),
    }
}

fn read_target_node(node_id: &str, issues: &mut Vec<String>) -> Option<AccessibilityNode> {
    match seatgeist_atspi::node(node_id, 0, 16) {
        Ok(mut node) => {
            node.value = None;
            node.value_truncated = false;
            node.children.clear();
            Some(node)
        }
        Err(error) => {
            issues.push("target_accessibility_unavailable".to_string());
            tracing::debug!(%error, "target accessibility observation unavailable after action");
            None
        }
    }
}

fn event_name(event: &AccessibilityEvent) -> String {
    let category = event
        .interface
        .strip_prefix("org.a11y.atspi.Event.")
        .unwrap_or(&event.interface);
    format!("{category}.{}", event.member).to_ascii_lowercase()
}

fn target_revision(window: &WindowInfo, node: Option<&AccessibilityNode>) -> String {
    let encoded = serde_json::to_vec(&(window, node)).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(encoded);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use async_trait::async_trait;
    use seatgeist_backend::{AccessibilityEventSubscription, Result as BackendResult};

    use super::*;

    struct MockEventBackend {
        closed: Arc<AtomicBool>,
    }

    struct MockEventSubscription {
        closed: Arc<AtomicBool>,
    }

    #[async_trait]
    impl AccessibilityEventBackend for MockEventBackend {
        async fn subscribe(
            &self,
            _target: AccessibilityEventTarget,
        ) -> BackendResult<Box<dyn AccessibilityEventSubscription>> {
            Ok(Box::new(MockEventSubscription {
                closed: self.closed.clone(),
            }))
        }
    }

    #[async_trait]
    impl AccessibilityEventSubscription for MockEventSubscription {
        async fn wait_for_event(
            &mut self,
            _timeout: Duration,
        ) -> BackendResult<Option<AccessibilityEvent>> {
            Ok(Some(AccessibilityEvent {
                interface: "org.a11y.atspi.Event.Object".to_string(),
                member: "StateChanged".to_string(),
                source_node_id: "atspi://:1.42/org/a11y/atspi/accessible/55".to_string(),
            }))
        }

        async fn close(self: Box<Self>) -> BackendResult<()> {
            self.closed.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    fn target() -> AccessibilityEventTarget {
        AccessibilityEventTarget {
            application_bus_name: ":1.42".to_string(),
            node_id: "atspi://:1.42/org/a11y/atspi/accessible/55".to_string(),
            window_node_id: "atspi://:1.42/org/a11y/atspi/accessible/2".to_string(),
        }
    }

    fn window() -> WindowInfo {
        WindowInfo {
            id: "kwin-window-42".to_string(),
            app_id: Some("org.example.App".to_string()),
            title: "Example".to_string(),
            pid: Some(42),
            monitor_id: None,
            geometry: None,
        }
    }

    #[tokio::test]
    async fn event_backend_returns_target_scoped_settle_metadata_and_closes() {
        let closed = Arc::new(AtomicBool::new(false));
        let backend = MockEventBackend {
            closed: closed.clone(),
        };
        let options = PostActionOptions {
            observe_after: true,
            settle_condition: ActionSettleCondition::Auto,
            settle_timeout_ms: 50,
            settle_interval_ms: 10,
            image: None,
        };
        let prepared = prepare_with_backend(&backend, target(), window(), Some(&options))
            .await
            .expect("prepare succeeds")
            .expect("event settle is selected");
        let observation = finish(prepared).await;
        let settle = observation.settle.expect("settle metadata is present");

        assert_eq!(settle.condition, ActionSettleCondition::AccessibilityChange);
        assert_eq!(settle.backend, ActionSettleBackend::AtspiEvent);
        assert!(settle.target_scoped);
        assert!(settle.settled);
        assert_eq!(settle.event.as_deref(), Some("object.statechanged"));
        assert_eq!(
            observation
                .target_window
                .as_ref()
                .map(|window| window.id.as_str()),
            Some("kwin-window-42")
        );
        assert!(closed.load(Ordering::SeqCst));
    }
}
