use std::{
    env,
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail};

pub(crate) fn exists(command: &str) -> bool {
    let Some(path) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&path).any(|directory| directory.join(command).is_file())
}

pub(crate) fn succeeds(command: &str, args: &[&str]) -> bool {
    Command::new(command)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub(crate) fn stdout(command: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(command)
        .args(args)
        .output()
        .with_context(|| format!("run {command}"))?;
    if !output.status.success() {
        bail!("{command} exited with status {}", output.status);
    }
    String::from_utf8(output.stdout).with_context(|| format!("{command} stdout is not UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probes_path_success_and_stdout_through_one_boundary() {
        assert!(exists("sh"));
        assert!(succeeds("sh", &["-c", "exit 0"]));
        assert!(succeeds(
            "sh",
            &["-c", "printf noisy-stdout; printf noisy-stderr >&2"]
        ));
        assert!(!succeeds("sh", &["-c", "exit 7"]));
        assert_eq!(
            stdout("sh", &["-c", "printf seatgeist"]).expect("stdout is captured"),
            "seatgeist"
        );
    }
}
