use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

pub fn current_euid() -> io::Result<u32> {
    let status = fs::read_to_string("/proc/self/status")?;
    parse_euid_from_proc_status(&status).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "could not parse effective uid from /proc/self/status",
        )
    })
}

pub fn default_socket_path() -> io::Result<PathBuf> {
    let runtime_dir = match env::var_os("XDG_RUNTIME_DIR") {
        Some(value) => PathBuf::from(value),
        None => PathBuf::from(format!("/run/user/{}", current_euid()?)),
    };

    Ok(runtime_dir.join("plasma-pilot").join("plasma-pilotd.sock"))
}

pub fn default_journal_path() -> io::Result<PathBuf> {
    let state_dir = match env::var_os("XDG_STATE_HOME") {
        Some(value) => PathBuf::from(value),
        None => {
            let home = env::var_os("HOME").ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "HOME is required when XDG_STATE_HOME is not set",
                )
            })?;
            PathBuf::from(home).join(".local").join("state")
        }
    };

    Ok(state_dir.join("plasma-pilot").join("journal.jsonl"))
}

pub fn default_panic_stop_path() -> io::Result<PathBuf> {
    let runtime_dir = match env::var_os("XDG_RUNTIME_DIR") {
        Some(value) => PathBuf::from(value),
        None => PathBuf::from(format!("/run/user/{}", current_euid()?)),
    };

    Ok(runtime_dir.join("plasma-pilot").join("panic-stop"))
}

pub fn parent_dir(path: &Path) -> io::Result<&Path> {
    path.parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "socket path has no parent"))
}

fn parse_euid_from_proc_status(status: &str) -> Option<u32> {
    let uid_line = status.lines().find(|line| line.starts_with("Uid:"))?;
    let mut fields = uid_line.split_whitespace().skip(1);
    let _real_uid = fields.next()?;
    let effective_uid = fields.next()?;
    effective_uid.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_effective_uid() {
        let status = "Name:\tplasma-pilotd\nUid:\t1000\t1001\t1000\t1000\n";
        assert_eq!(parse_euid_from_proc_status(status), Some(1001));
    }
}
