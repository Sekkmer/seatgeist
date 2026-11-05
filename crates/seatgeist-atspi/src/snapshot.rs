//! Strict, bounded reads for KWin-correlated snapshots. Unlike compatibility
//! tree reads, failures are never represented as a successfully empty subtree.
use super::*;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::Duration,
};

#[derive(Debug, Clone)]
pub struct TreeSample {
    pub root: AccessibilityNode,
    pub children_count: HashMap<String, usize>,
    pub complete: bool,
}

/// Separate boundary so snapshot/ref handling can use mock or future native
/// backends without importing a KWin or D-Bus implementation into its store.
pub trait SnapshotReader: Send + Sync {
    fn windows(&self, pid: u32) -> Result<Vec<AccessibilityMatch>>;
    fn tree(&self, node_id: &str, depth: usize, max_nodes: usize) -> Result<TreeSample>;
    fn context(&self, node_id: &str) -> Result<AccessibilityMatch>;
    /// Read-only actionability check, before the dispatch boundary.
    fn validate_action(&self, node_id: &str, action: &AccessibilityAction) -> Result<()>;
    fn invoke(&self, node_id: &str, action: AccessibilityAction, text: Option<&str>) -> Result<()>;
}

#[derive(Debug, Default)]
pub struct AtspiSnapshotReader;
const READ_TIMEOUT: Duration = Duration::from_secs(5);
impl SnapshotReader for AtspiSnapshotReader {
    fn windows(&self, pid: u32) -> Result<Vec<AccessibilityMatch>> {
        let bus = AtspiBus::connect_bounded(READ_TIMEOUT)?;
        let root = AtspiRef {
            service: ATSPI_ROOT_SERVICE.into(),
            path: ATSPI_ROOT_PATH.into(),
        };
        let mut result = Vec::new();
        for app in bus.children(&root)? {
            if bus.connection_process_id(&app.service).ok() != Some(pid) {
                continue;
            }
            let app_name = bus.name(&app)?;
            let mut pending = VecDeque::from(bus.children(&app)?);
            let mut visited = HashSet::new();
            while let Some(node) = pending.pop_front() {
                if !visited.insert(format!("{}{}", node.service, node.path)) {
                    continue;
                }
                if visited.len() > 512 {
                    return Err(invalid("window discovery budget exhausted"));
                }
                if is_window_role(&bus.role_name(&node)?) {
                    let mut budget = NodeBudget::new(1);
                    let value = bus.node_tree(&node, 0, &mut budget)?;
                    result.push(AccessibilityMatch {
                        window_name: value.name.clone(),
                        window_node_id: Some(value.id.clone()),
                        node: value,
                        application_name: app_name.clone(),
                        application_bus_name: app.service.clone(),
                        process_id: Some(pid),
                    });
                } else {
                    pending.extend(bus.children(&node)?);
                }
            }
        }
        Ok(result)
    }

    fn tree(&self, node_id: &str, depth: usize, max_nodes: usize) -> Result<TreeSample> {
        if max_nodes == 0 || max_nodes > 4096 || depth > 32 {
            return Err(invalid("snapshot bounds invalid"));
        }
        let bus = AtspiBus::connect_bounded(READ_TIMEOUT)?;
        let mut counts = HashMap::new();
        let mut complete = true;
        let mut visited = HashSet::new();
        let root = read_tree(
            &bus,
            &parse_node_id(node_id)?,
            depth,
            &mut NodeBudget::new(max_nodes),
            &mut counts,
            &mut complete,
            &mut visited,
        )?;
        Ok(TreeSample {
            root,
            children_count: counts,
            complete,
        })
    }

    fn context(&self, node_id: &str) -> Result<AccessibilityMatch> {
        let bus = AtspiBus::connect_bounded(READ_TIMEOUT)?;
        let original = parse_node_id(node_id)?;
        let original_role = bus.role_name(&original)?;
        let original_name = non_empty(bus.name(&original)?);
        let mut cursor = original.clone();
        let mut window = None;
        let mut visited = HashSet::new();
        let mut app_name = None;
        for _ in 0..128 {
            if !visited.insert(format!("{}{}", cursor.service, cursor.path)) {
                return Err(invalid("accessible parent cycle"));
            }
            let role = bus.role_name(&cursor)?;
            if window.is_none() && is_window_role(&role) {
                window = Some(cursor.clone());
            }
            if role == "application" {
                app_name = Some(bus.name(&cursor)?);
                break;
            }
            let parent =
                bus.get_property(&cursor.service, &cursor.path, ATSPI_ACCESSIBLE, "Parent")?;
            cursor = parse_object_refs(&parent)
                .into_iter()
                .next()
                .ok_or_else(|| invalid("accessible parent unavailable"))?;
            if cursor.service != original.service {
                return Err(invalid("accessible owner changed in parent chain"));
            }
        }
        let window = window.ok_or_else(|| invalid("accessible window ancestry unavailable"))?;
        let mut budget = NodeBudget::new(1);
        let node = bus.node_tree(&original, 0, &mut budget)?;
        if node.role != original_role || node.name != original_name {
            return Err(invalid("accessible changed during identity read"));
        }
        Ok(AccessibilityMatch {
            node,
            application_name: app_name
                .ok_or_else(|| invalid("accessible application ancestry unavailable"))?,
            application_bus_name: original.service.clone(),
            process_id: Some(bus.connection_process_id(&original.service)?),
            window_name: Some(bus.name(&window)?),
            window_node_id: Some(format!("atspi://{}{}", window.service, window.path)),
        })
    }

    fn validate_action(&self, node_id: &str, action: &AccessibilityAction) -> Result<()> {
        let bus = AtspiBus::connect_bounded(READ_TIMEOUT)?;
        let node = parse_node_id(node_id)?;
        if is_sensitive_role(&bus.role_name(&node)?) {
            return Err(invalid("sensitive ref action refused"));
        }
        if *action == AccessibilityAction::SetText {
            if !bus
                .interfaces(&node)?
                .iter()
                .any(|i| i == ATSPI_EDITABLE_TEXT)
            {
                return Err(invalid("ref target does not expose EditableText"));
            }
        } else {
            let names = bus.actions(&node)?;
            if !normalize_actions_for_role(&bus.role_name(&node)?, &names).contains(action) {
                return Err(invalid("ref action no longer advertised"));
            }
        }
        Ok(())
    }

    fn invoke(&self, node_id: &str, action: AccessibilityAction, text: Option<&str>) -> Result<()> {
        let bus = AtspiBus::connect_bounded(READ_TIMEOUT)?;
        let node = parse_node_id(node_id)?;
        match action {
            AccessibilityAction::SetText => {
                let text = text.ok_or_else(|| invalid("set_text requires text"))?;
                validate_set_text(text)?;
                bus.set_text(&node, text)
            }
            action => bus.invoke(&node, action),
        }
    }
}

fn invalid(message: &str) -> SeatgeistError {
    SeatgeistError::InvalidRequest(message.into())
}

fn read_tree(
    bus: &AtspiBus,
    node: &AtspiRef,
    depth: usize,
    budget: &mut NodeBudget,
    counts: &mut HashMap<String, usize>,
    complete: &mut bool,
    visited: &mut HashSet<String>,
) -> Result<AccessibilityNode> {
    let id = format!("atspi://{}{}", node.service, node.path);
    if !visited.insert(id.clone()) {
        return Err(invalid("accessible tree cycle"));
    }
    // Fail on a vanished object instead of manufacturing an "unknown" node.
    let role = bus.role_name(node)?;
    let name = non_empty(bus.name(node)?);
    let mut value = bus.node_tree(node, 0, budget)?;
    if value.role != role || value.name != name {
        return Err(invalid("accessible changed during snapshot read"));
    }
    if value.name.as_ref().is_some_and(|n| n.len() > 8192) {
        return Err(invalid("accessible identity exceeds snapshot bounds"));
    }
    if !value.sensitive
        && bus
            .interfaces(node)?
            .iter()
            .any(|i| i == ATSPI_EDITABLE_TEXT)
        && !value.actions.contains(&AccessibilityAction::SetText)
    {
        value.actions.push(AccessibilityAction::SetText);
    }
    let children = if value.sensitive {
        Vec::new()
    } else {
        bus.children(node)?
    };
    counts.insert(id, children.len());
    if depth == 0 && !children.is_empty() {
        *complete = false;
    }
    if depth > 0 {
        for child in children {
            if budget.remaining == 0 {
                *complete = false;
                break;
            }
            match read_tree(bus, &child, depth - 1, budget, counts, complete, visited) {
                Ok(child) => value.children.push(child),
                Err(_) => {
                    *complete = false;
                }
            }
        }
    }
    Ok(value)
}
