use std::{fmt, process::Child};

use anyhow::{Context, Result, bail};

use crate::commands;

const SYSTEMD_RUN: &str = "systemd-run";
const GTK_LAUNCH: &str = "gtk-launch";

pub(crate) trait DesktopEntryLauncher: fmt::Debug + Send + Sync {
    fn name(&self) -> &'static str;
    fn launch(&self, desktop_entry: &str, launch_id: &str) -> Result<Child>;
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SystemdDesktopEntryLauncher;

impl DesktopEntryLauncher for SystemdDesktopEntryLauncher {
    fn name(&self) -> &'static str {
        "systemd_user_transient_service"
    }

    fn launch(&self, desktop_entry: &str, launch_id: &str) -> Result<Child> {
        if !commands::exists(SYSTEMD_RUN) {
            bail!(
                "desktop launch backend unavailable: systemd-run is required to isolate launched applications from seatgeistd"
            );
        }
        if !commands::exists(GTK_LAUNCH) {
            bail!("desktop launch backend unavailable: gtk-launch is not installed");
        }
        let args = systemd_run_args(desktop_entry, launch_id)?;
        std::process::Command::new(SYSTEMD_RUN)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .context("start desktop entry through an isolated user transient service")
    }
}

fn systemd_run_args(desktop_entry: &str, launch_id: &str) -> Result<Vec<String>> {
    if launch_id.is_empty()
        || !launch_id
            .chars()
            .all(|character| character.is_ascii_hexdigit() || character == '-')
    {
        bail!("launch id is not a normalized UUID");
    }
    Ok(vec![
        "--user".to_string(),
        "--quiet".to_string(),
        "--collect".to_string(),
        "--service-type=exec".to_string(),
        "--property=ExitType=cgroup".to_string(),
        "--property=PartOf=graphical-session.target".to_string(),
        "--property=Slice=app.slice".to_string(),
        format!("--unit=seatgeist-launch-{launch_id}.service"),
        GTK_LAUNCH.to_string(),
        desktop_entry.to_string(),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_uses_an_independent_user_service() {
        let args = systemd_run_args(
            "org.mozilla.firefox",
            "e152d4b0-00cb-460b-a0ed-82ac8f246aed",
        )
        .expect("valid launch command");
        assert_eq!(args[0], "--user");
        assert!(args.iter().any(|arg| arg == "--property=ExitType=cgroup"));
        assert!(
            args.iter()
                .any(|arg| arg == "--property=PartOf=graphical-session.target")
        );
        assert!(args.iter().any(|arg| arg == "--property=Slice=app.slice"));
        assert_eq!(args[args.len() - 2], GTK_LAUNCH);
        assert_eq!(args[args.len() - 1], "org.mozilla.firefox");
    }

    #[test]
    fn launch_unit_name_rejects_injection() {
        assert!(systemd_run_args("firefox", "id;shutdown").is_err());
    }
}
