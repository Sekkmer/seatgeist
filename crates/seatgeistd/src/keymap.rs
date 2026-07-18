use std::process::Command;

use libseatgeist::XkbKeymapStatus;
use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct FileConfig {
    pub(super) rules: Option<String>,
    pub(super) model: Option<String>,
    pub(super) layout: Option<String>,
    pub(super) variant: Option<String>,
    pub(super) options: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Settings {
    pub(super) rules: Option<String>,
    pub(super) model: Option<String>,
    pub(super) layout: Option<String>,
    pub(super) variant: Option<String>,
    pub(super) options: Option<String>,
}

impl Settings {
    pub(super) fn as_names(&self) -> seatgeist_eis::XkbKeymapNames<'_> {
        seatgeist_eis::XkbKeymapNames {
            rules: self.rules.as_deref(),
            model: self.model.as_deref(),
            layout: self.layout.as_deref(),
            variant: self.variant.as_deref(),
            options: self.options.as_deref(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Config {
    configured: bool,
    settings: Settings,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Resolution {
    pub(super) settings: Settings,
    pub(super) status: XkbKeymapStatus,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct KdeConfig {
    model: Option<String>,
    layout_list: Option<String>,
    variant_list: Option<String>,
    options: Option<String>,
}

pub(super) fn config(file: Option<&FileConfig>) -> Config {
    let Some(file) = file else {
        return Config::default();
    };
    Config {
        configured: true,
        settings: Settings {
            rules: clean_value(file.rules.as_deref()),
            model: clean_value(file.model.as_deref()),
            layout: clean_value(file.layout.as_deref()),
            variant: clean_value(file.variant.as_deref()),
            // An explicitly empty options string means no xkbcommon options.
            options: file.options.clone(),
        },
    }
}

pub(super) fn resolve(config: &Config) -> Resolution {
    if config.configured {
        return resolve_with_kde(config, None, None);
    }
    resolve_with_kde(config, kde_current_layout(), kde_config())
}

fn resolve_with_kde(
    config: &Config,
    kde_current_layout: Option<String>,
    kde_config: Option<KdeConfig>,
) -> Resolution {
    if config.configured {
        let settings = config.settings.clone();
        return Resolution {
            status: status(
                "config",
                &settings,
                None,
                None,
                "using explicit [backends.keymap] RMLVO names for EIS key-combo lookup",
            ),
            settings,
        };
    }

    let kde_config_layouts = kde_config
        .as_ref()
        .and_then(|config| config.layout_list.clone());
    let mut settings = Settings {
        model: kde_config.as_ref().and_then(|config| config.model.clone()),
        options: kde_config
            .as_ref()
            .and_then(|config| config.options.clone()),
        ..Settings::default()
    };

    if let Some((layout, variant)) = kde_current_layout.as_deref().and_then(parse_layout_name) {
        settings.layout = Some(layout);
        settings.variant = variant;
        return Resolution {
            status: status(
                "kde_current_layout",
                &settings,
                kde_current_layout,
                kde_config_layouts,
                "using KDE current keyboard layout DBus metadata for EIS key-combo lookup",
            ),
            settings,
        };
    }

    if let Some(config) = kde_config
        && let Some(layout) = first_csv_value(config.layout_list.as_deref())
    {
        settings.layout = Some(layout);
        settings.variant = first_csv_value(config.variant_list.as_deref());
        return Resolution {
            status: status(
                "kde_kxkbrc",
                &settings,
                kde_current_layout,
                kde_config_layouts,
                "using first configured KDE kxkbrc layout for EIS key-combo lookup; current-layout DBus metadata was unavailable",
            ),
            settings,
        };
    }

    let settings = Settings::default();
    Resolution {
        status: status(
            "xkbcommon_default",
            &settings,
            kde_current_layout,
            kde_config_layouts,
            "KDE keyboard layout metadata was unavailable; using xkbcommon defaults for EIS key-combo lookup",
        ),
        settings,
    }
}

fn status(
    source: impl Into<String>,
    settings: &Settings,
    kde_current_layout: Option<String>,
    kde_config_layouts: Option<String>,
    setup_hint: impl Into<String>,
) -> XkbKeymapStatus {
    XkbKeymapStatus {
        source: source.into(),
        rules: settings.rules.clone(),
        model: settings.model.clone(),
        layout: settings.layout.clone(),
        variant: settings.variant.clone(),
        options: settings.options.clone(),
        kde_current_layout,
        kde_config_layouts,
        setup_hint: setup_hint.into(),
    }
}

fn kde_current_layout() -> Option<String> {
    command_output(
        "qdbus6",
        &[
            "org.kde.keyboard",
            "/Layouts",
            "org.kde.KeyboardLayouts.getCurrentLayout",
        ],
    )
    .or_else(|| {
        command_output(
            "qdbus6",
            &["org.kde.keyboard", "/Layouts", "getCurrentLayout"],
        )
    })
}

fn kde_config() -> Option<KdeConfig> {
    let config = KdeConfig {
        model: kreadconfig("Model").and_then(|value| clean_value(Some(&value))),
        layout_list: kreadconfig("LayoutList").and_then(|value| clean_value(Some(&value))),
        variant_list: kreadconfig("VariantList"),
        options: kreadconfig("Options"),
    };
    (config.model.is_some()
        || config.layout_list.is_some()
        || config.variant_list.is_some()
        || config.options.is_some())
    .then_some(config)
}

fn kreadconfig(key: &str) -> Option<String> {
    command_output(
        "kreadconfig6",
        &["--file", "kxkbrc", "--group", "Layout", "--key", key],
    )
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    clean_value(Some(&value))
}

fn parse_layout_name(value: &str) -> Option<(String, Option<String>)> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Some((layout, variant)) = value
        .strip_suffix(')')
        .and_then(|value| value.split_once('('))
    {
        let layout = clean_layout_token(layout)?;
        let variant = clean_value(Some(variant)).and_then(|value| clean_layout_token(&value));
        return Some((layout, variant));
    }
    clean_layout_token(value).map(|layout| (layout, None))
}

fn clean_layout_token(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || !value
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-')
    {
        return None;
    }
    Some(value.to_string())
}

fn first_csv_value(value: Option<&str>) -> Option<String> {
    value
        .and_then(|value| value.split(',').next())
        .and_then(|value| clean_value(Some(value)))
}

fn clean_value(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_normalizes_rmlvo_names_and_preserves_empty_options() {
        assert_eq!(config(None), Config::default());
        let file = FileConfig {
            rules: Some(" evdev ".to_string()),
            model: Some(" pc105 ".to_string()),
            layout: Some(" de ".to_string()),
            variant: Some(" ".to_string()),
            options: Some("".to_string()),
        };
        assert_eq!(
            config(Some(&file)),
            Config {
                configured: true,
                settings: Settings {
                    rules: Some("evdev".to_string()),
                    model: Some("pc105".to_string()),
                    layout: Some("de".to_string()),
                    variant: None,
                    options: Some("".to_string()),
                },
            }
        );
    }

    #[test]
    fn parses_kde_layout_names_and_first_configured_value() {
        assert_eq!(
            parse_layout_name("gb(intl)"),
            Some(("gb".to_string(), Some("intl".to_string())))
        );
        assert_eq!(parse_layout_name(" us "), Some(("us".to_string(), None)));
        assert_eq!(parse_layout_name("English (US)"), None);
        assert_eq!(
            first_csv_value(Some("de(nodeadkeys),us")),
            Some("de(nodeadkeys)".to_string())
        );
        assert_eq!(first_csv_value(Some(" ,us")), None);
    }

    #[test]
    fn explicit_config_resolution_avoids_live_kde_probes() {
        let config = Config {
            configured: true,
            settings: Settings {
                rules: Some("evdev".to_string()),
                model: Some("pc105".to_string()),
                layout: Some("us".to_string()),
                variant: None,
                options: Some("".to_string()),
            },
        };
        let resolution = resolve(&config);

        assert_eq!(resolution.settings.layout.as_deref(), Some("us"));
        assert_eq!(resolution.status.source, "config");
        assert_eq!(resolution.status.layout.as_deref(), Some("us"));
        assert_eq!(resolution.status.options.as_deref(), Some(""));
        assert!(resolution.status.kde_current_layout.is_none());
    }

    #[test]
    fn current_kde_layout_wins_over_the_configured_layout_list() {
        let resolution = resolve_with_kde(
            &Config::default(),
            Some("gb(intl)".to_string()),
            Some(KdeConfig {
                model: Some("pc105".to_string()),
                layout_list: Some("de,us".to_string()),
                variant_list: Some("nodeadkeys,".to_string()),
                options: Some("grp:alt_shift_toggle".to_string()),
            }),
        );

        assert_eq!(resolution.status.source, "kde_current_layout");
        assert_eq!(resolution.settings.layout.as_deref(), Some("gb"));
        assert_eq!(resolution.settings.variant.as_deref(), Some("intl"));
        assert_eq!(resolution.settings.model.as_deref(), Some("pc105"));
        assert_eq!(
            resolution.status.kde_config_layouts.as_deref(),
            Some("de,us")
        );
    }

    #[test]
    fn invalid_current_layout_falls_back_to_first_kxkbrc_layout() {
        let resolution = resolve_with_kde(
            &Config::default(),
            Some("English (US)".to_string()),
            Some(KdeConfig {
                layout_list: Some("de,us".to_string()),
                variant_list: Some("nodeadkeys,".to_string()),
                ..KdeConfig::default()
            }),
        );

        assert_eq!(resolution.status.source, "kde_kxkbrc");
        assert_eq!(resolution.settings.layout.as_deref(), Some("de"));
        assert_eq!(resolution.settings.variant.as_deref(), Some("nodeadkeys"));
        assert_eq!(
            resolution.status.kde_current_layout.as_deref(),
            Some("English (US)")
        );
    }

    #[test]
    fn missing_kde_metadata_uses_xkbcommon_defaults() {
        let resolution = resolve_with_kde(&Config::default(), None, None);

        assert_eq!(resolution.status.source, "xkbcommon_default");
        assert_eq!(resolution.settings, Settings::default());
        assert!(resolution.status.kde_current_layout.is_none());
        assert!(resolution.status.kde_config_layouts.is_none());
    }
}
