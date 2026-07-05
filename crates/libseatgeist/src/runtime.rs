use std::{
    env, fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
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

pub fn current_egid() -> io::Result<u32> {
    let status = fs::read_to_string("/proc/self/status")?;
    parse_egid_from_proc_status(&status).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "could not parse effective gid from /proc/self/status",
        )
    })
}

pub fn default_socket_path() -> io::Result<PathBuf> {
    let runtime_dir = match env::var_os("XDG_RUNTIME_DIR") {
        Some(value) => PathBuf::from(value),
        None => PathBuf::from(format!("/run/user/{}", current_euid()?)),
    };

    Ok(runtime_dir.join("seatgeist").join("seatgeistd.sock"))
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

    Ok(state_dir.join("seatgeist").join("journal.jsonl"))
}

pub fn default_panic_stop_path() -> io::Result<PathBuf> {
    let runtime_dir = match env::var_os("XDG_RUNTIME_DIR") {
        Some(value) => PathBuf::from(value),
        None => PathBuf::from(format!("/run/user/{}", current_euid()?)),
    };

    Ok(runtime_dir.join("seatgeist").join("panic-stop"))
}

pub fn default_approval_file_path() -> io::Result<PathBuf> {
    let runtime_dir = match env::var_os("XDG_RUNTIME_DIR") {
        Some(value) => PathBuf::from(value),
        None => PathBuf::from(format!("/run/user/{}", current_euid()?)),
    };

    Ok(runtime_dir.join("seatgeist").join("approvals.jsonl"))
}

pub fn default_screenshot_dir_path() -> io::Result<PathBuf> {
    let runtime_dir = match env::var_os("XDG_RUNTIME_DIR") {
        Some(value) => PathBuf::from(value),
        None => PathBuf::from(format!("/run/user/{}", current_euid()?)),
    };

    Ok(runtime_dir.join("seatgeist").join("screenshots"))
}

pub fn default_screenshot_output_path(kind: &str) -> io::Result<PathBuf> {
    let dir = default_screenshot_dir_path()?;
    let unix_time_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?
        .as_millis();
    Ok(default_screenshot_output_path_at(&dir, kind, unix_time_ms))
}

pub fn default_screenshot_output_path_at(dir: &Path, kind: &str, unix_time_ms: u128) -> PathBuf {
    dir.join(format!("{unix_time_ms}-{kind}.png"))
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

fn parse_egid_from_proc_status(status: &str) -> Option<u32> {
    let gid_line = status.lines().find(|line| line.starts_with("Gid:"))?;
    let mut fields = gid_line.split_whitespace().skip(1);
    let _real_gid = fields.next()?;
    let effective_gid = fields.next()?;
    effective_gid.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_effective_uid() {
        let status = "Name:\tseatgeistd\nUid:\t1000\t1001\t1000\t1000\n";
        assert_eq!(parse_euid_from_proc_status(status), Some(1001));
    }

    #[test]
    fn parses_effective_gid() {
        let status = "Name:\tseatgeistd\nGid:\t1000\t1002\t1000\t1000\n";
        assert_eq!(parse_egid_from_proc_status(status), Some(1002));
    }

    #[test]
    fn builds_default_screenshot_output_path() {
        assert_eq!(
            default_screenshot_output_path_at(
                Path::new("/run/user/1000/seatgeist/screenshots"),
                "tile",
                42,
            ),
            PathBuf::from("/run/user/1000/seatgeist/screenshots/42-tile.png")
        );
    }
}
