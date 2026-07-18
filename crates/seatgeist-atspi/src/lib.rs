use std::{collections::HashMap, process::Command};

mod cache;
mod events;
mod roles;

pub use events::AtspiEventBackend;

use libseatgeist::{
    AccessibilityAction, AccessibilityBounds, AccessibilityFindRequest, AccessibilityNode,
    AccessibilityTextAttributes, CoordinateSpace, SeatgeistError, TextAttribute,
};
use roles::{resolve_role_id, resolve_role_value};

pub const BACKEND_NAME: &str = "atspi";

const ATSPI_ROOT_SERVICE: &str = "org.a11y.atspi.Registry";
const ATSPI_ROOT_PATH: &str = "/org/a11y/atspi/accessible/root";
const ATSPI_ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
const ATSPI_APPLICATION: &str = "org.a11y.atspi.Application";
const ATSPI_CACHE: &str = "org.a11y.atspi.Cache";
const ATSPI_CACHE_PATH: &str = "/org/a11y/atspi/cache";
const ATSPI_COMPONENT: &str = "org.a11y.atspi.Component";
const ATSPI_ACTION: &str = "org.a11y.atspi.Action";
const ATSPI_EDITABLE_TEXT: &str = "org.a11y.atspi.EditableText";
const ATSPI_TEXT: &str = "org.a11y.atspi.Text";
const ATSPI_VALUE: &str = "org.a11y.atspi.Value";
const STATE_FOCUSED: usize = 12;
const DEFAULT_SEARCH_DEPTH: usize = 12;
const DEFAULT_VALUE_MAX_CHARS: i32 = 512;
const DEFAULT_SET_TEXT_MAX_CHARS: usize = 8192;
const BUSCTL_ACTION_TIMEOUT_ARG: &str = "--timeout=2s";

pub type Result<T> = std::result::Result<T, SeatgeistError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessibilityMatch {
    pub node: AccessibilityNode,
    pub application_name: String,
    pub application_bus_name: String,
    pub process_id: Option<u32>,
    pub window_name: Option<String>,
    pub window_node_id: Option<String>,
}

impl AccessibilityMatch {
    pub fn event_target(&self) -> Result<seatgeist_backend::AccessibilityEventTarget> {
        let window_node_id = self.window_node_id.clone().ok_or_else(|| {
            SeatgeistError::Io("AT-SPI match does not include a containing window node".to_string())
        })?;
        Ok(seatgeist_backend::AccessibilityEventTarget {
            application_bus_name: self.application_bus_name.clone(),
            node_id: self.node.id.clone(),
            window_node_id,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AtspiRef {
    service: String,
    path: String,
}

#[derive(Debug, Clone)]
struct AtspiBus {
    address: String,
}

#[derive(Debug)]
struct NodeBudget {
    remaining: usize,
}

impl NodeBudget {
    fn new(max_nodes: usize) -> Self {
        Self {
            remaining: max_nodes,
        }
    }

    fn take(&mut self) -> bool {
        if self.remaining == 0 {
            return false;
        }
        self.remaining -= 1;
        true
    }
}

pub fn available() -> bool {
    accessibility_bus_address().is_ok()
}

pub fn focused_tree(depth: usize, max_nodes: usize) -> Result<Option<AccessibilityNode>> {
    if max_nodes == 0 {
        return Err(SeatgeistError::InvalidRequest(
            "max_nodes must be greater than zero".to_string(),
        ));
    }

    let bus = AtspiBus::connect()?;
    let roots = bus.children(&AtspiRef {
        service: ATSPI_ROOT_SERVICE.to_string(),
        path: ATSPI_ROOT_PATH.to_string(),
    })?;
    let mut search_budget = NodeBudget::new(max_nodes);
    for root in roots {
        if let Ok(items) = bus.cache_items(&root)
            && let Some(focused) = cached_focused_node(&items)
        {
            let mut build_budget = NodeBudget::new(max_nodes);
            if let Ok(tree) = bus.node_tree(&focused, depth, &mut build_budget) {
                return Ok(Some(tree));
            }
        }
        if let Some(focused) = bus.find_focused(&root, DEFAULT_SEARCH_DEPTH, &mut search_budget)? {
            let mut build_budget = NodeBudget::new(max_nodes);
            return bus.node_tree(&focused, depth, &mut build_budget).map(Some);
        }
    }

    Ok(None)
}

pub fn node(node_id: &str, depth: usize, max_nodes: usize) -> Result<AccessibilityNode> {
    if max_nodes == 0 {
        return Err(SeatgeistError::InvalidRequest(
            "max_nodes must be greater than zero".to_string(),
        ));
    }
    let node = parse_node_id(node_id)?;
    let bus = AtspiBus::connect()?;
    let mut budget = NodeBudget::new(max_nodes);
    bus.node_tree(&node, depth, &mut budget)
}

pub fn find(request: AccessibilityFindRequest) -> Result<Vec<AccessibilityNode>> {
    find_with_context(request).map(|matches| {
        matches
            .into_iter()
            .map(|candidate| candidate.node)
            .collect()
    })
}

pub fn find_with_context(request: AccessibilityFindRequest) -> Result<Vec<AccessibilityMatch>> {
    validate_find_request(&request)?;
    let bus = AtspiBus::connect()?;
    let roots = bus.children(&AtspiRef {
        service: ATSPI_ROOT_SERVICE.to_string(),
        path: ATSPI_ROOT_PATH.to_string(),
    })?;
    let mut shared_budget = NodeBudget::new(request.max_nodes);
    let mut matches = Vec::new();

    for root in roots {
        if matches.len() >= request.max_results {
            break;
        }
        let app_name = bus.name(&root).unwrap_or_default();
        let process_id = bus.connection_process_id(&root.service).ok();
        if let Some(app) = request.app.as_deref()
            && !contains_case_insensitive(&app_name, app)
        {
            continue;
        }
        let mut app_budget = NodeBudget::new(request.max_nodes);
        let budget = if request.app.is_some() {
            &mut app_budget
        } else {
            &mut shared_budget
        };
        if cached_find_supported(&request)
            && let Ok(items) = bus.cache_items(&root)
        {
            bus.find_cached_matches(
                &items,
                &request,
                &app_name,
                &root.service,
                process_id,
                budget,
                &mut matches,
            );
            continue;
        }
        bus.find_matches(
            &root,
            &request,
            &app_name,
            &root.service,
            process_id,
            None,
            None,
            DEFAULT_SEARCH_DEPTH,
            budget,
            &mut matches,
        )?;
    }

    Ok(matches)
}

pub fn text_attributes(
    node_id: &str,
    offset: i32,
    include_defaults: bool,
) -> Result<AccessibilityTextAttributes> {
    if offset < 0 {
        return Err(SeatgeistError::InvalidRequest(
            "offset must be greater than or equal to zero".to_string(),
        ));
    }
    let node = parse_node_id(node_id)?;
    let bus = AtspiBus::connect()?;
    bus.text_attributes(&node, node_id, offset, include_defaults)
}

pub fn invoke(node_id: &str, action: AccessibilityAction) -> Result<()> {
    let node = parse_node_id(node_id)?;
    let bus = AtspiBus::connect()?;
    bus.invoke(&node, action)
}

pub fn set_text(node_id: &str, text: &str) -> Result<()> {
    validate_set_text(text)?;
    let node = parse_node_id(node_id)?;
    let bus = AtspiBus::connect()?;
    bus.set_text(&node, text)
}

pub fn insert_text(node_id: &str, offset: i32, text: &str) -> Result<()> {
    if offset < 0 {
        return Err(SeatgeistError::InvalidRequest(
            "offset must be greater than or equal to zero".to_string(),
        ));
    }
    validate_set_text(text)?;
    let node = parse_node_id(node_id)?;
    let bus = AtspiBus::connect()?;
    bus.insert_text(&node, offset, text)
}

pub fn delete_text(node_id: &str, start_offset: i32, end_offset: i32) -> Result<()> {
    validate_text_range(start_offset, end_offset)?;
    let node = parse_node_id(node_id)?;
    let bus = AtspiBus::connect()?;
    bus.delete_text(&node, start_offset, end_offset)
}

pub fn copy_text(node_id: &str, start_offset: i32, end_offset: i32) -> Result<()> {
    validate_text_range(start_offset, end_offset)?;
    let node = parse_node_id(node_id)?;
    let bus = AtspiBus::connect()?;
    bus.copy_text(&node, start_offset, end_offset)
}

pub fn cut_text(node_id: &str, start_offset: i32, end_offset: i32) -> Result<()> {
    validate_text_range(start_offset, end_offset)?;
    let node = parse_node_id(node_id)?;
    let bus = AtspiBus::connect()?;
    bus.cut_text(&node, start_offset, end_offset)
}

pub fn paste_text(node_id: &str, offset: i32) -> Result<()> {
    if offset < 0 {
        return Err(SeatgeistError::InvalidRequest(
            "offset must be greater than or equal to zero".to_string(),
        ));
    }
    let node = parse_node_id(node_id)?;
    let bus = AtspiBus::connect()?;
    bus.paste_text(&node, offset)
}

pub fn set_caret(node_id: &str, offset: i32) -> Result<()> {
    if offset < 0 {
        return Err(SeatgeistError::InvalidRequest(
            "offset must be greater than or equal to zero".to_string(),
        ));
    }
    let node = parse_node_id(node_id)?;
    let bus = AtspiBus::connect()?;
    bus.set_caret(&node, offset)
}

pub fn set_selection(
    node_id: &str,
    selection_num: i32,
    start_offset: i32,
    end_offset: i32,
) -> Result<()> {
    if selection_num < 0 {
        return Err(SeatgeistError::InvalidRequest(
            "selection_num must be greater than or equal to zero".to_string(),
        ));
    }
    validate_text_range(start_offset, end_offset)?;
    let node = parse_node_id(node_id)?;
    let bus = AtspiBus::connect()?;
    bus.set_selection(&node, selection_num, start_offset, end_offset)
}

pub fn set_current_value(node_id: &str, value: f64) -> Result<()> {
    if !value.is_finite() {
        return Err(SeatgeistError::InvalidRequest(
            "value must be finite".to_string(),
        ));
    }
    let node = parse_node_id(node_id)?;
    let bus = AtspiBus::connect()?;
    bus.set_current_value(&node, value)
}

impl AtspiBus {
    fn connect() -> Result<Self> {
        Ok(Self {
            address: accessibility_bus_address()?,
        })
    }

    fn find_focused(
        &self,
        node: &AtspiRef,
        depth: usize,
        budget: &mut NodeBudget,
    ) -> Result<Option<AtspiRef>> {
        if !budget.take() {
            return Ok(None);
        }
        if state_has_focused(&self.states(node).unwrap_or_default()) {
            return Ok(Some(node.clone()));
        }
        if depth == 0 {
            return Ok(None);
        }
        for child in self.children(node).unwrap_or_default() {
            if let Some(focused) = self.find_focused(&child, depth - 1, budget)? {
                return Ok(Some(focused));
            }
        }
        Ok(None)
    }

    fn node_tree(
        &self,
        node: &AtspiRef,
        depth: usize,
        budget: &mut NodeBudget,
    ) -> Result<AccessibilityNode> {
        if !budget.take() {
            return Err(SeatgeistError::InvalidRequest(
                "accessibility tree max_nodes exhausted".to_string(),
            ));
        }

        let role = self
            .role_name(node)
            .unwrap_or_else(|_| "unknown".to_string());
        let sensitive = is_sensitive_role(&role);
        let states = state_names(&self.states(node).unwrap_or_default());
        let interfaces = self.interfaces(node).unwrap_or_default();
        let available_actions = if interfaces.iter().any(|interface| interface == ATSPI_ACTION) {
            self.actions(node).unwrap_or_default()
        } else {
            Vec::new()
        };
        let bounds = if interfaces
            .iter()
            .any(|interface| interface == ATSPI_COMPONENT)
        {
            self.extents(node).ok()
        } else {
            None
        };
        let (value, value_truncated) = if sensitive {
            (None, false)
        } else {
            self.node_value(node, &interfaces).unwrap_or((None, false))
        };
        let mut children = Vec::new();
        if depth > 0 {
            for child in self.children(node).unwrap_or_default() {
                if budget.remaining == 0 {
                    break;
                }
                if let Ok(child) = self.node_tree(&child, depth - 1, budget) {
                    children.push(child);
                }
            }
        }

        Ok(AccessibilityNode {
            id: format!("atspi://{}{}", node.service, node.path),
            role: role.clone(),
            name: non_empty(self.name(node).unwrap_or_default()),
            value,
            value_truncated,
            sensitive,
            states,
            bounds,
            available_actions: available_actions.clone(),
            actions: normalize_actions_for_role(&role, &available_actions),
            children,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn find_matches(
        &self,
        node: &AtspiRef,
        request: &AccessibilityFindRequest,
        app_name: &str,
        application_bus_name: &str,
        process_id: Option<u32>,
        window_name: Option<&str>,
        window_node: Option<&AtspiRef>,
        remaining_depth: usize,
        budget: &mut NodeBudget,
        matches: &mut Vec<AccessibilityMatch>,
    ) -> Result<()> {
        if matches.len() >= request.max_results || !budget.take() {
            return Ok(());
        }

        let role = self
            .role_name(node)
            .unwrap_or_else(|_| "unknown".to_string());
        let name = self.name(node).unwrap_or_default();
        let effective_window = if is_window_role(&role) {
            non_empty_ref(&name).or(window_name)
        } else {
            window_name
        };
        let effective_window_node = if is_window_role(&role) {
            Some(node)
        } else {
            window_node
        };

        if self.node_matches(&role, &name, app_name, effective_window, request) {
            let mut build_budget = NodeBudget::new(request.max_nodes);
            if let Ok(node) = self.node_tree(node, request.depth, &mut build_budget) {
                matches.push(AccessibilityMatch {
                    node,
                    application_name: app_name.to_string(),
                    application_bus_name: application_bus_name.to_string(),
                    process_id,
                    window_name: effective_window.map(ToOwned::to_owned),
                    window_node_id: effective_window_node
                        .map(|window| format!("atspi://{}{}", window.service, window.path)),
                });
            }
        }

        if remaining_depth == 0 || matches.len() >= request.max_results {
            return Ok(());
        }

        for child in self.children(node).unwrap_or_default() {
            if matches.len() >= request.max_results || budget.remaining == 0 {
                break;
            }
            self.find_matches(
                &child,
                request,
                app_name,
                application_bus_name,
                process_id,
                effective_window,
                effective_window_node,
                remaining_depth - 1,
                budget,
                matches,
            )?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn find_cached_matches(
        &self,
        items: &[cache::CacheItem],
        request: &AccessibilityFindRequest,
        app_name: &str,
        application_bus_name: &str,
        process_id: Option<u32>,
        budget: &mut NodeBudget,
        matches: &mut Vec<AccessibilityMatch>,
    ) {
        let by_node = items
            .iter()
            .map(|item| ((item.node.service.as_str(), item.node.path.as_str()), item))
            .collect::<HashMap<_, _>>();
        let name_query = request.name_contains.as_deref().unwrap_or_default();
        for item in items {
            if matches.len() >= request.max_results {
                break;
            }
            if !contains_case_insensitive(&item.name, name_query) {
                continue;
            }
            if !budget.take() {
                break;
            }
            let role = self.role_name_from_id(&item.node, item.role);
            if !self.node_matches(&role, &item.name, app_name, None, request) {
                continue;
            }
            let (window_name, window_node) = self.cached_window_context(item, &by_node);
            if let Ok(node) = self.cached_node(item, role) {
                matches.push(AccessibilityMatch {
                    node,
                    application_name: app_name.to_string(),
                    application_bus_name: application_bus_name.to_string(),
                    process_id,
                    window_name,
                    window_node_id: window_node
                        .map(|window| format!("atspi://{}{}", window.service, window.path)),
                });
            }
        }
    }

    fn cached_window_context(
        &self,
        item: &cache::CacheItem,
        by_node: &HashMap<(&str, &str), &cache::CacheItem>,
    ) -> (Option<String>, Option<AtspiRef>) {
        let mut parent = item.parent.clone();
        for _ in 0..DEFAULT_SEARCH_DEPTH {
            let Some(candidate) = by_node.get(&(parent.service.as_str(), parent.path.as_str()))
            else {
                break;
            };
            let role = self.role_name_from_id(&candidate.node, candidate.role);
            if is_window_role(&role) {
                return (
                    non_empty(candidate.name.clone()),
                    Some(candidate.node.clone()),
                );
            }
            parent = candidate.parent.clone();
        }
        (None, None)
    }

    fn cached_node(&self, item: &cache::CacheItem, role: String) -> Result<AccessibilityNode> {
        let sensitive = is_sensitive_role(&role);
        let available_actions = if item
            .interfaces
            .iter()
            .any(|interface| interface == ATSPI_ACTION)
        {
            self.actions(&item.node).unwrap_or_default()
        } else {
            Vec::new()
        };
        let bounds = if item
            .interfaces
            .iter()
            .any(|interface| interface == ATSPI_COMPONENT)
        {
            self.extents(&item.node).ok()
        } else {
            None
        };
        let (value, value_truncated) = if sensitive {
            (None, false)
        } else {
            self.node_value(&item.node, &item.interfaces)
                .unwrap_or((None, false))
        };
        let actions = normalize_actions_for_role(&role, &available_actions);
        Ok(AccessibilityNode {
            id: format!("atspi://{}{}", item.node.service, item.node.path),
            role,
            name: non_empty(item.name.clone()),
            value,
            value_truncated,
            sensitive,
            states: state_names(&item.states),
            bounds,
            available_actions: available_actions.clone(),
            actions,
            children: Vec::new(),
        })
    }

    fn cache_items(&self, root: &AtspiRef) -> Result<Vec<cache::CacheItem>> {
        let app_address = parse_single_string(&self.call(
            &root.service,
            &root.path,
            ATSPI_APPLICATION,
            "GetApplicationBusAddress",
        )?)?;
        if app_address.trim().is_empty() {
            return Err(SeatgeistError::BackendUnavailable(
                "AT-SPI application returned an empty direct bus address".to_string(),
            ));
        }
        let output = Command::new("busctl")
            .args([
                "--address",
                &app_address,
                "call",
                ATSPI_CACHE,
                ATSPI_CACHE_PATH,
                ATSPI_CACHE,
                "GetItems",
            ])
            .output()
            .map_err(|err| {
                SeatgeistError::BackendUnavailable(format!("run busctl AT-SPI cache: {err}"))
            })?;
        cache::parse_items(&command_output(output, "busctl AT-SPI cache GetItems")?)
    }

    fn node_matches(
        &self,
        role: &str,
        name: &str,
        app_name: &str,
        window_name: Option<&str>,
        request: &AccessibilityFindRequest,
    ) -> bool {
        if let Some(query) = request.role.as_deref()
            && !role_name_matches(role, query)
        {
            return false;
        }
        if let Some(query) = request.name_contains.as_deref()
            && !contains_case_insensitive(name, query)
        {
            return false;
        }
        if let Some(query) = request.app.as_deref()
            && !contains_case_insensitive(app_name, query)
        {
            return false;
        }
        if let Some(query) = request.window_name_contains.as_deref()
            && !window_name
                .map(|name| contains_case_insensitive(name, query))
                .unwrap_or(false)
        {
            return false;
        }
        true
    }

    fn call(&self, service: &str, path: &str, interface: &str, method: &str) -> Result<String> {
        let output = Command::new("busctl")
            .args([
                "--address",
                &self.address,
                "call",
                service,
                path,
                interface,
                method,
            ])
            .output()
            .map_err(|err| SeatgeistError::BackendUnavailable(format!("run busctl: {err}")))?;
        command_output(output, "busctl AT-SPI call")
    }

    fn connection_process_id(&self, service: &str) -> Result<u32> {
        let output = self.call_with_args(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetConnectionUnixProcessID",
            &["s", service],
        )?;
        parse_connection_process_id(&output).ok_or_else(|| {
            SeatgeistError::Io(format!(
                "AT-SPI bus did not return a process id for {service}"
            ))
        })
    }

    fn call_with_args(
        &self,
        service: &str,
        path: &str,
        interface: &str,
        method: &str,
        args: &[&str],
    ) -> Result<String> {
        let output = Command::new("busctl")
            .args([
                "--address",
                &self.address,
                "call",
                service,
                path,
                interface,
                method,
            ])
            .args(args)
            .output()
            .map_err(|err| SeatgeistError::BackendUnavailable(format!("run busctl: {err}")))?;
        command_output(output, "busctl AT-SPI call")
    }

    fn get_property(
        &self,
        service: &str,
        path: &str,
        interface: &str,
        property: &str,
    ) -> Result<String> {
        let output = Command::new("busctl")
            .args([
                "--address",
                &self.address,
                "get-property",
                service,
                path,
                interface,
                property,
            ])
            .output()
            .map_err(|err| SeatgeistError::BackendUnavailable(format!("run busctl: {err}")))?;
        command_output(output, "busctl AT-SPI get-property")
    }

    fn set_property(
        &self,
        service: &str,
        path: &str,
        interface: &str,
        property: &str,
        signature: &str,
        value: &str,
    ) -> Result<String> {
        let output = Command::new("busctl")
            .args([
                "--address",
                &self.address,
                "set-property",
                service,
                path,
                interface,
                property,
                signature,
                value,
            ])
            .output()
            .map_err(|err| SeatgeistError::BackendUnavailable(format!("run busctl: {err}")))?;
        command_output(output, "busctl AT-SPI set-property")
    }

    fn children(&self, node: &AtspiRef) -> Result<Vec<AtspiRef>> {
        let output = self.call(&node.service, &node.path, ATSPI_ACCESSIBLE, "GetChildren")?;
        Ok(parse_object_refs(&output))
    }

    fn role_name(&self, node: &AtspiRef) -> Result<String> {
        match self.call(&node.service, &node.path, ATSPI_ACCESSIBLE, "GetRole") {
            Ok(output) => resolve_role_id(&output, || self.localized_role_name(node)),
            Err(_) => {
                let output =
                    self.call(&node.service, &node.path, ATSPI_ACCESSIBLE, "GetRoleName")?;
                resolve_role_name(&output, || self.localized_role_name(node))
            }
        }
    }

    fn role_name_from_id(&self, node: &AtspiRef, role_id: u32) -> String {
        resolve_role_value(role_id, || self.localized_role_name(node))
            .unwrap_or_else(|_| "unknown".to_string())
    }

    fn localized_role_name(&self, node: &AtspiRef) -> Result<String> {
        let localized = self.call(
            &node.service,
            &node.path,
            ATSPI_ACCESSIBLE,
            "GetLocalizedRoleName",
        )?;
        parse_single_string(&localized)
    }

    fn name(&self, node: &AtspiRef) -> Result<String> {
        let output = self.get_property(&node.service, &node.path, ATSPI_ACCESSIBLE, "Name")?;
        parse_single_string(&output)
    }

    fn states(&self, node: &AtspiRef) -> Result<Vec<u32>> {
        let output = self.call(&node.service, &node.path, ATSPI_ACCESSIBLE, "GetState")?;
        Ok(parse_uint_array(&output))
    }

    fn interfaces(&self, node: &AtspiRef) -> Result<Vec<String>> {
        let output = self.call(&node.service, &node.path, ATSPI_ACCESSIBLE, "GetInterfaces")?;
        Ok(parse_strings(&output))
    }

    fn extents(&self, node: &AtspiRef) -> Result<AccessibilityBounds> {
        let output = self.call_with_args(
            &node.service,
            &node.path,
            ATSPI_COMPONENT,
            "GetExtents",
            &["u", "0"],
        )?;
        let (x, y, width, height) = parse_extents(&output)?;
        Ok(AccessibilityBounds {
            x,
            y,
            width,
            height,
            space: CoordinateSpace::LogicalPixel,
        })
    }

    fn actions(&self, node: &AtspiRef) -> Result<Vec<String>> {
        let output = action_query_command(&self.address, node)
            .output()
            .map_err(|err| SeatgeistError::BackendUnavailable(format!("run busctl: {err}")))?;
        let output = command_output(output, "busctl AT-SPI action query")?;
        Ok(parse_action_names(&output))
    }

    fn text_attributes(
        &self,
        node: &AtspiRef,
        node_id: &str,
        offset: i32,
        include_defaults: bool,
    ) -> Result<AccessibilityTextAttributes> {
        let role = self
            .role_name(node)
            .unwrap_or_else(|_| "unknown".to_string());
        if is_sensitive_role(&role) {
            return Err(SeatgeistError::PolicyDenied(
                "refusing to read text attributes on sensitive accessibility node".to_string(),
            ));
        }
        let interfaces = self.interfaces(node)?;
        if !interfaces.iter().any(|interface| interface == ATSPI_TEXT) {
            return Err(SeatgeistError::InvalidRequest(
                "node does not expose org.a11y.atspi.Text".to_string(),
            ));
        }

        let offset = offset.to_string();
        let include_defaults = if include_defaults { "true" } else { "false" };
        let output = self.call_with_args(
            &node.service,
            &node.path,
            ATSPI_TEXT,
            "GetAttributeRun",
            &["ib", &offset, include_defaults],
        )?;
        let (attributes, start_offset, end_offset) = parse_text_attributes(&output)?;
        Ok(AccessibilityTextAttributes {
            node_id: node_id.to_string(),
            start_offset,
            end_offset,
            attributes,
        })
    }

    fn invoke(&self, node: &AtspiRef, action: AccessibilityAction) -> Result<()> {
        let role = self
            .role_name(node)
            .unwrap_or_else(|_| "unknown".to_string());
        let actions = self.actions(node)?;
        let Some(index) = action_index_for_role(&role, &actions, &action) else {
            return Err(SeatgeistError::InvalidRequest(format!(
                "node does not expose {} action",
                action.as_str()
            )));
        };
        let index_string = index.to_string();
        let output = self.call_with_args(
            &node.service,
            &node.path,
            ATSPI_ACTION,
            "DoAction",
            &["i", &index_string],
        )?;
        if parse_bool_value(&output)? {
            Ok(())
        } else {
            Err(SeatgeistError::BackendUnavailable(format!(
                "AT-SPI DoAction({index}) returned false"
            )))
        }
    }

    fn set_text(&self, node: &AtspiRef, text: &str) -> Result<()> {
        let role = self
            .role_name(node)
            .unwrap_or_else(|_| "unknown".to_string());
        if is_sensitive_role(&role) {
            return Err(SeatgeistError::PolicyDenied(
                "refusing to set text on sensitive accessibility node".to_string(),
            ));
        }
        let interfaces = self.interfaces(node)?;
        if !interfaces
            .iter()
            .any(|interface| interface == ATSPI_EDITABLE_TEXT)
        {
            return Err(SeatgeistError::InvalidRequest(
                "node does not expose org.a11y.atspi.EditableText".to_string(),
            ));
        }
        let output = self.call_with_args(
            &node.service,
            &node.path,
            ATSPI_EDITABLE_TEXT,
            "SetTextContents",
            &["s", text],
        )?;
        if parse_bool_value(&output)? {
            Ok(())
        } else {
            Err(SeatgeistError::BackendUnavailable(
                "AT-SPI SetTextContents returned false".to_string(),
            ))
        }
    }

    fn insert_text(&self, node: &AtspiRef, offset: i32, text: &str) -> Result<()> {
        let role = self
            .role_name(node)
            .unwrap_or_else(|_| "unknown".to_string());
        if is_sensitive_role(&role) {
            return Err(SeatgeistError::PolicyDenied(
                "refusing to insert text on sensitive accessibility node".to_string(),
            ));
        }
        let interfaces = self.interfaces(node)?;
        if !interfaces
            .iter()
            .any(|interface| interface == ATSPI_EDITABLE_TEXT)
        {
            return Err(SeatgeistError::InvalidRequest(
                "node does not expose org.a11y.atspi.EditableText".to_string(),
            ));
        }
        let length = text.len().to_string();
        let offset = offset.to_string();
        let output = self.call_with_args(
            &node.service,
            &node.path,
            ATSPI_EDITABLE_TEXT,
            "InsertText",
            &["i", &offset, "s", text, "i", &length],
        )?;
        if parse_bool_value(&output)? {
            Ok(())
        } else {
            Err(SeatgeistError::BackendUnavailable(
                "AT-SPI InsertText returned false".to_string(),
            ))
        }
    }

    fn delete_text(&self, node: &AtspiRef, start_offset: i32, end_offset: i32) -> Result<()> {
        self.editable_text_range_action(node, start_offset, end_offset, "DeleteText", "delete")
    }

    fn copy_text(&self, node: &AtspiRef, start_offset: i32, end_offset: i32) -> Result<()> {
        self.editable_text_range_action(node, start_offset, end_offset, "CopyText", "copy")
    }

    fn cut_text(&self, node: &AtspiRef, start_offset: i32, end_offset: i32) -> Result<()> {
        self.editable_text_range_action(node, start_offset, end_offset, "CutText", "cut")
    }

    fn editable_text_range_action(
        &self,
        node: &AtspiRef,
        start_offset: i32,
        end_offset: i32,
        method: &str,
        action: &str,
    ) -> Result<()> {
        let role = self
            .role_name(node)
            .unwrap_or_else(|_| "unknown".to_string());
        if is_sensitive_role(&role) {
            return Err(SeatgeistError::PolicyDenied(format!(
                "refusing to {action} text on sensitive accessibility node"
            )));
        }
        let interfaces = self.interfaces(node)?;
        if !interfaces
            .iter()
            .any(|interface| interface == ATSPI_EDITABLE_TEXT)
        {
            return Err(SeatgeistError::InvalidRequest(
                "node does not expose org.a11y.atspi.EditableText".to_string(),
            ));
        }
        let start = start_offset.to_string();
        let end = end_offset.to_string();
        let output = self.call_with_args(
            &node.service,
            &node.path,
            ATSPI_EDITABLE_TEXT,
            method,
            &["ii", &start, &end],
        )?;
        if parse_bool_value(&output)? {
            Ok(())
        } else {
            Err(SeatgeistError::BackendUnavailable(format!(
                "AT-SPI {method} returned false"
            )))
        }
    }

    fn paste_text(&self, node: &AtspiRef, offset: i32) -> Result<()> {
        let role = self
            .role_name(node)
            .unwrap_or_else(|_| "unknown".to_string());
        if is_sensitive_role(&role) {
            return Err(SeatgeistError::PolicyDenied(
                "refusing to paste text on sensitive accessibility node".to_string(),
            ));
        }
        let interfaces = self.interfaces(node)?;
        if !interfaces
            .iter()
            .any(|interface| interface == ATSPI_EDITABLE_TEXT)
        {
            return Err(SeatgeistError::InvalidRequest(
                "node does not expose org.a11y.atspi.EditableText".to_string(),
            ));
        }
        let offset = offset.to_string();
        let output = self.call_with_args(
            &node.service,
            &node.path,
            ATSPI_EDITABLE_TEXT,
            "PasteText",
            &["i", &offset],
        )?;
        if parse_bool_value(&output)? {
            Ok(())
        } else {
            Err(SeatgeistError::BackendUnavailable(
                "AT-SPI PasteText returned false".to_string(),
            ))
        }
    }

    fn set_caret(&self, node: &AtspiRef, offset: i32) -> Result<()> {
        self.ensure_text_control_node(node, "set caret")?;
        let offset = offset.to_string();
        let output = self.call_with_args(
            &node.service,
            &node.path,
            ATSPI_TEXT,
            "SetCaretOffset",
            &["i", &offset],
        )?;
        if parse_bool_value(&output)? {
            Ok(())
        } else {
            Err(SeatgeistError::BackendUnavailable(
                "AT-SPI SetCaretOffset returned false".to_string(),
            ))
        }
    }

    fn set_selection(
        &self,
        node: &AtspiRef,
        selection_num: i32,
        start_offset: i32,
        end_offset: i32,
    ) -> Result<()> {
        self.ensure_text_control_node(node, "set selection")?;
        let selection_num = selection_num.to_string();
        let start = start_offset.to_string();
        let end = end_offset.to_string();
        let output = self.call_with_args(
            &node.service,
            &node.path,
            ATSPI_TEXT,
            "SetSelection",
            &["iii", &selection_num, &start, &end],
        )?;
        if parse_bool_value(&output)? {
            Ok(())
        } else {
            Err(SeatgeistError::BackendUnavailable(
                "AT-SPI SetSelection returned false".to_string(),
            ))
        }
    }

    fn ensure_text_control_node(&self, node: &AtspiRef, action: &str) -> Result<()> {
        let role = self
            .role_name(node)
            .unwrap_or_else(|_| "unknown".to_string());
        if is_sensitive_role(&role) {
            return Err(SeatgeistError::PolicyDenied(format!(
                "refusing to {action} on sensitive accessibility node"
            )));
        }
        let interfaces = self.interfaces(node)?;
        if !interfaces.iter().any(|interface| interface == ATSPI_TEXT) {
            return Err(SeatgeistError::InvalidRequest(
                "node does not expose org.a11y.atspi.Text".to_string(),
            ));
        }
        Ok(())
    }

    fn set_current_value(&self, node: &AtspiRef, value: f64) -> Result<()> {
        let role = self
            .role_name(node)
            .unwrap_or_else(|_| "unknown".to_string());
        if is_sensitive_role(&role) {
            return Err(SeatgeistError::PolicyDenied(
                "refusing to set value on sensitive accessibility node".to_string(),
            ));
        }
        let interfaces = self.interfaces(node)?;
        if !interfaces.iter().any(|interface| interface == ATSPI_VALUE) {
            return Err(SeatgeistError::InvalidRequest(
                "node does not expose org.a11y.atspi.Value".to_string(),
            ));
        }
        let value = value.to_string();
        self.set_property(
            &node.service,
            &node.path,
            ATSPI_VALUE,
            "CurrentValue",
            "d",
            &value,
        )?;
        Ok(())
    }

    fn node_value(&self, node: &AtspiRef, interfaces: &[String]) -> Result<(Option<String>, bool)> {
        if interfaces.iter().any(|interface| interface == ATSPI_TEXT) {
            return self.text_value(node);
        }
        if interfaces.iter().any(|interface| interface == ATSPI_VALUE) {
            let output =
                self.get_property(&node.service, &node.path, ATSPI_VALUE, "CurrentValue")?;
            return Ok((Some(parse_scalar_value(&output)?), false));
        }
        Ok((None, false))
    }

    fn text_value(&self, node: &AtspiRef) -> Result<(Option<String>, bool)> {
        let output = self.get_property(&node.service, &node.path, ATSPI_TEXT, "CharacterCount")?;
        let character_count = parse_i32_value(&output)?.max(0);
        if character_count == 0 {
            return Ok((None, false));
        }
        let end = character_count.min(DEFAULT_VALUE_MAX_CHARS);
        let end_string = end.to_string();
        let text = self.call_with_args(
            &node.service,
            &node.path,
            ATSPI_TEXT,
            "GetText",
            &["ii", "0", &end_string],
        )?;
        Ok((
            non_empty(parse_single_string(&text)?),
            character_count > end,
        ))
    }
}

fn accessibility_bus_address() -> Result<String> {
    let output = Command::new("busctl")
        .args([
            "--user",
            "call",
            "org.a11y.Bus",
            "/org/a11y/bus",
            "org.a11y.Bus",
            "GetAddress",
        ])
        .output()
        .map_err(|err| SeatgeistError::BackendUnavailable(format!("run busctl: {err}")))?;
    parse_single_string(&command_output(output, "busctl org.a11y.Bus GetAddress")?)
}

fn validate_find_request(request: &AccessibilityFindRequest) -> Result<()> {
    if request.max_results == 0 {
        return Err(SeatgeistError::InvalidRequest(
            "max_results must be greater than zero".to_string(),
        ));
    }
    if request.max_nodes == 0 {
        return Err(SeatgeistError::InvalidRequest(
            "max_nodes must be greater than zero".to_string(),
        ));
    }
    if request.role.is_none()
        && request.name_contains.is_none()
        && request.app.is_none()
        && request.window_name_contains.is_none()
    {
        return Err(SeatgeistError::InvalidRequest(
            "at least one accessibility find filter is required".to_string(),
        ));
    }
    Ok(())
}

fn cached_find_supported(request: &AccessibilityFindRequest) -> bool {
    request.app.is_some()
        && request.name_contains.is_some()
        && request.window_name_contains.is_none()
        && request.depth == 0
}

fn cached_focused_node(items: &[cache::CacheItem]) -> Option<AtspiRef> {
    items
        .iter()
        .find(|item| state_has_focused(&item.states))
        .map(|item| item.node.clone())
}

fn validate_set_text(text: &str) -> Result<()> {
    let char_count = text.chars().count();
    if char_count > DEFAULT_SET_TEXT_MAX_CHARS {
        return Err(SeatgeistError::InvalidRequest(format!(
            "text exceeds {DEFAULT_SET_TEXT_MAX_CHARS} character limit"
        )));
    }
    Ok(())
}

fn validate_text_range(start_offset: i32, end_offset: i32) -> Result<()> {
    if start_offset < 0 {
        return Err(SeatgeistError::InvalidRequest(
            "start_offset must be greater than or equal to zero".to_string(),
        ));
    }
    if end_offset <= start_offset {
        return Err(SeatgeistError::InvalidRequest(
            "end_offset must be greater than start_offset".to_string(),
        ));
    }
    Ok(())
}

fn parse_node_id(node_id: &str) -> Result<AtspiRef> {
    let rest = node_id.strip_prefix("atspi://").ok_or_else(|| {
        SeatgeistError::InvalidRequest(format!("invalid AT-SPI node id: {node_id}"))
    })?;
    let Some((service, path)) = rest.split_once('/') else {
        return Err(SeatgeistError::InvalidRequest(format!(
            "invalid AT-SPI node id: {node_id}"
        )));
    };
    if service.is_empty() || path.is_empty() {
        return Err(SeatgeistError::InvalidRequest(format!(
            "invalid AT-SPI node id: {node_id}"
        )));
    }
    Ok(AtspiRef {
        service: service.to_string(),
        path: format!("/{path}"),
    })
}

fn command_output(output: std::process::Output, context: &str) -> Result<String> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SeatgeistError::BackendUnavailable(format!(
            "{context} exited with status {}: {stderr}",
            output.status
        )));
    }
    String::from_utf8(output.stdout).map_err(|err| {
        SeatgeistError::BackendUnavailable(format!("{context} output was not UTF-8: {err}"))
    })
}

fn parse_object_refs(output: &str) -> Vec<AtspiRef> {
    parse_strings(output)
        .chunks_exact(2)
        .map(|chunk| AtspiRef {
            service: chunk[0].clone(),
            path: chunk[1].clone(),
        })
        .collect()
}

fn parse_single_string(output: &str) -> Result<String> {
    parse_strings(output).into_iter().next().ok_or_else(|| {
        SeatgeistError::InvalidRequest(format!("expected string in AT-SPI output: {output}"))
    })
}

fn resolve_role_name<F>(standard_output: &str, localized: F) -> Result<String>
where
    F: FnOnce() -> Result<String>,
{
    let standard = parse_single_string(standard_output)?;
    if standard.trim().is_empty() {
        localized()
    } else {
        Ok(standard)
    }
}

fn action_query_command(address: &str, node: &AtspiRef) -> Command {
    let mut command = Command::new("busctl");
    command.args([
        BUSCTL_ACTION_TIMEOUT_ARG,
        "--address",
        address,
        "call",
        &node.service,
        &node.path,
        ATSPI_ACTION,
        "GetActions",
    ]);
    command
}

fn parse_strings(input: &str) -> Vec<String> {
    let mut strings = Vec::new();
    let mut rest = input;
    while let Some((value, next)) = parse_quoted(rest) {
        strings.push(value);
        rest = next;
    }
    strings
}

fn parse_quoted(input: &str) -> Option<(String, &str)> {
    let bytes = input.as_bytes();
    let start = bytes.iter().position(|byte| *byte == b'"')?;
    let mut output = Vec::new();
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                let rest = &input[index + 1..];
                return String::from_utf8(output).ok().map(|value| (value, rest));
            }
            b'\\' => {
                index += 1;
                if index >= bytes.len() {
                    return None;
                }
                if index + 2 < bytes.len()
                    && bytes[index].is_ascii_digit()
                    && bytes[index + 1].is_ascii_digit()
                    && bytes[index + 2].is_ascii_digit()
                {
                    let octal = std::str::from_utf8(&bytes[index..index + 3]).ok()?;
                    let value = u8::from_str_radix(octal, 8).ok()?;
                    output.push(value);
                    index += 3;
                    continue;
                }
                output.push(bytes[index]);
                index += 1;
            }
            byte => {
                output.push(byte);
                index += 1;
            }
        }
    }
    None
}

fn parse_uint_array(output: &str) -> Vec<u32> {
    let mut parts = output.split_whitespace();
    if parts.next() != Some("au") {
        return Vec::new();
    }
    let count = parts
        .next()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    parts
        .take(count)
        .filter_map(|value| value.parse::<u32>().ok())
        .collect()
}

fn parse_connection_process_id(output: &str) -> Option<u32> {
    output
        .split_whitespace()
        .find_map(|part| part.parse::<u32>().ok())
}

fn parse_extents(output: &str) -> Result<(i32, i32, u32, u32)> {
    let values = output
        .trim()
        .strip_prefix("(iiii)")
        .unwrap_or(output)
        .split_whitespace()
        .filter_map(|value| value.parse::<i32>().ok())
        .collect::<Vec<_>>();
    if values.len() != 4 {
        return Err(SeatgeistError::InvalidRequest(format!(
            "expected AT-SPI extents tuple: {output}"
        )));
    }
    Ok((
        values[0],
        values[1],
        values[2].max(0) as u32,
        values[3].max(0) as u32,
    ))
}

fn parse_i32_value(output: &str) -> Result<i32> {
    output
        .split_whitespace()
        .rev()
        .find_map(|value| value.parse::<i32>().ok())
        .ok_or_else(|| SeatgeistError::InvalidRequest(format!("expected i32 value: {output}")))
}

fn parse_scalar_value(output: &str) -> Result<String> {
    output
        .split_whitespace()
        .last()
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| SeatgeistError::InvalidRequest(format!("expected scalar value: {output}")))
}

fn parse_bool_value(output: &str) -> Result<bool> {
    match output.split_whitespace().last() {
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        _ => Err(SeatgeistError::InvalidRequest(format!(
            "expected bool value: {output}"
        ))),
    }
}

fn parse_text_attributes(output: &str) -> Result<(Vec<TextAttribute>, i32, i32)> {
    let attributes = parse_strings(output)
        .chunks_exact(2)
        .filter_map(|chunk| {
            non_empty(chunk[0].clone()).map(|name| TextAttribute {
                name,
                value: chunk[1].clone(),
            })
        })
        .collect::<Vec<_>>();
    let values = unquote_for_numeric_parse(output)
        .split_whitespace()
        .filter_map(|value| value.parse::<i32>().ok())
        .collect::<Vec<_>>();
    let [.., start_offset, end_offset] = values.as_slice() else {
        return Err(SeatgeistError::InvalidRequest(format!(
            "expected AT-SPI text attribute range: {output}"
        )));
    };
    Ok((attributes, *start_offset, *end_offset))
}

fn unquote_for_numeric_parse(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut in_quote = false;
    let mut escaped = false;
    for character in input.chars() {
        if in_quote {
            output.push(' ');
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_quote = false;
            }
        } else if character == '"' {
            in_quote = true;
            output.push(' ');
        } else {
            output.push(character);
        }
    }
    output
}

fn parse_action_names(output: &str) -> Vec<String> {
    parse_strings(output)
        .chunks_exact(3)
        .filter_map(|chunk| non_empty(chunk[0].clone()))
        .collect()
}

fn state_has_focused(words: &[u32]) -> bool {
    state_bit(words, STATE_FOCUSED)
}

fn state_names(words: &[u32]) -> Vec<String> {
    STATE_NAMES
        .iter()
        .enumerate()
        .filter(|(index, _)| state_bit(words, *index))
        .map(|(_, name)| (*name).to_string())
        .collect()
}

fn state_bit(words: &[u32], index: usize) -> bool {
    let word = index / 32;
    let bit = index % 32;
    words
        .get(word)
        .map(|value| value & (1_u32 << bit) != 0)
        .unwrap_or(false)
}

fn normalize_actions(actions: &[String]) -> Vec<AccessibilityAction> {
    let mut normalized = Vec::new();
    for action in actions {
        let candidate = if action_name_matches(action, &AccessibilityAction::Press) {
            Some(AccessibilityAction::Press)
        } else if action_name_matches(action, &AccessibilityAction::SetText) {
            Some(AccessibilityAction::SetText)
        } else if action_name_matches(action, &AccessibilityAction::Focus) {
            Some(AccessibilityAction::Focus)
        } else if action_name_matches(action, &AccessibilityAction::Select) {
            Some(AccessibilityAction::Select)
        } else {
            None
        };
        if let Some(candidate) = candidate
            && !normalized.contains(&candidate)
        {
            normalized.push(candidate);
        }
    }
    normalized
}

fn normalize_actions_for_role(role: &str, actions: &[String]) -> Vec<AccessibilityAction> {
    let mut normalized = normalize_actions(actions);
    if normalized.is_empty()
        && action_index_for_role(role, actions, &AccessibilityAction::Press).is_some()
    {
        normalized.push(AccessibilityAction::Press);
    }
    normalized
}

fn action_index_for_role(
    role: &str,
    actions: &[String],
    action: &AccessibilityAction,
) -> Option<usize> {
    actions
        .iter()
        .position(|candidate| action_name_matches(candidate, action))
        .or_else(|| {
            (*action == AccessibilityAction::Press
                && matches!(role.to_ascii_lowercase().as_str(), "button" | "push button")
                && actions.len() == 1
                && action_label_is_empty(&actions[0]))
            .then_some(0)
        })
}

fn action_label_is_empty(label: &str) -> bool {
    label.trim().trim_matches(';').trim().is_empty()
}

fn action_name_matches(action_name: &str, action: &AccessibilityAction) -> bool {
    let lower = action_name.to_ascii_lowercase();
    match action {
        AccessibilityAction::Press => {
            lower.contains("press")
                || lower.contains("click")
                || lower.contains("activate")
                || lower.contains("default")
        }
        AccessibilityAction::SetText => lower.contains("set") && lower.contains("text"),
        AccessibilityAction::Focus => lower.contains("focus"),
        AccessibilityAction::Select => lower.contains("select"),
    }
}

fn role_name_matches(candidate: &str, query: &str) -> bool {
    candidate.eq_ignore_ascii_case(query)
        || (query.eq_ignore_ascii_case("button") && candidate.eq_ignore_ascii_case("push button"))
}

fn is_sensitive_role(role: &str) -> bool {
    role.eq_ignore_ascii_case("password text") || role.eq_ignore_ascii_case("password")
}

fn is_window_role(role: &str) -> bool {
    matches!(
        role.to_ascii_lowercase().as_str(),
        "frame" | "dialog" | "window" | "alert"
    )
}

fn contains_case_insensitive(value: &str, query: &str) -> bool {
    value
        .to_ascii_lowercase()
        .contains(&query.to_ascii_lowercase())
}

fn non_empty(value: String) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

fn non_empty_ref(value: &str) -> Option<&str> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

const STATE_NAMES: &[&str] = &[
    "invalid",
    "active",
    "armed",
    "busy",
    "checked",
    "collapsed",
    "defunct",
    "editable",
    "enabled",
    "expandable",
    "expanded",
    "focusable",
    "focused",
    "has_tooltip",
    "horizontal",
    "iconified",
    "modal",
    "multi_line",
    "multiselectable",
    "opaque",
    "pressed",
    "resizable",
    "selectable",
    "selected",
    "sensitive",
    "showing",
    "single_line",
    "stale",
    "transient",
    "vertical",
    "visible",
    "manages_descendants",
    "indeterminate",
    "required",
    "truncated",
    "animated",
    "invalid_entry",
    "supports_autocompletion",
    "selectable_text",
    "is_default",
    "visited",
    "checkable",
    "has_popup",
    "read_only",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_object_refs_from_busctl_output() {
        let refs = parse_object_refs(
            r#"a(so) 2 ":1.23" "/org/a11y/atspi/accessible/root" ":1.37" "/org/a11y/atspi/accessible/1""#,
        );
        assert_eq!(
            refs,
            vec![
                AtspiRef {
                    service: ":1.23".to_string(),
                    path: "/org/a11y/atspi/accessible/root".to_string(),
                },
                AtspiRef {
                    service: ":1.37".to_string(),
                    path: "/org/a11y/atspi/accessible/1".to_string(),
                },
            ]
        );
    }

    #[test]
    fn parses_accessibility_bus_connection_process_id() {
        assert_eq!(parse_connection_process_id("u 4242\n"), Some(4242));
        assert_eq!(parse_connection_process_id("u invalid\n"), None);
    }

    #[test]
    fn decodes_busctl_octal_escaped_strings() {
        let value =
            parse_single_string(r#"s "outfit.txt  \342\200\224 Kate""#).expect("string parses");
        assert_eq!(value, "outfit.txt  - Kate".replace('-', "\u{2014}"));
    }

    #[test]
    fn maps_state_bitset_to_names() {
        let states = state_names(&[1_u32 << STATE_FOCUSED]);
        assert_eq!(states, vec!["focused"]);
        assert!(state_has_focused(&[1_u32 << STATE_FOCUSED]));
    }

    #[test]
    fn uses_localized_role_when_webkit_returns_an_empty_standard_role() {
        let role = resolve_role_name("s \"\"\n", || Ok("push button".to_string()))
            .expect("localized fallback resolves");
        assert_eq!(role, "push button");

        let role = resolve_role_name("s \"entry\"\n", || {
            Err(SeatgeistError::InvalidRequest(
                "localized fallback must not run for a standard role".to_string(),
            ))
        })
        .expect("standard role resolves");
        assert_eq!(role, "entry");
    }

    #[test]
    fn bounds_atspi_action_queries() {
        let node = AtspiRef {
            service: ":1.42".to_string(),
            path: "/org/a11y/atspi/accessible/7".to_string(),
        };
        let command = action_query_command("unix:path=/run/user/1000/at-spi/bus_1", &node);
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            args.first().map(String::as_str),
            Some(BUSCTL_ACTION_TIMEOUT_ARG)
        );
        assert!(args.iter().any(|arg| arg == "GetActions"));
    }

    #[test]
    fn parses_component_extents() {
        let extents = parse_extents("(iiii) 10 20 640 480").expect("extents parse");
        assert_eq!(extents, (10, 20, 640, 480));
    }

    #[test]
    fn parses_action_names_from_triples() {
        let actions = parse_action_names(r#"a(sss) 2 "click" "" "" "press" "desc" "Ctrl+P""#);
        assert_eq!(actions, vec!["click", "press"]);
        assert_eq!(
            normalize_actions(&actions),
            vec![AccessibilityAction::Press]
        );
    }

    #[test]
    fn parses_text_attributes_from_dictionary_and_offsets() {
        let (attributes, start_offset, end_offset) =
            parse_text_attributes(r#"a{ss} 2 "weight" "bold" "style" "italic" 3 9"#)
                .expect("text attributes parse");
        assert_eq!(
            attributes,
            vec![
                TextAttribute {
                    name: "weight".to_string(),
                    value: "bold".to_string(),
                },
                TextAttribute {
                    name: "style".to_string(),
                    value: "italic".to_string(),
                },
            ]
        );
        assert_eq!(start_offset, 3);
        assert_eq!(end_offset, 9);
    }

    #[test]
    fn parses_text_attributes_from_tuple_wrapped_output() {
        let (attributes, start_offset, end_offset) =
            parse_text_attributes(r#"(a{ss}ii) 1 "fg-color" "rgb(1,2,3)" 0 12"#)
                .expect("text attributes parse");
        assert_eq!(
            attributes,
            vec![TextAttribute {
                name: "fg-color".to_string(),
                value: "rgb(1,2,3)".to_string(),
            }]
        );
        assert_eq!(start_offset, 0);
        assert_eq!(end_offset, 12);
    }

    #[test]
    fn parses_text_attribute_offsets_after_quoted_numbers() {
        let (_, start_offset, end_offset) =
            parse_text_attributes(r#"a{ss} 1 "level" "2" 4 8"#).expect("text attributes parse");
        assert_eq!(start_offset, 4);
        assert_eq!(end_offset, 8);
    }

    #[test]
    fn parses_numeric_property_values() {
        assert_eq!(parse_i32_value("i 42").expect("i32 parses"), 42);
        assert_eq!(parse_i32_value("v i 7").expect("variant i32 parses"), 7);
        assert_eq!(
            parse_scalar_value("d 0.75").expect("double scalar parses"),
            "0.75"
        );
        assert!(parse_bool_value("b true").expect("bool parses"));
        assert!(!parse_bool_value("b false").expect("bool parses"));
    }

    #[test]
    fn parses_atspi_node_id() {
        let node =
            parse_node_id("atspi://:1.42/org/a11y/atspi/accessible/7").expect("node id parses");
        assert_eq!(node.service, ":1.42");
        assert_eq!(node.path, "/org/a11y/atspi/accessible/7");
    }

    #[test]
    fn matches_normalized_action_names() {
        assert!(action_name_matches("click", &AccessibilityAction::Press));
        assert!(action_name_matches(
            "set text",
            &AccessibilityAction::SetText
        ));
        assert!(action_name_matches(
            "grab focus",
            &AccessibilityAction::Focus
        ));
        assert!(action_name_matches("select", &AccessibilityAction::Select));
    }

    #[test]
    fn maps_one_unlabeled_firefox_button_action_to_press_only() {
        let unlabeled = vec![";;".to_string()];
        assert_eq!(
            normalize_actions_for_role("button", &unlabeled),
            vec![AccessibilityAction::Press]
        );
        assert_eq!(
            action_index_for_role("button", &unlabeled, &AccessibilityAction::Press),
            Some(0)
        );
        assert!(normalize_actions_for_role("text", &unlabeled).is_empty());
        assert_eq!(
            action_index_for_role("button", &unlabeled, &AccessibilityAction::Focus),
            None
        );
        assert!(normalize_actions_for_role("button", &[";;".into(), ";;".into()]).is_empty());
    }

    #[test]
    fn generic_button_query_matches_qt_push_button_role() {
        assert!(role_name_matches("button", "button"));
        assert!(role_name_matches("push button", "button"));
        assert!(!role_name_matches("toggle button", "button"));
        assert!(!role_name_matches("button", "push button"));
    }

    #[test]
    fn validates_set_text_limit() {
        validate_set_text("").expect("empty text can clear a field");
        let long = "x".repeat(DEFAULT_SET_TEXT_MAX_CHARS + 1);
        let err = validate_set_text(&long).expect_err("long text is rejected");
        assert!(err.to_string().contains("character limit"));
    }

    #[test]
    fn validates_text_range_for_selection() {
        validate_text_range(0, 1).expect("valid range passes");
        let err = validate_text_range(-1, 1).expect_err("negative start is invalid");
        assert!(err.to_string().contains("start_offset"));
        let err = validate_text_range(2, 2).expect_err("empty range is invalid");
        assert!(err.to_string().contains("end_offset"));
    }

    #[test]
    fn validates_find_requires_a_filter() {
        let err = validate_find_request(&AccessibilityFindRequest {
            role: None,
            name_contains: None,
            app: None,
            window_name_contains: None,
            depth: 0,
            max_results: 1,
            max_nodes: 1,
        })
        .expect_err("empty query is invalid");
        assert!(err.to_string().contains("at least one"));
    }

    #[test]
    fn cache_fast_path_requires_narrow_depth_zero_query() {
        let mut request = AccessibilityFindRequest {
            role: Some("button".to_string()),
            name_contains: Some("Save".to_string()),
            app: Some("Firefox".to_string()),
            window_name_contains: None,
            depth: 0,
            max_results: 5,
            max_nodes: 512,
        };
        assert!(cached_find_supported(&request));

        request.depth = 1;
        assert!(!cached_find_supported(&request));
        request.depth = 0;
        request.window_name_contains = Some("Document".to_string());
        assert!(!cached_find_supported(&request));
        request.window_name_contains = None;
        request.name_contains = None;
        assert!(!cached_find_supported(&request));
    }

    #[test]
    fn cached_name_prefilter_does_not_starve_later_same_app_roots() {
        let bus = AtspiBus {
            address: "unused-for-nonmatches".to_string(),
        };
        let items = (0..600)
            .map(|index| cache::CacheItem {
                node: AtspiRef {
                    service: ":1.42".to_string(),
                    path: format!("/node/{index}"),
                },
                parent: AtspiRef {
                    service: ":1.42".to_string(),
                    path: "/root".to_string(),
                },
                interfaces: Vec::new(),
                name: format!("Unrelated {index}"),
                role: 0,
                states: Vec::new(),
            })
            .collect::<Vec<_>>();
        let request = AccessibilityFindRequest {
            role: Some("button".to_string()),
            name_contains: Some("Wanted".to_string()),
            app: Some("Firefox".to_string()),
            window_name_contains: None,
            depth: 0,
            max_results: 5,
            max_nodes: 1,
        };
        let mut budget = NodeBudget::new(1);
        let mut matches = Vec::new();

        bus.find_cached_matches(
            &items,
            &request,
            "Firefox",
            ":1.42",
            Some(42),
            &mut budget,
            &mut matches,
        );

        assert_eq!(budget.remaining, 1);
        assert!(matches.is_empty());
    }

    #[test]
    fn finds_focused_node_from_bulk_cache_states() {
        let focused_word = 1_u32 << STATE_FOCUSED;
        let output = format!(
            "a((so)(so)(so)iiassusau) 1 \":1.42\" \
             \"/org/a11y/atspi/accessible/7\" \":1.42\" \
             \"/org/a11y/atspi/accessible/root\" \":1.42\" \
             \"/org/a11y/atspi/accessible/2\" 0 0 1 \
             \"org.a11y.atspi.Accessible\" \"Editor\" 60 \"\" 1 {focused_word}"
        );
        let items = cache::parse_items(&output).expect("cache fixture parses");

        let focused = cached_focused_node(&items).expect("focused cache item found");
        assert_eq!(focused.service, ":1.42");
        assert_eq!(focused.path, "/org/a11y/atspi/accessible/7");
    }

    #[test]
    fn matches_case_insensitive_substrings() {
        assert!(contains_case_insensitive("System Settings", "settings"));
        assert!(!contains_case_insensitive("Kate", "firefox"));
    }
}
