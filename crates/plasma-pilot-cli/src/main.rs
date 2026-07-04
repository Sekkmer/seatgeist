use anyhow::Result;
use clap::{Parser, Subcommand};
use libplasma_pilot::{BackendCapability, CapabilitySet, HealthStatus};

#[derive(Debug, Parser)]
#[command(version, about = "PlasmaPilot diagnostics and manual control CLI")]
struct Cli {
    #[arg(long, env = "PLASMA_PILOT_SOCKET")]
    socket: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Doctor,
    Capabilities,
    Screenshot {
        #[arg(long)]
        output: String,
    },
    Windows,
    ActiveWindow,
    Journal {
        #[command(subcommand)]
        command: JournalCommand,
    },
}

#[derive(Debug, Subcommand)]
enum JournalCommand {
    Tail,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let _socket = cli.socket;

    match cli.command {
        Command::Doctor => println!("{}", serde_json::to_string_pretty(&stub_health())?),
        Command::Capabilities => {
            println!("{}", serde_json::to_string_pretty(&stub_capabilities())?)
        }
        Command::Screenshot { output } => {
            println!("screenshot backend is not implemented yet; requested output={output}");
        }
        Command::Windows => println!("window backend is not implemented yet"),
        Command::ActiveWindow => println!("active-window backend is not implemented yet"),
        Command::Journal {
            command: JournalCommand::Tail,
        } => println!("journal backend is not implemented yet"),
    }

    Ok(())
}

fn stub_health() -> HealthStatus {
    HealthStatus {
        service: "plasma-pilot-cli".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        status: "stub".to_string(),
    }
}

fn stub_capabilities() -> CapabilitySet {
    CapabilitySet {
        capabilities: vec![
            BackendCapability::Screenshot,
            BackendCapability::WindowList,
            BackendCapability::WindowFocus,
            BackendCapability::PointerInput,
            BackendCapability::KeyboardInput,
            BackendCapability::ClipboardText,
        ],
    }
}
