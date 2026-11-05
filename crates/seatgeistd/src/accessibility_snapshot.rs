//! Owner-bound, bounded refs. AT-SPI paths are locators, never authorization.
use crate::{
    DaemonRuntime,
    session_owner::{SessionOwner, SessionOwnerIdentity},
};
use anyhow::{Context, Result, bail};
use libseatgeist::{
    AccessibilityAction, AccessibilityNode, AccessibilityRefActionRequest, AccessibilitySnapshot,
    AccessibilitySnapshotNode, AccessibilitySnapshotRequest, ActionDisposition, DaemonResponse,
    ErrorKind, JournalClientContext, RetryAdvice, TargetWindowGuard, WindowInfo,
};
use seatgeist_atspi::snapshot::{AtspiSnapshotReader, SnapshotReader, TreeSample};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

struct SnapshotContext<'a> {
    store: &'a SnapshotStore,
    windows: &'a dyn seatgeist_backend::WindowBackend,
    policy: &'a crate::AppPolicy,
}
impl<'a> From<&'a DaemonRuntime> for SnapshotContext<'a> {
    fn from(runtime: &'a DaemonRuntime) -> Self {
        Self {
            store: &runtime.accessibility_snapshot_store,
            windows: runtime.window_backend.as_ref(),
            policy: &runtime.app_policy,
        }
    }
}

const TTL: Duration = Duration::from_secs(300);
const MAX_SNAPSHOTS: usize = 32;
const MAX_OWNER_SNAPSHOTS: usize = 8;
const MAX_REFS: usize = 4096;
#[derive(Debug, Clone, Default)]
pub(crate) struct SnapshotStore(Arc<Mutex<HashMap<String, SnapshotRecord>>>);
#[derive(Debug, Clone)]
struct SnapshotRecord {
    owner: SessionOwnerIdentity,
    owner_generation: Option<String>,
    window: WindowInfo,
    process_generation: String,
    window_node_id: String,
    expires: Instant,
    entries: Vec<AccessibilityNode>,
}
#[derive(Debug, Clone)]
pub(crate) struct ResolvedRef {
    record: SnapshotRecord,
    node: AccessibilityNode,
}
impl ResolvedRef {
    pub(crate) fn guard(&self) -> TargetWindowGuard {
        guard(&self.record.window)
    }
    pub(crate) fn name(&self) -> &str {
        self.node.name.as_deref().unwrap_or("")
    }
}
fn owner(client: Option<&JournalClientContext>) -> Result<(SessionOwnerIdentity, Option<String>)> {
    let owner = SessionOwner::from_client(client)?.identity();
    let generation = match owner {
        SessionOwnerIdentity::Process(pid) => Some(process_generation(pid)?),
        _ => None,
    };
    Ok((owner, generation))
}
fn process_generation(pid: u32) -> Result<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .context("accessibility process unavailable")?;
    let start = stat
        .rsplit_once(')')
        .and_then(|(_, s)| s.split_whitespace().nth(19))
        .ok_or_else(|| anyhow::anyhow!("accessibility process generation unavailable"))?;
    let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
    Ok(format!("{}:{start}", boot.trim()))
}
fn parse_ref(value: &str) -> Result<(&str, usize)> {
    let (snapshot, element) = value
        .strip_prefix('@')
        .and_then(|s| s.rsplit_once(":e"))
        .ok_or_else(|| {
            anyhow::anyhow!("accessibility_ref_invalid: use the full snapshot-qualified ref")
        })?;
    let uuid = snapshot
        .strip_prefix("sg1:")
        .ok_or_else(|| anyhow::anyhow!("accessibility_ref_invalid: unsupported ref version"))?;
    uuid::Uuid::parse_str(uuid).context("accessibility_ref_invalid: malformed snapshot id")?;
    let index = element
        .parse::<usize>()
        .ok()
        .filter(|i| *i > 0 && *i <= MAX_REFS)
        .ok_or_else(|| anyhow::anyhow!("accessibility_ref_invalid: malformed element index"))?;
    Ok((snapshot, index - 1))
}
impl SnapshotStore {
    pub(crate) fn resolve(
        &self,
        reference: &str,
        client: Option<&JournalClientContext>,
    ) -> Result<ResolvedRef> {
        let (snapshot, index) = parse_ref(reference)?;
        let (owner, owner_generation) = owner(client)?;
        let mut records = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("snapshot store poisoned"))?;
        records.retain(|_, r| r.expires > Instant::now());
        let record = records.get(snapshot).ok_or_else(|| {
            anyhow::anyhow!("accessibility_snapshot_expired: take a new snapshot")
        })?;
        if record.owner != owner || record.owner_generation != owner_generation {
            bail!("accessibility_ref_owner_mismatch: snapshot belongs to another client");
        }
        let node = record.entries.get(index).cloned().ok_or_else(|| {
            anyhow::anyhow!("accessibility_ref_stale: element absent from snapshot")
        })?;
        Ok(ResolvedRef {
            record: record.clone(),
            node,
        })
    }
}
fn guard(window: &WindowInfo) -> TargetWindowGuard {
    TargetWindowGuard {
        expected_window_id: window.id.clone(),
        expected_app_id: window.app_id.clone(),
        expected_pid: window.pid,
        title_contains: None,
    }
}
fn validate_window(record: &SnapshotRecord, window: &WindowInfo) -> Result<()> {
    if record.window.id != window.id
        || record.window.pid != window.pid
        || record.window.app_id != window.app_id
    {
        bail!("accessibility_ref_stale: KWin window identity changed");
    }
    if process_generation(
        window
            .pid
            .ok_or_else(|| anyhow::anyhow!("accessibility_ref_stale: target pid unavailable"))?,
    )? != record.process_generation
    {
        bail!("accessibility_ref_stale: target process generation changed");
    }
    Ok(())
}
fn validate_node(saved: &AccessibilityNode, live: &AccessibilityNode) -> Result<()> {
    if saved.id != live.id
        || saved.role != live.role
        || saved.name != live.name
        || saved.sensitive != live.sensitive
    {
        bail!("accessibility_ref_stale: accessible identity changed; take a fresh snapshot");
    }
    if live.sensitive || crate::secret_field_label(live.name.as_deref().unwrap_or("")) {
        bail!("accessibility_ref_stale: sensitive targets are unavailable through refs");
    }
    Ok(())
}
fn validate_context(
    record: &SnapshotRecord,
    candidate: &seatgeist_atspi::AccessibilityMatch,
    window: &WindowInfo,
) -> Result<()> {
    if candidate.window_node_id.as_deref() != Some(record.window_node_id.as_str()) {
        bail!("accessibility_ref_stale: accessible moved to another window");
    }
    crate::target::resolve_semantic_target(
        candidate.clone(),
        std::slice::from_ref(window),
        &guard(window),
    )?;
    Ok(())
}
pub(crate) fn refusal(error: anyhow::Error) -> DaemonResponse {
    let message = format!("{error:#}");
    let reason = if message.contains("owner_mismatch") {
        "accessibility_ref_owner_mismatch"
    } else if message.contains("snapshot_expired") {
        "accessibility_snapshot_expired"
    } else if message.contains("ref_invalid") {
        "accessibility_ref_invalid"
    } else {
        "accessibility_ref_stale"
    };
    DaemonResponse::Error {
        kind: if message.contains("app policy") {
            ErrorKind::AppDenied
        } else if reason == "accessibility_ref_owner_mismatch" {
            ErrorKind::SessionOwnerMismatch
        } else if reason == "accessibility_ref_invalid" {
            ErrorKind::Validation
        } else {
            ErrorKind::TargetMismatch
        },
        reason_code: Some(reason.into()),
        message,
        disposition: Some(ActionDisposition::not_delivered(RetryAdvice::AfterChange)),
    }
}

pub(crate) async fn snapshot(
    request: AccessibilitySnapshotRequest,
    client: Option<&JournalClientContext>,
    runtime: &DaemonRuntime,
) -> DaemonResponse {
    match snapshot_inner(
        request,
        client,
        &SnapshotContext::from(runtime),
        Arc::new(AtspiSnapshotReader),
    )
    .await
    {
        Ok(value) => DaemonResponse::AccessibilitySnapshot(Box::new(value)),
        Err(error) => refusal(error),
    }
}
async fn exact_window(
    runtime: &SnapshotContext<'_>,
    id: &str,
    control: bool,
) -> Result<WindowInfo> {
    let window = runtime
        .windows
        .list_windows()
        .await
        .map_err(anyhow::Error::msg)?
        .into_iter()
        .find(|w| w.id == id)
        .ok_or_else(|| anyhow::anyhow!("accessibility_ref_stale: exact KWin window is gone"))?;
    if crate::observation_policy::app_is_protected(runtime.policy, window.app_id.as_deref()) {
        bail!("app policy denied snapshot of protected application");
    }
    if control {
        crate::window_safety::enforce_app_policy_for_app(
            runtime.policy,
            window.app_id.as_deref(),
            "accessibility ref target",
        )?;
    }
    Ok(window)
}
async fn snapshot_inner(
    request: AccessibilitySnapshotRequest,
    client: Option<&JournalClientContext>,
    runtime: &SnapshotContext<'_>,
    reader: Arc<dyn SnapshotReader>,
) -> Result<AccessibilitySnapshot> {
    if request.window_id.is_some() == request.root_ref.is_some() {
        bail!("accessibility_ref_invalid: supply exactly one window_id or root_ref");
    }
    if request.depth > 32 || request.max_nodes == 0 || request.max_nodes > MAX_REFS {
        bail!("accessibility_ref_invalid: depth must be 0..32 and max_nodes 1..4096");
    }
    let (owner, owner_generation) = owner(client)?;
    let depth = if request.skeleton { 3 } else { request.depth };
    let (snapshot_id, mut record, root_id) = if let Some(reference) = request.root_ref.as_deref() {
        let resolved = runtime.store.resolve(reference, client)?;
        let window = exact_window(runtime, &resolved.record.window.id, false).await?;
        validate_window(&resolved.record, &window)?;
        let r = reader.clone();
        let id = resolved.node.id.clone();
        let current = tokio::task::spawn_blocking(move || r.context(&id)).await??;
        validate_node(&resolved.node, &current.node)?;
        validate_context(&resolved.record, &current, &window)?;
        let (id, _) = parse_ref(reference)?;
        (id.to_owned(), resolved.record, resolved.node.id)
    } else {
        let window = exact_window(
            runtime,
            request.window_id.as_deref().unwrap_or_default(),
            false,
        )
        .await?;
        let pid = window
            .pid
            .ok_or_else(|| anyhow::anyhow!("snapshot requires KWin target pid"))?;
        let process_generation = process_generation(pid)?;
        let r = reader.clone();
        let matches = tokio::task::spawn_blocking(move || r.windows(pid)).await??;
        let mut candidates = matches
            .into_iter()
            .filter_map(|candidate| {
                crate::target::resolve_semantic_target(
                    candidate,
                    std::slice::from_ref(&window),
                    &guard(&window),
                )
                .ok()
            })
            .collect::<Vec<_>>();
        if candidates.len() != 1 {
            bail!(
                "accessibility_ref_stale: exact KWin/AT-SPI window correlation missing or ambiguous"
            );
        }
        let target = candidates.remove(0);
        let root_id = target.node.id.clone();
        (
            format!("sg1:{}", uuid::Uuid::new_v4()),
            SnapshotRecord {
                owner,
                owner_generation,
                window,
                process_generation,
                window_node_id: target.event_target.window_node_id,
                expires: Instant::now() + TTL,
                entries: Vec::new(),
            },
            root_id,
        )
    };
    let r = reader.clone();
    let root = root_id.clone();
    let max_nodes = request.max_nodes;
    let sample = tokio::task::spawn_blocking(move || r.tree(&root, depth, max_nodes)).await??;
    let window = exact_window(runtime, &record.window.id, false).await?;
    validate_window(&record, &window)?;
    // Correlate the root again after a potentially slow read. Never publish a
    // replacement or a tree that crossed windows during collection.
    let r = reader.clone();
    let root = root_id;
    let current = tokio::task::spawn_blocking(move || r.context(&root)).await??;
    validate_context(&record, &current, &window)?;
    if record.expires <= Instant::now() {
        bail!("accessibility_snapshot_expired: read outlived snapshot");
    }
    record.window = window.clone();
    let nodes_observed = count_nodes(&sample.root);
    let root = render(&sample.root, &sample, &snapshot_id, &mut record.entries)?;
    let root =
        filter_tree(root, request.interactive_only, request.compact, true).expect("root retained");
    if record
        .entries
        .iter()
        .map(|node| serde_json::to_vec(node).map(|v| v.len()))
        .collect::<std::result::Result<Vec<_>, _>>()?
        .iter()
        .sum::<usize>()
        > 2 * 1024 * 1024
    {
        bail!("accessibility_ref_invalid: snapshot byte budget exhausted; narrow the tree");
    }
    let ref_count = record.entries.len();
    let expires_in_ms = record
        .expires
        .saturating_duration_since(Instant::now())
        .as_millis() as u64;
    let mut records = runtime
        .store
        .0
        .lock()
        .map_err(|_| anyhow::anyhow!("snapshot store poisoned"))?;
    records.retain(|_, r| r.expires > Instant::now());
    if request.root_ref.is_some() {
        let old = records.get(&snapshot_id).ok_or_else(|| {
            anyhow::anyhow!("accessibility_snapshot_expired: take a fresh snapshot")
        })?;
        // Two simultaneous drills must not overwrite each other's refs.
        if old.entries.len() > record.entries.len()
            || !old.entries.iter().zip(&record.entries).all(|(a, b)| a == b)
        {
            bail!("accessibility_ref_stale: concurrent drill; observe again");
        }
    } else {
        if records.len() >= MAX_SNAPSHOTS
            || records.values().filter(|r| r.owner == record.owner).count() >= MAX_OWNER_SNAPSHOTS
        {
            let owner_full =
                records.values().filter(|r| r.owner == record.owner).count() >= MAX_OWNER_SNAPSHOTS;
            let oldest = records
                .iter()
                .filter(|(_, r)| !owner_full || r.owner == record.owner)
                .min_by_key(|(_, r)| r.expires)
                .map(|(id, _)| id.clone());
            if let Some(oldest) = oldest {
                records.remove(&oldest);
            }
        }
    }
    records.insert(snapshot_id.clone(), record);
    Ok(AccessibilitySnapshot {
        snapshot_id,
        window,
        root,
        complete: sample.complete,
        nodes_observed,
        ref_count,
        expires_in_ms,
    })
}
fn count_nodes(node: &AccessibilityNode) -> usize {
    1 + node.children.iter().map(count_nodes).sum::<usize>()
}
fn render(
    node: &AccessibilityNode,
    sample: &TreeSample,
    snapshot: &str,
    entries: &mut Vec<AccessibilityNode>,
) -> Result<AccessibilitySnapshotNode> {
    let mut saved = node.clone();
    saved.children.clear();
    saved.sensitive |= crate::secret_field_label(saved.name.as_deref().unwrap_or(""));
    if saved.sensitive {
        saved.value = None;
        saved.value_truncated = false;
    }
    let index = if let Some(index) = entries.iter().position(|n| {
        n.id == saved.id
            && n.role == saved.role
            && n.name == saved.name
            && n.sensitive == saved.sensitive
    }) {
        index
    } else {
        if entries.len() >= MAX_REFS {
            bail!(
                "accessibility_ref_invalid: snapshot ref budget exhausted; start a fresh snapshot"
            );
        }
        entries.push(saved.clone());
        entries.len() - 1
    };
    let children = if saved.sensitive {
        Vec::new()
    } else {
        node.children
            .iter()
            .map(|n| render(n, sample, snapshot, entries))
            .collect::<Result<Vec<_>>>()?
    };
    let children_count = if saved.sensitive {
        0
    } else {
        sample
            .children_count
            .get(&node.id)
            .copied()
            .unwrap_or(node.children.len())
    };
    Ok(AccessibilitySnapshotNode {
        ref_id: format!("@{snapshot}:e{}", index + 1),
        role: saved.role,
        name: saved.name,
        value: saved.value,
        value_truncated: saved.value_truncated,
        sensitive: saved.sensitive,
        states: saved.states,
        bounds: saved.bounds,
        actions: saved.actions,
        children_count,
        subtree_truncated: children_count > children.len(),
        children,
    })
}
fn filter_tree(
    mut node: AccessibilitySnapshotNode,
    interactive: bool,
    compact: bool,
    root: bool,
) -> Option<AccessibilitySnapshotNode> {
    node.children = node
        .children
        .into_iter()
        .filter_map(|n| filter_tree(n, interactive, compact, false))
        .collect();
    if !root
        && interactive
        && node.actions.is_empty()
        && node.children.is_empty()
        && !node.subtree_truncated
    {
        return None;
    }
    if !root
        && compact
        && node.name.is_none()
        && node.value.is_none()
        && node.actions.is_empty()
        && !node.subtree_truncated
        && node.children.len() == 1
    {
        return node.children.pop();
    }
    Some(node)
}

pub(crate) async fn act(
    request: AccessibilityRefActionRequest,
    client: Option<&JournalClientContext>,
    runtime: &DaemonRuntime,
    options: Option<&libseatgeist::PostActionOptions>,
) -> DaemonResponse {
    match act_inner(
        request,
        client,
        &SnapshotContext::from(runtime),
        options,
        Arc::new(AtspiSnapshotReader),
    )
    .await
    {
        Ok(response) => response,
        Err(error) => refusal(error),
    }
}
async fn act_inner(
    request: AccessibilityRefActionRequest,
    client: Option<&JournalClientContext>,
    runtime: &SnapshotContext<'_>,
    options: Option<&libseatgeist::PostActionOptions>,
    reader: Arc<dyn SnapshotReader>,
) -> Result<DaemonResponse> {
    if (request.action == AccessibilityAction::SetText) != request.text.is_some()
        || request
            .text
            .as_ref()
            .is_some_and(|text| text.len() > 32768 || text.chars().count() > 8192)
    {
        bail!(
            "accessibility_ref_invalid: text is required only for set_text and must fit 8192 characters"
        );
    }
    let resolved = runtime.store.resolve(&request.ref_id, client)?;
    let window = exact_window(runtime, &resolved.record.window.id, true).await?;
    validate_window(&resolved.record, &window)?;
    let r = reader.clone();
    let id = resolved.node.id.clone();
    let current = tokio::task::spawn_blocking(move || r.context(&id)).await??;
    validate_node(&resolved.node, &current.node)?;
    validate_context(&resolved.record, &current, &window)?;
    let target = crate::target::resolve_semantic_target(
        current,
        std::slice::from_ref(&window),
        &guard(&window),
    )?;
    let prepared = if options.is_some() {
        crate::prepare_semantic_settle(&target.clone().into(), options).await
    } else {
        None
    };
    // Subscribe-before-dispatch can wait; repeat every identity check after it.
    let window = exact_window(runtime, &resolved.record.window.id, true).await?;
    validate_window(&resolved.record, &window)?;
    runtime.store.resolve(&request.ref_id, client)?;
    let r = reader.clone();
    let id = resolved.node.id.clone();
    let current = tokio::task::spawn_blocking(move || r.context(&id)).await??;
    validate_node(&resolved.node, &current.node)?;
    validate_context(&resolved.record, &current, &window)?;
    if !current.node.actions.contains(&request.action)
        && request.action != AccessibilityAction::SetText
    {
        bail!("accessibility_ref_stale: requested action no longer advertised");
    }
    if !current.node.states.iter().any(|state| state == "enabled")
        || current
            .node
            .states
            .iter()
            .any(|state| matches!(state.as_str(), "defunct" | "disabled"))
        || (request.action == AccessibilityAction::SetText
            && current.node.states.iter().any(|state| state == "read_only"))
    {
        bail!("accessibility_ref_stale: target is not actionable");
    }
    let r = reader.clone();
    let id = resolved.node.id.clone();
    let action = request.action.clone();
    tokio::task::spawn_blocking(move || r.validate_action(&id, &action)).await??;
    let r = reader.clone();
    let id = resolved.node.id;
    let action = request.action;
    let text = request.text;
    let result = tokio::task::spawn_blocking(move || r.invoke(&id, action, text.as_deref())).await;
    match result {
        Ok(Ok(())) => {
            let mut result = crate::semantic_action_result(
                "accessibility ref action dispatched".into(),
                prepared,
            )
            .await;
            result.disposition = Some(ActionDisposition::delivered());
            Ok(DaemonResponse::Action(Box::new(result)))
        }
        result => {
            // A timeout/transport error after attempting a write is uncertain;
            // never infer non-delivery from an error string.
            drop(prepared);
            let message = match result {
                Ok(Err(e)) => e.to_string(),
                Err(e) => e.to_string(),
                _ => unreachable!(),
            };
            Ok(DaemonResponse::Error {
                kind: ErrorKind::BackendFailed,
                reason_code: Some("accessibility_ref_delivery_unknown".into()),
                message,
                disposition: Some(ActionDisposition::unknown()),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use seatgeist_atspi::AccessibilityMatch;
    use seatgeist_backend::WindowBackend;
    use seatgeist_testkit::MockWindowBackend;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Reader {
        tree: Mutex<AccessibilityNode>,
        reads: Mutex<Vec<(String, usize, usize)>>,
        writes: AtomicUsize,
        fail_write: bool,
        missing_capability: bool,
        wrong_window: bool,
    }
    fn node(index: usize, children: Vec<AccessibilityNode>) -> AccessibilityNode {
        let mut n = seatgeist_testkit::sample_accessibility_node();
        n.id = format!("atspi://:1.55/org/a11y/atspi/accessible/{index}");
        n.role = if index == 0 { "frame" } else { "button" }.into();
        n.name = Some(if index == 0 {
            "Test".into()
        } else {
            format!("Button {index}")
        });
        n.actions = if index == 0 {
            vec![]
        } else {
            vec![AccessibilityAction::Press]
        };
        n.children = children;
        n
    }
    fn find<'a>(node: &'a AccessibilityNode, id: &str) -> Option<&'a AccessibilityNode> {
        if node.id == id {
            Some(node)
        } else {
            node.children.iter().find_map(|n| find(n, id))
        }
    }
    impl SnapshotReader for Reader {
        fn windows(&self, _pid: u32) -> seatgeist_atspi::Result<Vec<AccessibilityMatch>> {
            Ok(vec![self.context(&node(0, vec![]).id)?])
        }
        fn context(&self, id: &str) -> seatgeist_atspi::Result<AccessibilityMatch> {
            let tree = self.tree.lock().unwrap();
            let mut n = find(&tree, id)
                .cloned()
                .ok_or_else(|| libseatgeist::SeatgeistError::Io("node removed".into()))?;
            n.children.clear();
            Ok(AccessibilityMatch {
                node: n,
                application_name: "org.example.Test".into(),
                application_bus_name: ":1.55".into(),
                process_id: Some(std::process::id()),
                window_name: Some("Test".into()),
                window_node_id: Some(node(if self.wrong_window { 999 } else { 0 }, vec![]).id),
            })
        }
        fn tree(
            &self,
            id: &str,
            depth: usize,
            max_nodes: usize,
        ) -> seatgeist_atspi::Result<TreeSample> {
            self.reads
                .lock()
                .unwrap()
                .push((id.into(), depth, max_nodes));
            fn trim(
                n: &AccessibilityNode,
                depth: usize,
                left: &mut usize,
                counts: &mut HashMap<String, usize>,
                complete: &mut bool,
            ) -> AccessibilityNode {
                *left -= 1;
                counts.insert(n.id.clone(), n.children.len());
                let mut out = n.clone();
                out.children.clear();
                if depth == 0 && !n.children.is_empty() {
                    *complete = false;
                }
                if depth > 0 {
                    for child in &n.children {
                        if *left == 0 {
                            *complete = false;
                            break;
                        }
                        out.children
                            .push(trim(child, depth - 1, left, counts, complete));
                    }
                }
                out
            }
            let tree = self.tree.lock().unwrap();
            let n = find(&tree, id).unwrap();
            let mut counts = HashMap::new();
            let mut complete = true;
            let root = trim(n, depth, &mut { max_nodes }, &mut counts, &mut complete);
            Ok(TreeSample {
                root,
                children_count: counts,
                complete,
            })
        }
        fn validate_action(&self, _: &str, _: &AccessibilityAction) -> seatgeist_atspi::Result<()> {
            if self.missing_capability {
                Err(libseatgeist::SeatgeistError::InvalidRequest(
                    "EditableText missing".into(),
                ))
            } else {
                Ok(())
            }
        }
        fn invoke(
            &self,
            _: &str,
            _: AccessibilityAction,
            _: Option<&str>,
        ) -> seatgeist_atspi::Result<()> {
            self.writes.fetch_add(1, Ordering::SeqCst);
            if self.fail_write {
                Err(libseatgeist::SeatgeistError::Io(
                    "reply lost after delivery".into(),
                ))
            } else {
                Ok(())
            }
        }
    }
    fn reader() -> Arc<Reader> {
        Arc::new(Reader {
            tree: Mutex::new(node(
                0,
                vec![node(1, vec![node(2, vec![node(3, vec![node(4, vec![])])])])],
            )),
            reads: Mutex::new(vec![]),
            writes: AtomicUsize::new(0),
            fail_write: false,
            missing_capability: false,
            wrong_window: false,
        })
    }
    fn client() -> JournalClientContext {
        JournalClientContext {
            pid: Some(std::process::id()),
            tool: Some("seatgeist-mcp".into()),
            process_name: Some("seatgeist-mcp".into()),
        }
    }
    fn window() -> WindowInfo {
        WindowInfo {
            id: "kwin-test".into(),
            app_id: Some("org.example.Test".into()),
            title: "Test".into(),
            pid: Some(std::process::id()),
            geometry: None,
            monitor_id: None,
        }
    }
    fn request() -> AccessibilitySnapshotRequest {
        AccessibilitySnapshotRequest {
            window_id: Some("kwin-test".into()),
            root_ref: None,
            skeleton: true,
            depth: 8,
            max_nodes: 100,
            interactive_only: false,
            compact: false,
        }
    }
    fn action(reference: String) -> AccessibilityRefActionRequest {
        AccessibilityRefActionRequest {
            ref_id: reference,
            action: AccessibilityAction::Press,
            text: None,
            destructive: false,
            target_guard: None,
        }
    }
    fn context<'a>(
        store: &'a SnapshotStore,
        windows: &'a MockWindowBackend,
        policy: &'a crate::AppPolicy,
    ) -> SnapshotContext<'a> {
        SnapshotContext {
            store,
            windows,
            policy,
        }
    }

    #[tokio::test]
    async fn skeleton_drill_preserves_refs_and_reads_only_requested_region_without_focus() {
        let store = SnapshotStore::default();
        let backend = MockWindowBackend::new(vec![window()], None);
        let policy = crate::AppPolicy::default();
        let ctx = context(&store, &backend, &policy);
        let reader = reader();
        let first = snapshot_inner(request(), Some(&client()), &ctx, reader.clone())
            .await
            .unwrap();
        assert!(!first.complete);
        assert_eq!(first.nodes_observed, 4);
        let cut = &first.root.children[0].children[0].children[0];
        assert_eq!(cut.children_count, 1);
        assert!(cut.subtree_truncated);
        let mut drill = request();
        drill.window_id = None;
        drill.root_ref = Some(cut.ref_id.clone());
        drill.skeleton = false;
        let second = snapshot_inner(drill, Some(&client()), &ctx, reader.clone())
            .await
            .unwrap();
        assert_eq!(first.snapshot_id, second.snapshot_id);
        assert_eq!(cut.ref_id, second.root.ref_id);
        assert!(second.complete);
        assert!(second.expires_in_ms <= first.expires_in_ms);
        assert_eq!(reader.reads.lock().unwrap()[1].0, node(3, vec![]).id);
        assert_eq!(reader.reads.lock().unwrap()[0].1, 3);
        let outcome = act_inner(
            action(second.root.children[0].ref_id.clone()),
            Some(&client()),
            &ctx,
            None,
            reader.clone(),
        )
        .await
        .unwrap();
        assert_eq!(outcome.disposition(), Some(ActionDisposition::delivered()));
        assert_eq!(reader.writes.load(Ordering::SeqCst), 1);
        assert!(backend.focused_windows().unwrap().is_empty());
        assert_eq!(backend.active_window_reads().unwrap(), 0);
        assert_eq!(backend.active_window().await.unwrap(), None);
    }

    #[tokio::test]
    async fn changed_or_removed_elements_and_foreign_owners_never_dispatch() {
        let store = SnapshotStore::default();
        let backend = MockWindowBackend::new(vec![window()], None);
        let policy = crate::AppPolicy::default();
        let ctx = context(&store, &backend, &policy);
        let reader = reader();
        let first = snapshot_inner(request(), Some(&client()), &ctx, reader.clone())
            .await
            .unwrap();
        let reference = first.root.children[0].ref_id.clone();
        let mut foreign = client();
        foreign.tool = Some("seatgeist-cli".into());
        foreign.process_name = Some("seatgeist-cli".into());
        assert!(
            store
                .resolve(&reference, Some(&foreign))
                .unwrap_err()
                .to_string()
                .contains("owner_mismatch")
        );
        reader.tree.lock().unwrap().children[0].name = Some("Replacement".into());
        assert!(
            act_inner(
                action(reference.clone()),
                Some(&client()),
                &ctx,
                None,
                reader.clone()
            )
            .await
            .is_err()
        );
        reader.tree.lock().unwrap().children.clear();
        assert!(
            act_inner(
                action(reference),
                Some(&client()),
                &ctx,
                None,
                reader.clone()
            )
            .await
            .is_err()
        );
        assert_eq!(reader.writes.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn expiry_policy_and_process_generation_are_checked_again_before_dispatch() {
        let store = SnapshotStore::default();
        let backend = MockWindowBackend::new(vec![window()], None);
        let policy = crate::AppPolicy::default();
        let ctx = context(&store, &backend, &policy);
        let reader = reader();
        let first = snapshot_inner(request(), Some(&client()), &ctx, reader.clone())
            .await
            .unwrap();
        let reference = first.root.children[0].ref_id.clone();
        let deny = crate::AppPolicy {
            deny: vec!["org.example.Test".into()],
            allow: vec![],
        };
        let denied = context(&store, &backend, &deny);
        assert!(
            act_inner(
                action(reference.clone()),
                Some(&client()),
                &denied,
                None,
                reader.clone()
            )
            .await
            .is_err()
        );
        store
            .0
            .lock()
            .unwrap()
            .get_mut(&first.snapshot_id)
            .unwrap()
            .process_generation = "reused-pid".into();
        assert!(
            act_inner(
                action(reference.clone()),
                Some(&client()),
                &ctx,
                None,
                reader.clone()
            )
            .await
            .is_err()
        );
        store
            .0
            .lock()
            .unwrap()
            .get_mut(&first.snapshot_id)
            .unwrap()
            .expires = Instant::now();
        assert!(
            store
                .resolve(&reference, Some(&client()))
                .unwrap_err()
                .to_string()
                .contains("expired")
        );
        assert_eq!(reader.writes.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn lost_write_reply_is_unknown_and_is_never_replayed() {
        let store = SnapshotStore::default();
        let backend = MockWindowBackend::new(vec![window()], None);
        let policy = crate::AppPolicy::default();
        let ctx = context(&store, &backend, &policy);
        let mut reader = reader();
        Arc::get_mut(&mut reader).unwrap().fail_write = true;
        let first = snapshot_inner(request(), Some(&client()), &ctx, reader.clone())
            .await
            .unwrap();
        let result = act_inner(
            action(first.root.children[0].ref_id.clone()),
            Some(&client()),
            &ctx,
            None,
            reader.clone(),
        )
        .await
        .unwrap();
        assert_eq!(result.disposition(), Some(ActionDisposition::unknown()));
        assert_eq!(reader.writes.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn missing_editable_interface_refuses_before_write_and_sensitive_values_are_redacted() {
        let store = SnapshotStore::default();
        let backend = MockWindowBackend::new(vec![window()], None);
        let policy = crate::AppPolicy::default();
        let ctx = context(&store, &backend, &policy);
        let mut reader = reader();
        Arc::get_mut(&mut reader).unwrap().missing_capability = true;
        {
            let mut n = reader.tree.lock().unwrap();
            n.children[0].children[0].sensitive = true;
            n.children[0].children[0].value = Some("private".into());
        }
        let first = snapshot_inner(request(), Some(&client()), &ctx, reader.clone())
            .await
            .unwrap();
        assert!(!serde_json::to_string(&first).unwrap().contains("private"));
        let mut request = action(first.root.children[0].ref_id.clone());
        request.action = AccessibilityAction::SetText;
        request.text = Some("new".into());
        assert!(
            act_inner(request, Some(&client()), &ctx, None, reader.clone())
                .await
                .is_err()
        );
        assert_eq!(reader.writes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn bare_refs_and_changed_identity_are_not_resolved_by_geometry() {
        for value in [
            "@e1",
            "@sg1:bad:e1",
            "@sg1:00000000-0000-0000-0000-000000000000:e0",
        ] {
            assert!(parse_ref(value).is_err());
        }
        let a = node(1, vec![]);
        let mut b = a.clone();
        b.id = node(2, vec![]).id;
        assert!(validate_node(&a, &b).is_err());
    }
    #[tokio::test]
    async fn owner_quota_evicts_own_oldest_and_does_not_overwrite_other_refs() {
        let store = SnapshotStore::default();
        let backend = MockWindowBackend::new(vec![window()], None);
        let policy = crate::AppPolicy::default();
        let ctx = context(&store, &backend, &policy);
        let reader = reader();
        let first = snapshot_inner(request(), Some(&client()), &ctx, reader.clone())
            .await
            .unwrap();
        let mut seed = store.0.lock().unwrap().remove(&first.snapshot_id).unwrap();
        let owner = seed.owner.clone();
        for i in 0..MAX_SNAPSHOTS {
            let mut record = seed.clone();
            record.expires =
                Instant::now() + TTL - Duration::from_millis((MAX_SNAPSHOTS - i) as u64);
            if i >= MAX_OWNER_SNAPSHOTS {
                record.owner = SessionOwnerIdentity::Tool {
                    tool: format!("other{i}"),
                    process_name: "other".into(),
                };
            }
            store.0.lock().unwrap().insert(format!("seed{i}"), record);
        }
        seed.entries.clear();
        let _new = snapshot_inner(request(), Some(&client()), &ctx, reader)
            .await
            .unwrap();
        let records = store.0.lock().unwrap();
        assert_eq!(records.len(), MAX_SNAPSHOTS);
        assert_eq!(
            records.values().filter(|r| r.owner == owner).count(),
            MAX_OWNER_SNAPSHOTS
        );
        assert!(!records.contains_key("seed0"));
        assert!(records.contains_key("seed8"));
    }

    #[tokio::test]
    async fn moved_window_ancestry_and_changed_kwin_identity_refuse_without_writes() {
        let store = SnapshotStore::default();
        let backend = MockWindowBackend::new(vec![window()], None);
        let policy = crate::AppPolicy::default();
        let ctx = context(&store, &backend, &policy);
        let mut reader = reader();
        let first = snapshot_inner(request(), Some(&client()), &ctx, reader.clone())
            .await
            .unwrap();
        let reference = first.root.children[0].ref_id.clone();
        Arc::get_mut(&mut reader).unwrap().wrong_window = true;
        assert!(
            act_inner(
                action(reference.clone()),
                Some(&client()),
                &ctx,
                None,
                reader.clone()
            )
            .await
            .is_err()
        );
        Arc::get_mut(&mut reader).unwrap().wrong_window = false;
        let mut changed = window();
        changed.app_id = Some("different.app".into());
        let backend = MockWindowBackend::new(vec![changed], None);
        let ctx = context(&store, &backend, &policy);
        assert!(
            act_inner(
                action(reference),
                Some(&client()),
                &ctx,
                None,
                reader.clone()
            )
            .await
            .is_err()
        );
        assert_eq!(reader.writes.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn node_budget_and_presentation_filters_keep_cut_regions_addressable() {
        let store = SnapshotStore::default();
        let backend = MockWindowBackend::new(vec![window()], None);
        let policy = crate::AppPolicy::default();
        let ctx = context(&store, &backend, &policy);
        let reader = reader();
        {
            let mut tree = reader.tree.lock().unwrap();
            tree.children[0].actions.clear();
            tree.children[0].name = None;
        }
        let mut req = request();
        req.max_nodes = 2;
        req.interactive_only = true;
        req.compact = true;
        let result = snapshot_inner(req, Some(&client()), &ctx, reader)
            .await
            .unwrap();
        assert!(!result.complete);
        assert_eq!(result.nodes_observed, 2);
        assert_eq!(result.root.children.len(), 1);
        assert!(result.root.children[0].subtree_truncated);
        assert!(
            store
                .resolve(&result.root.children[0].ref_id, Some(&client()))
                .is_ok()
        );
    }

    #[test]
    fn ref_actions_keep_normal_destructive_policy_and_exact_target_guard_requirements() {
        let mut request = action("@qualified:e1".into());
        let ordinary = libseatgeist::DaemonRequest::AccessibilityRefAction(request.clone());
        assert_eq!(
            crate::safety_class_for_request(&ordinary),
            libseatgeist::SafetyClass::ControlSemantic
        );
        let settings = crate::SafetySettings {
            require_focus_guard: true,
            ..Default::default()
        };
        assert!(crate::enforce_required_focus_guard(&settings, &ordinary).is_err());
        request.destructive = true;
        request.target_guard = Some(guard(&window()));
        let guarded = libseatgeist::DaemonRequest::AccessibilityRefAction(request);
        assert_eq!(
            crate::safety_class_for_request(&guarded),
            libseatgeist::SafetyClass::DestructiveAction
        );
        assert!(crate::enforce_required_focus_guard(&settings, &guarded).is_ok());
    }
    #[tokio::test]
    async fn disabling_a_control_after_observation_refuses_before_input() {
        let store = SnapshotStore::default();
        let backend = MockWindowBackend::new(vec![window()], None);
        let policy = crate::AppPolicy::default();
        let ctx = context(&store, &backend, &policy);
        let reader = reader();
        let first = snapshot_inner(request(), Some(&client()), &ctx, reader.clone())
            .await
            .unwrap();
        reader.tree.lock().unwrap().children[0].states.clear();
        assert!(
            act_inner(
                action(first.root.children[0].ref_id.clone()),
                Some(&client()),
                &ctx,
                None,
                reader.clone()
            )
            .await
            .is_err()
        );
        assert_eq!(reader.writes.load(Ordering::SeqCst), 0);
    }
}
