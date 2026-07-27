use std::{
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use libseatgeist::{ToolApprovalLevel, default_panic_stop_path};
use seatgeist_policy::PolicyConfig;
use serde::Deserialize;

use crate::{keymap, window_safety::app_id_matches, xdg};

pub(crate) const DEFAULT_REQUIRE_FOCUS_GUARD: bool = true;
pub(crate) const DEFAULT_HUMAN_INPUT_QUIET_MS: u64 = 1500;
pub(crate) const DEFAULT_CONTROL_RATE_LIMIT_PER_MINUTE: u32 = 120;
pub(crate) const DEFAULT_PREVIEW_MAX_EDGE: u32 = 1600;
pub(crate) const DEFAULT_TILE_MAX_EDGE: u32 = 1600;
pub(crate) const DEFAULT_PROTECTED_APP_IDS: &[&str] = &["org.keepassxc.KeePassXC"];

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct JournalSettings {
    pub(crate) include_artifact_metadata: bool,
    pub(crate) include_error_details: bool,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct DaemonConfigFile {
    pub(crate) daemon: Option<DaemonFileConfig>,
    pub(crate) journal: Option<JournalFileConfig>,
    pub(crate) backends: Option<BackendFileConfig>,
    pub(crate) policy: Option<PolicyFileConfig>,
    pub(crate) apps: Option<AppsFileConfig>,
    pub(crate) safety: Option<SafetyFileConfig>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct DaemonFileConfig {
    pub(crate) socket: Option<String>,
    pub(crate) journal: Option<String>,
    pub(crate) panic_stop_file: Option<String>,
    pub(crate) approval_file: Option<String>,
    pub(crate) capture_restore_file: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct JournalFileConfig {
    pub(crate) include_artifact_metadata: Option<bool>,
    pub(crate) include_error_details: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct BackendFileConfig {
    pub(crate) input: Option<InputBackendPreference>,
    pub(crate) keymap: Option<keymap::FileConfig>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
#[value(rename_all = "snake_case")]
pub(crate) enum InputBackendPreference {
    #[default]
    Auto,
    KwinAgentSeat,
    PortalRemoteDesktop,
    Libei,
    Uinput,
}

impl InputBackendPreference {
    pub(crate) fn status_name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::KwinAgentSeat => "kwin_agent_seat",
            Self::PortalRemoteDesktop => "portal_remote_desktop",
            Self::Libei => "libei",
            Self::Uinput => "uinput",
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct PolicyFileConfig {
    pub(crate) default_observe: Option<ToolApprovalLevel>,
    pub(crate) default_control: Option<ToolApprovalLevel>,
    pub(crate) destructive_actions: Option<ToolApprovalLevel>,
    pub(crate) secret_fields: Option<ToolApprovalLevel>,
    pub(crate) default_clipboard_read: Option<ToolApprovalLevel>,
    pub(crate) default_clipboard_write: Option<ToolApprovalLevel>,
    #[serde(alias = "full_resolution_screenshot")]
    pub(crate) default_full_resolution_screenshot: Option<ToolApprovalLevel>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct AppsFileConfig {
    pub(crate) allow: Option<Vec<String>>,
    pub(crate) deny: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct AppPolicy {
    pub(crate) allow: Vec<String>,
    pub(crate) deny: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SafetyFileConfig {
    pub(crate) require_focus_guard: Option<bool>,
    pub(crate) pause_on_human_input: Option<bool>,
    pub(crate) human_input_activity_file: Option<String>,
    pub(crate) human_input_quiet_ms: Option<u64>,
    pub(crate) control_rate_limit_per_minute: Option<u32>,
    pub(crate) preview_max_edge: Option<u32>,
    pub(crate) tile_max_edge: Option<u32>,
    pub(crate) redact_regions: Option<Vec<RedactRegionFileConfig>>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RedactRegionFileConfig {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SafetySettings {
    pub(crate) require_focus_guard: bool,
    pub(crate) pause_on_human_input: bool,
    pub(crate) human_input_activity_file: Option<PathBuf>,
    pub(crate) human_input_quiet_ms: u64,
    pub(crate) control_rate_limit_per_minute: Option<u32>,
    pub(crate) preview_max_edge: u32,
    pub(crate) tile_max_edge: u32,
    pub(crate) screenshot_redactions: Vec<RedactRegion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RedactRegion {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

pub(crate) fn load_daemon_config(explicit_path: Option<&Path>) -> Result<DaemonConfigFile> {
    let path = explicit_path
        .map(Path::to_path_buf)
        .unwrap_or_else(default_config_path);

    if !path.exists() {
        if explicit_path.is_some() {
            bail!("config file does not exist: {}", path.display());
        }
        return Ok(DaemonConfigFile::default());
    }

    let contents = fs::read_to_string(&path)
        .with_context(|| format!("read config file {}", path.display()))?;
    toml::from_str(&contents).with_context(|| format!("parse config file {}", path.display()))
}

fn default_config_path() -> PathBuf {
    xdg::config_home().join("seatgeist/config.toml")
}

pub(crate) fn configured_path(
    cli_path: Option<PathBuf>,
    config_path: Option<&str>,
    default_path: impl FnOnce() -> std::io::Result<PathBuf>,
) -> Result<PathBuf> {
    if let Some(path) = cli_path {
        return Ok(path);
    }
    if let Some(path) = config_path {
        return expand_config_path(path);
    }
    default_path().map_err(Into::into)
}

pub(crate) fn configured_optional_path(
    cli_path: Option<PathBuf>,
    config_path: Option<&str>,
) -> Result<Option<PathBuf>> {
    if let Some(path) = cli_path {
        return Ok(Some(path));
    }
    config_path.map(expand_config_path).transpose()
}

pub(crate) fn input_backend_preference(
    cli_backend: Option<InputBackendPreference>,
    config_backend: Option<InputBackendPreference>,
) -> InputBackendPreference {
    cli_backend.or(config_backend).unwrap_or_default()
}

fn expand_config_path(value: &str) -> Result<PathBuf> {
    let mut expanded = value.to_string();
    for name in [
        "XDG_RUNTIME_DIR",
        "XDG_STATE_HOME",
        "XDG_CONFIG_HOME",
        "HOME",
    ] {
        let marker = format!("${name}");
        if expanded.contains(&marker) {
            let replacement = env::var(name)
                .with_context(|| format!("{name} is required to expand config path {value}"))?;
            expanded = expanded.replace(&marker, &replacement);
        }
    }
    Ok(PathBuf::from(expanded))
}

pub(crate) fn policy_config(
    file_policy: Option<&PolicyFileConfig>,
    allow_control: bool,
    allow_clipboard_read: bool,
    allow_full_resolution_screenshot: bool,
) -> PolicyConfig {
    let mut config = PolicyConfig::default();
    if let Some(file_policy) = file_policy {
        if let Some(level) = &file_policy.default_observe {
            config.default_observe = level.clone();
        }
        if let Some(level) = &file_policy.default_control {
            config.default_control = level.clone();
        }
        if let Some(level) = &file_policy.destructive_actions {
            config.default_destructive_actions = level.clone();
        }
        if let Some(level) = &file_policy.secret_fields {
            config.default_secret_fields = level.clone();
        }
        if let Some(level) = &file_policy.default_clipboard_read {
            config.default_clipboard_read = level.clone();
        }
        if let Some(level) = &file_policy.default_clipboard_write {
            config.default_clipboard_write = level.clone();
        }
        if let Some(level) = &file_policy.default_full_resolution_screenshot {
            config.default_full_resolution_screenshot = level.clone();
        }
    }
    if allow_control {
        config.default_control = ToolApprovalLevel::Allow;
    }
    if allow_clipboard_read {
        config.default_clipboard_read = ToolApprovalLevel::Allow;
    }
    if allow_full_resolution_screenshot {
        config.default_full_resolution_screenshot = ToolApprovalLevel::Allow;
    }
    config
}

pub(crate) fn app_policy(file_apps: Option<&AppsFileConfig>) -> AppPolicy {
    let configured_deny = file_apps
        .and_then(|apps| apps.deny.as_deref())
        .unwrap_or(&[]);
    let mut deny = DEFAULT_PROTECTED_APP_IDS
        .iter()
        .map(|app_id| (*app_id).to_string())
        .collect::<Vec<_>>();
    deny.extend(configured_deny.iter().cloned());

    AppPolicy {
        allow: normalize_app_policy_list(
            file_apps
                .and_then(|apps| apps.allow.as_deref())
                .unwrap_or(&[]),
        ),
        deny: normalize_app_policy_list(&deny),
    }
}

pub(crate) fn journal_settings(file_journal: Option<&JournalFileConfig>) -> JournalSettings {
    JournalSettings {
        include_artifact_metadata: file_journal
            .and_then(|journal| journal.include_artifact_metadata)
            .unwrap_or(false),
        include_error_details: file_journal
            .and_then(|journal| journal.include_error_details)
            .unwrap_or(false),
    }
}

fn normalize_app_policy_list(values: &[String]) -> Vec<String> {
    let mut normalized: Vec<String> = Vec::new();
    for value in values {
        let value = value.trim();
        if !value.is_empty() && !normalized.iter().any(|seen| app_id_matches(seen, value)) {
            normalized.push(value.to_string());
        }
    }
    normalized
}

pub(crate) fn safety_settings(file_safety: Option<&SafetyFileConfig>) -> Result<SafetySettings> {
    let screenshot_redactions = file_safety
        .and_then(|safety| safety.redact_regions.as_deref())
        .map(redact_regions)
        .unwrap_or_default();
    let pause_on_human_input = file_safety
        .and_then(|safety| safety.pause_on_human_input)
        .unwrap_or(false);
    let human_input_activity_file = if pause_on_human_input {
        Some(
            match file_safety.and_then(|safety| safety.human_input_activity_file.as_deref()) {
                Some(path) => expand_config_path(path)?,
                None => default_human_input_activity_path()?,
            },
        )
    } else {
        None
    };

    Ok(SafetySettings {
        require_focus_guard: file_safety
            .and_then(|safety| safety.require_focus_guard)
            .unwrap_or(DEFAULT_REQUIRE_FOCUS_GUARD),
        pause_on_human_input,
        human_input_activity_file,
        human_input_quiet_ms: file_safety
            .and_then(|safety| safety.human_input_quiet_ms)
            .unwrap_or(DEFAULT_HUMAN_INPUT_QUIET_MS),
        control_rate_limit_per_minute: file_safety
            .and_then(|safety| safety.control_rate_limit_per_minute)
            .map(|limit| if limit == 0 { None } else { Some(limit) })
            .unwrap_or(Some(DEFAULT_CONTROL_RATE_LIMIT_PER_MINUTE)),
        preview_max_edge: configured_positive_u32(
            file_safety.and_then(|safety| safety.preview_max_edge),
            DEFAULT_PREVIEW_MAX_EDGE,
            "safety.preview_max_edge",
        )?,
        tile_max_edge: configured_positive_u32(
            file_safety.and_then(|safety| safety.tile_max_edge),
            DEFAULT_TILE_MAX_EDGE,
            "safety.tile_max_edge",
        )?,
        screenshot_redactions,
    })
}

fn configured_positive_u32(value: Option<u32>, default: u32, name: &str) -> Result<u32> {
    match value {
        Some(0) => bail!("{name} must be greater than zero"),
        Some(value) => Ok(value),
        None => Ok(default),
    }
}

fn default_human_input_activity_path() -> Result<PathBuf> {
    Ok(default_panic_stop_path()?.with_file_name("human-input-active"))
}

fn redact_regions(values: &[RedactRegionFileConfig]) -> Vec<RedactRegion> {
    values
        .iter()
        .filter(|region| region.width > 0 && region.height > 0)
        .map(|region| RedactRegion {
            x: region.x,
            y: region.y,
            width: region.width,
            height: region.height,
        })
        .collect()
}
