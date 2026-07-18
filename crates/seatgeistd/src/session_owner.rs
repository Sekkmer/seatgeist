use anyhow::{Result, bail};
use libseatgeist::JournalClientContext;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SessionOwnerScope {
    Process,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum SessionOwnerIdentity {
    Process(u32),
    Tool { tool: String, process_name: String },
}

impl SessionOwnerScope {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Process => "process",
            Self::Tool => "tool",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionOwner {
    tool: Option<String>,
    pid: u32,
    process_name: Option<String>,
    scope: SessionOwnerScope,
}

impl SessionOwner {
    pub(crate) fn from_client(client: Option<&JournalClientContext>) -> Result<Self> {
        let client = client.ok_or_else(|| {
            anyhow::anyhow!("capture session owner requires trusted Unix peer credentials")
        })?;
        let pid = client.pid.ok_or_else(|| {
            anyhow::anyhow!("capture session owner requires a trusted Unix peer pid")
        })?;
        let cli_tool_scope = client.tool.as_deref() == Some("seatgeist-cli")
            && client.process_name.as_deref() == Some("seatgeist-cli");
        Ok(Self {
            tool: client.tool.clone(),
            pid,
            process_name: client.process_name.clone(),
            scope: if cli_tool_scope {
                SessionOwnerScope::Tool
            } else {
                SessionOwnerScope::Process
            },
        })
    }

    pub(crate) fn require_matches(&self, client: Option<&JournalClientContext>) -> Result<()> {
        let requester = Self::from_client(client)?;
        if self.identity() != requester.identity() {
            bail!("session owner mismatch");
        }
        Ok(())
    }

    pub(crate) fn identity(&self) -> SessionOwnerIdentity {
        match self.scope {
            SessionOwnerScope::Process => SessionOwnerIdentity::Process(self.pid),
            SessionOwnerScope::Tool => SessionOwnerIdentity::Tool {
                tool: self.tool.clone().unwrap_or_default(),
                process_name: self.process_name.clone().unwrap_or_default(),
            },
        }
    }

    pub(crate) fn tool(&self) -> Option<&str> {
        self.tool.as_deref()
    }

    pub(crate) const fn pid(&self) -> u32 {
        self.pid
    }

    pub(crate) const fn scope(&self) -> SessionOwnerScope {
        self.scope
    }

    #[cfg(test)]
    pub(crate) fn test_process(pid: u32) -> Self {
        Self {
            tool: Some("test-client".to_string()),
            pid,
            process_name: Some("test-client".to_string()),
            scope: SessionOwnerScope::Process,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(tool: &str, pid: u32, process_name: &str) -> JournalClientContext {
        JournalClientContext {
            tool: Some(tool.to_string()),
            pid: Some(pid),
            process_name: Some(process_name.to_string()),
        }
    }

    #[test]
    fn mcp_owners_are_process_scoped() {
        let owner = SessionOwner::from_client(Some(&client("seatgeist-mcp", 100, "seatgeist-mcp")))
            .expect("owner constructs");
        assert_eq!(owner.scope(), SessionOwnerScope::Process);
        owner
            .require_matches(Some(&client("seatgeist-mcp", 100, "seatgeist-mcp")))
            .expect("same MCP process owns session");
        assert!(
            owner
                .require_matches(Some(&client("seatgeist-mcp", 101, "seatgeist-mcp")))
                .is_err()
        );
    }

    #[test]
    fn verified_cli_owners_are_tool_scoped_across_invocations() {
        let owner = SessionOwner::from_client(Some(&client("seatgeist-cli", 100, "seatgeist-cli")))
            .expect("owner constructs");
        assert_eq!(owner.scope(), SessionOwnerScope::Tool);
        owner
            .require_matches(Some(&client("seatgeist-cli", 101, "seatgeist-cli")))
            .expect("another verified CLI invocation may continue manual lifecycle");
        assert!(
            owner
                .require_matches(Some(&client("other", 101, "other")))
                .is_err()
        );
    }

    #[test]
    fn opening_requires_trusted_peer_credentials() {
        assert!(SessionOwner::from_client(None).is_err());
        let without_pid = JournalClientContext {
            tool: Some("seatgeist-mcp".to_string()),
            pid: None,
            process_name: Some("seatgeist-mcp".to_string()),
        };
        assert!(SessionOwner::from_client(Some(&without_pid)).is_err());
    }
}
