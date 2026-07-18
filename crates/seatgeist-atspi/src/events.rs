use std::future::poll_fn;
use std::pin::Pin;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use libseatgeist::SeatgeistError;
use seatgeist_backend::{
    AccessibilityEvent, AccessibilityEventBackend, AccessibilityEventSubscription,
    AccessibilityEventTarget, Result,
};
use zbus::export::futures_core::Stream;

use super::{ATSPI_ROOT_SERVICE, accessibility_bus_address, parse_node_id};

const REGISTRY_PATH: &str = "/org/a11y/atspi/registry";
const REGISTRY_INTERFACE: &str = "org.a11y.atspi.Registry";
const EVENT_INTERFACE_PREFIX: &str = "org.a11y.atspi.Event.";
const EVENT_REGISTRATIONS: [&str; 3] = ["object:", "window:", "focus:"];

#[derive(Debug, Clone, Copy, Default)]
pub struct AtspiEventBackend;

struct AtspiEventSubscription {
    connection: zbus::Connection,
    stream: zbus::MessageStream,
    target: AccessibilityEventTarget,
}

#[async_trait]
impl AccessibilityEventBackend for AtspiEventBackend {
    async fn subscribe(
        &self,
        target: AccessibilityEventTarget,
    ) -> Result<Box<dyn AccessibilityEventSubscription>> {
        validate_target(&target)?;
        let address = accessibility_bus_address()?;
        let connection = zbus::connection::Builder::address(address.as_str())
            .map_err(|error| backend_unavailable("parse AT-SPI bus address", error))?
            .build()
            .await
            .map_err(|error| backend_unavailable("connect AT-SPI event bus", error))?;
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(target.application_bus_name.as_str())
            .map_err(|error| backend_unavailable("build AT-SPI event sender rule", error))?
            .build();
        let stream = zbus::MessageStream::for_match_rule(rule, &connection, Some(32))
            .await
            .map_err(|error| backend_unavailable("subscribe AT-SPI event signals", error))?;

        for event in EVENT_REGISTRATIONS {
            register_event(&connection, event, &target.application_bus_name).await?;
        }

        Ok(Box::new(AtspiEventSubscription {
            connection,
            stream,
            target,
        }))
    }
}

#[async_trait]
impl AccessibilityEventSubscription for AtspiEventSubscription {
    async fn wait_for_event(&mut self, timeout: Duration) -> Result<Option<AccessibilityEvent>> {
        let started = Instant::now();
        loop {
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Ok(None);
            }
            let next = tokio::time::timeout(
                remaining,
                poll_fn(|context| Pin::new(&mut self.stream).poll_next(context)),
            )
            .await;
            let message = match next {
                Err(_) => return Ok(None),
                Ok(None) => return Ok(None),
                Ok(Some(Err(error))) => {
                    return Err(backend_unavailable("receive AT-SPI event", error));
                }
                Ok(Some(Ok(message))) => message,
            };
            let header = message.header();
            let Some(interface) = header.interface().map(ToString::to_string) else {
                continue;
            };
            let Some(member) = header.member().map(ToString::to_string) else {
                continue;
            };
            let Some(path) = header.path().map(ToString::to_string) else {
                continue;
            };
            if !event_is_relevant(&interface, &path, &self.target) {
                continue;
            }
            return Ok(Some(AccessibilityEvent {
                interface,
                member,
                source_node_id: format!("atspi://{}{}", self.target.application_bus_name, path),
            }));
        }
    }

    async fn close(self: Box<Self>) -> Result<()> {
        let mut first_error = None;
        for event in EVENT_REGISTRATIONS {
            if let Err(error) =
                deregister_event(&self.connection, event, &self.target.application_bus_name).await
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

fn validate_target(target: &AccessibilityEventTarget) -> Result<()> {
    let node = parse_node_id(&target.node_id)?;
    let window = parse_node_id(&target.window_node_id)?;
    if node.service != target.application_bus_name || window.service != target.application_bus_name
    {
        return Err(SeatgeistError::InvalidRequest(
            "AT-SPI event target nodes must belong to the correlated application bus name"
                .to_string(),
        ));
    }
    Ok(())
}

fn event_is_relevant(
    interface: &str,
    source_path: &str,
    target: &AccessibilityEventTarget,
) -> bool {
    if !interface.starts_with(EVENT_INTERFACE_PREFIX) {
        return false;
    }
    let node_path = parse_node_id(&target.node_id).ok().map(|node| node.path);
    let window_path = parse_node_id(&target.window_node_id)
        .ok()
        .map(|node| node.path);
    node_path.as_deref() == Some(source_path) || window_path.as_deref() == Some(source_path)
}

async fn register_event(
    connection: &zbus::Connection,
    event: &str,
    application_bus_name: &str,
) -> Result<()> {
    registry_proxy(connection)
        .await?
        .call::<_, _, ()>(
            "RegisterEvent",
            &(event, Vec::<String>::new(), application_bus_name),
        )
        .await
        .map_err(|error| backend_unavailable("register AT-SPI event", error))
}

async fn deregister_event(
    connection: &zbus::Connection,
    event: &str,
    application_bus_name: &str,
) -> Result<()> {
    registry_proxy(connection)
        .await?
        .call::<_, _, ()>("DeregisterEvent", &(event, application_bus_name))
        .await
        .map_err(|error| backend_unavailable("deregister AT-SPI event", error))
}

async fn registry_proxy(connection: &zbus::Connection) -> Result<zbus::Proxy<'_>> {
    zbus::Proxy::new(
        connection,
        ATSPI_ROOT_SERVICE,
        REGISTRY_PATH,
        REGISTRY_INTERFACE,
    )
    .await
    .map_err(|error| backend_unavailable("create AT-SPI Registry proxy", error))
}

fn backend_unavailable(context: &str, error: impl std::fmt::Display) -> SeatgeistError {
    SeatgeistError::BackendUnavailable(format!("{context}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> AccessibilityEventTarget {
        AccessibilityEventTarget {
            application_bus_name: ":1.42".to_string(),
            node_id: "atspi://:1.42/org/a11y/atspi/accessible/55".to_string(),
            window_node_id: "atspi://:1.42/org/a11y/atspi/accessible/2".to_string(),
        }
    }

    #[test]
    fn validates_correlated_application_identity() {
        validate_target(&target()).expect("matching application is valid");
        let mut mismatched = target();
        mismatched.node_id = "atspi://:1.99/org/a11y/atspi/accessible/55".to_string();
        assert!(validate_target(&mismatched).is_err());
    }

    #[test]
    fn accepts_only_target_node_or_containing_window_events() {
        let target = target();
        assert!(event_is_relevant(
            "org.a11y.atspi.Event.Object",
            "/org/a11y/atspi/accessible/55",
            &target
        ));
        assert!(event_is_relevant(
            "org.a11y.atspi.Event.Window",
            "/org/a11y/atspi/accessible/2",
            &target
        ));
        assert!(!event_is_relevant(
            "org.a11y.atspi.Event.Object",
            "/org/a11y/atspi/accessible/99",
            &target
        ));
        assert!(!event_is_relevant(
            "org.freedesktop.DBus",
            "/org/a11y/atspi/accessible/55",
            &target
        ));
    }
}
