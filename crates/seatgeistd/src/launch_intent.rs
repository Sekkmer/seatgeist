//! Placement/focus intent for an already-admitted, pre-exec child. No process launch.
use crate::{
    kwin_bridge::LaunchWindowTicket,
    safety_runtime::{ApprovalStore, PanicStopState},
};
use anyhow::{Context, Result, bail};
use libseatgeist::{JournalClientContext, LaunchIntentStatus, SafetyClass};
use std::{
    collections::HashMap,
    fs,
    os::fd::OwnedFd,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Identity {
    pid: u32,
    parent: u32,
    uid: u32,
    start: u64,
}

fn identity(pid: u32) -> Result<Identity> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))
        .context("read launch process generation")?;
    let fields: Vec<_> = stat
        .rsplit_once(')')
        .context("invalid process stat")?
        .1
        .split_whitespace()
        .collect();
    if fields.first().is_none_or(|s| matches!(*s, "Z" | "X")) {
        bail!("launch process is not live");
    }
    let parent = fields.get(1).context("missing parent pid")?.parse()?;
    let start = fields
        .get(19)
        .context("missing process start ticks")?
        .parse()?;
    let status = fs::read_to_string(format!("/proc/{pid}/status"))?;
    let uids: Vec<u32> = status
        .lines()
        .find_map(|s| s.strip_prefix("Uid:"))
        .context("missing process uid")?
        .split_whitespace()
        .map(str::parse)
        .collect::<std::result::Result<_, _>>()?;
    let uid = libseatgeist::current_euid()?;
    if uids.len() != 4 || uids.iter().any(|id| *id != uid) {
        bail!("launch process uid mismatch");
    }
    Ok(Identity {
        pid,
        parent,
        uid,
        start,
    })
}

#[derive(Debug)]
struct ProcessPin {
    identity: Identity,
    fd: OwnedFd,
}
impl ProcessPin {
    fn open(pid: u32) -> Result<Self> {
        let before = identity(pid)?;
        // pidfds survive exec, and become readable on exit. Unsupported kernels
        // refuse this operation rather than silently degrading to a numeric PID.
        let fd = rustix::process::pidfd_open(
            rustix::process::Pid::from_raw(pid as i32).context("invalid launch pid")?,
            rustix::process::PidfdFlags::empty(),
        )
        .context("pin launch process with pidfd")?;
        let pin = Self {
            identity: before,
            fd,
        };
        if !pin.live() {
            bail!("launch process generation changed while pinning");
        }
        Ok(pin)
    }
    fn live(&self) -> bool {
        let mut events = [rustix::event::PollFd::new(
            &self.fd,
            rustix::event::PollFlags::IN,
        )];
        rustix::event::poll(
            &mut events,
            Some(&rustix::event::Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            }),
        )
        .is_ok_and(|count| count == 0)
            && identity(self.identity.pid).is_ok_and(|now| now == self.identity)
    }
}

fn controller(client: Option<&JournalClientContext>) -> Result<Identity> {
    let client = client.context("launch intent requires a verified CLI peer")?;
    if client.tool.as_deref() != Some("seatgeist-cli")
        || client.process_name.as_deref() != Some("seatgeist-cli")
    {
        bail!("launch intent requires the CLI invoked by the living child controller");
    }
    let peer = identity(
        client
            .pid
            .context("launch intent requires trusted peer pid")?,
    )?;
    identity(peer.parent)
}

#[derive(Debug, Clone)]
pub(crate) struct IntentSafety {
    pub policy: seatgeist_policy::PolicyEngine,
    pub approvals: ApprovalStore,
    pub panic: PanicStopState,
}
impl IntentSafety {
    fn authorized(&self) -> bool {
        !self.panic.status().enabled
            && crate::enforce_safety_class(
                &self.policy,
                &self.approvals,
                "launch_intent",
                SafetyClass::ControlSemantic,
                false,
            )
            .is_ok()
    }
}

struct Lease {
    controller: ProcessPin,
    child: ProcessPin,
    app_id: String,
    deadline: Instant,
    status: LaunchIntentStatus,
    window_id: Option<String>,
    ticket: Option<LaunchWindowTicket>,
    safety: IntentSafety,
}
impl Lease {
    fn live(&self) -> bool {
        self.deadline > Instant::now()
            && self.controller.live()
            && self.child.live()
            && self.safety.authorized()
    }
}

#[derive(Clone, Default)]
pub(crate) struct LaunchIntentStore(Arc<Mutex<HashMap<String, Lease>>>);
impl std::fmt::Debug for LaunchIntentStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LaunchIntentStore(..)")
    }
}
impl LaunchIntentStore {
    pub(crate) fn reserve(
        &self,
        child_pid: u32,
        child_start: u64,
        app_id: String,
        timeout_ms: u64,
        client: Option<&JournalClientContext>,
        safety: IntentSafety,
    ) -> Result<LaunchIntentStatus> {
        let owner = controller(client)?;
        let controller = ProcessPin::open(owner.pid)?;
        if controller.identity != owner {
            bail!("launch controller generation changed");
        }
        let child = ProcessPin::open(child_pid)?;
        if child.identity.parent != owner.pid
            || child.identity.start != child_start
            || child_pid == owner.pid
        {
            bail!("launch child must be the controller's exact live direct child generation");
        }
        let id = uuid::Uuid::new_v4().to_string();
        let status = LaunchIntentStatus {
            launch_id: id.clone(),
            state: "armed".into(),
            expires_unix_ms: crate::unix_time_ms()? + timeout_ms,
            controller_pid: owner.pid,
            controller_start_ticks: owner.start,
            child_pid,
            child_start_ticks: child_start,
            window: None,
            focus_preserved: None,
        };
        let mut leases = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("launch intent store poisoned"))?;
        if leases.len() >= 16 {
            bail!("launch intent quota exceeded");
        }
        if leases
            .values()
            .any(|l| l.child.identity.pid == child_pid && l.deadline > Instant::now())
        {
            bail!("launch child already has a pending intent");
        }
        leases.insert(
            id,
            Lease {
                controller,
                child,
                app_id,
                deadline: Instant::now() + Duration::from_millis(timeout_ms),
                status: status.clone(),
                window_id: None,
                ticket: None,
                safety,
            },
        );
        Ok(status)
    }
    pub(crate) fn attach(&self, id: &str, ticket: LaunchWindowTicket) -> Result<()> {
        let mut leases = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("launch intent store poisoned"))?;
        let lease = leases
            .get_mut(id)
            .context("launch intent was cancelled or expired")?;
        if !lease.live() {
            bail!("launch child/controller/authorization expired before arming");
        }
        lease.ticket = Some(ticket);
        Ok(())
    }
    pub(crate) fn validate(&self, id: &str, pid: u32, app: &str, window_id: &str) -> bool {
        let Ok(mut leases) = self.0.lock() else {
            return false;
        };
        let Some(lease) = leases.get_mut(id) else {
            return false;
        };
        if !lease.live()
            || pid != lease.child.identity.pid
            || app != lease.app_id
            || window_id.is_empty()
        {
            return false;
        }
        if let Some(original) = &lease.window_id {
            return original == window_id;
        }
        lease.window_id = Some(window_id.into());
        true
    }
    pub(crate) fn take_ticket(
        &self,
        id: &str,
        client: Option<&JournalClientContext>,
    ) -> Result<(LaunchWindowTicket, Duration)> {
        let owner = controller(client)?;
        let mut leases = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("launch intent store poisoned"))?;
        let lease = leases
            .get_mut(id)
            .context("unknown, completed, cancelled or expired launch intent")?;
        if owner != lease.controller.identity {
            bail!("launch intent controller mismatch");
        }
        if !lease.live() {
            bail!("launch intent child/controller/authorization expired");
        }
        let ticket = lease
            .ticket
            .take()
            .context("launch intent already waiting or not armed")?;
        Ok((
            ticket,
            lease.deadline.saturating_duration_since(Instant::now()),
        ))
    }
    pub(crate) fn remove_owned(
        &self,
        id: &str,
        client: Option<&JournalClientContext>,
        completed: bool,
    ) -> Result<LaunchIntentStatus> {
        let owner = controller(client)?;
        let mut leases = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("launch intent store poisoned"))?;
        let lease = leases
            .get(id)
            .context("unknown, completed, cancelled or expired launch intent")?;
        if owner != lease.controller.identity {
            bail!("launch intent controller mismatch");
        }
        if completed && !lease.live() {
            bail!("launch identity or authorization expired before confirmation");
        }
        Ok(leases.remove(id).unwrap().status)
    }
    pub(crate) fn discard(&self, id: &str) -> bool {
        self.0
            .lock()
            .ok()
            .is_some_and(|mut leases| leases.remove(id).is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command};
    struct Held(Child);
    impl Held {
        fn new() -> Self {
            Self(Command::new("/bin/sleep").arg("30").spawn().unwrap())
        }
    }
    impl Drop for Held {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    fn safety() -> IntentSafety {
        let mut config = seatgeist_policy::PolicyConfig::default();
        config.default_control = libseatgeist::ToolApprovalLevel::Allow;
        IntentSafety {
            policy: seatgeist_policy::PolicyEngine::new(config),
            approvals: ApprovalStore::new(None),
            panic: PanicStopState::new(std::env::temp_dir().join(format!(
                "seatgeist-intent-panic-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ))),
        }
    }
    fn cli(peer: &Held) -> JournalClientContext {
        // Unit boundary: production fills pid/name from trusted Unix credentials.
        JournalClientContext {
            tool: Some("seatgeist-cli".into()),
            pid: Some(peer.0.id()),
            process_name: Some("seatgeist-cli".into()),
        }
    }
    #[test]
    fn exact_child_generation_controller_app_and_single_window_are_required() {
        let peer = Held::new();
        let child = Held::new();
        let client = cli(&peer);
        let store = LaunchIntentStore::default();
        let generation = identity(child.0.id()).unwrap().start;
        assert!(
            store
                .reserve(
                    child.0.id(),
                    generation + 1,
                    "app".into(),
                    30000,
                    Some(&client),
                    safety()
                )
                .is_err()
        );
        let status = store
            .reserve(
                child.0.id(),
                generation,
                "app".into(),
                30000,
                Some(&client),
                safety(),
            )
            .unwrap();
        assert_eq!(status.controller_pid, std::process::id());
        assert!(!store.validate(&status.launch_id, peer.0.id(), "app", "w"));
        assert!(!store.validate(&status.launch_id, child.0.id(), "other-app", "w"));
        assert!(store.validate(&status.launch_id, child.0.id(), "app", "w"));
        assert!(store.validate(&status.launch_id, child.0.id(), "app", "w"));
        assert!(!store.validate(&status.launch_id, child.0.id(), "app", "replacement"));
        let mut foreign = client.clone();
        foreign.pid = Some(std::process::id());
        assert!(
            store
                .remove_owned(&status.launch_id, Some(&foreign), false)
                .is_err()
        );
        store
            .remove_owned(&status.launch_id, Some(&client), false)
            .unwrap();
        assert!(!store.validate(&status.launch_id, child.0.id(), "app", "w"));
    }
    #[test]
    fn expiry_exit_and_revoked_authority_cannot_place_a_window() {
        let peer = Held::new();
        let mut child = Held::new();
        let client = cli(&peer);
        let store = LaunchIntentStore::default();
        let auth = safety();
        let status = store
            .reserve(
                child.0.id(),
                identity(child.0.id()).unwrap().start,
                "app".into(),
                30000,
                Some(&client),
                auth.clone(),
            )
            .unwrap();
        assert!(store.validate(&status.launch_id, child.0.id(), "app", "w"));
        auth.panic.set_enabled(true).unwrap();
        assert!(!store.validate(&status.launch_id, child.0.id(), "app", "w"));
        auth.panic.set_enabled(false).unwrap();
        store
            .0
            .lock()
            .unwrap()
            .get_mut(&status.launch_id)
            .unwrap()
            .deadline = Instant::now();
        assert!(!store.validate(&status.launch_id, child.0.id(), "app", "w"));
        store.discard(&status.launch_id);
        let status = store
            .reserve(
                child.0.id(),
                identity(child.0.id()).unwrap().start,
                "app".into(),
                30000,
                Some(&client),
                auth,
            )
            .unwrap();
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        assert!(!store.validate(&status.launch_id, child.0.id(), "app", "w"));
        assert!(store.take_ticket(&status.launch_id, Some(&client)).is_err());
    }
    #[test]
    fn neither_another_process_nor_cli_scope_alone_owns_a_child() {
        let peer = Held::new();
        let mut client = cli(&peer);
        let store = LaunchIntentStore::default();
        // The controller itself is not its own direct child.
        let own = identity(std::process::id()).unwrap();
        assert!(
            store
                .reserve(
                    own.pid,
                    own.start,
                    "app".into(),
                    30000,
                    Some(&client),
                    safety()
                )
                .is_err()
        );
        client.tool = Some("seatgeist-mcp".into());
        assert!(controller(Some(&client)).is_err());
        assert!(controller(None).is_err());
    }
}
