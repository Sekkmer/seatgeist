use anyhow::{Result, bail};
use clap::Args;
use libseatgeist::TargetWindowGuard;

#[derive(Debug, Clone, Args, Default)]
pub(crate) struct TargetGuardArgs {
    #[arg(long)]
    target_window_id: Option<String>,
    #[arg(long)]
    target_app_id: Option<String>,
    #[arg(long)]
    target_pid: Option<u32>,
    #[arg(long)]
    target_title_contains: Option<String>,
}

impl TargetGuardArgs {
    pub(crate) fn into_guard(self) -> Result<Option<TargetWindowGuard>> {
        let supplied = self.target_window_id.is_some()
            || self.target_app_id.is_some()
            || self.target_pid.is_some()
            || self.target_title_contains.is_some();
        if !supplied {
            return Ok(None);
        }
        let Some(expected_window_id) = self.target_window_id else {
            bail!("--target-window-id is required when a target guard is supplied");
        };
        Ok(Some(TargetWindowGuard {
            expected_window_id,
            expected_app_id: self.target_app_id,
            expected_pid: self.target_pid,
            title_contains: self.target_title_contains,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_target_arguments_produce_no_guard() {
        assert_eq!(
            TargetGuardArgs::default()
                .into_guard()
                .expect("empty target guard is valid"),
            None
        );
    }

    #[test]
    fn target_window_id_is_required_for_partial_guards() {
        let error = TargetGuardArgs {
            target_app_id: Some("org.mozilla.firefox".to_string()),
            ..TargetGuardArgs::default()
        }
        .into_guard()
        .expect_err("partial target guard must fail");

        assert!(error.to_string().contains("--target-window-id is required"));
    }

    #[test]
    fn complete_target_guard_preserves_identity_constraints() {
        let guard = TargetGuardArgs {
            target_window_id: Some("kwin-firefox-1".to_string()),
            target_app_id: Some("org.mozilla.firefox".to_string()),
            target_pid: Some(4242),
            target_title_contains: Some("Meeting".to_string()),
        }
        .into_guard()
        .expect("complete target guard is valid")
        .expect("guard is present");

        assert_eq!(guard.expected_window_id, "kwin-firefox-1");
        assert_eq!(
            guard.expected_app_id.as_deref(),
            Some("org.mozilla.firefox")
        );
        assert_eq!(guard.expected_pid, Some(4242));
        assert_eq!(guard.title_contains.as_deref(), Some("Meeting"));
    }
}
