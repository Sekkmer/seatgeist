use anyhow::Result;
use clap::Parser;
use libplasma_pilot::{BackendCapability, CapabilitySet, HealthStatus};
use tracing::info;

#[derive(Debug, Parser)]
#[command(version, about = "PlasmaPilot local desktop-control daemon")]
struct Args {
    #[arg(long, env = "PLASMA_PILOT_SOCKET")]
    socket: Option<String>,

    #[arg(long)]
    print_capabilities: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    if args.print_capabilities {
        println!("{}", serde_json::to_string_pretty(&capabilities())?);
        return Ok(());
    }

    let socket = args
        .socket
        .unwrap_or_else(|| "$XDG_RUNTIME_DIR/plasma-pilot/plasma-pilotd.sock".to_string());
    info!(%socket, "plasma-pilotd stub started");
    println!("{}", serde_json::to_string_pretty(&health())?);
    Ok(())
}

fn health() -> HealthStatus {
    HealthStatus {
        service: "plasma-pilotd".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        status: "stub".to_string(),
    }
}

fn capabilities() -> CapabilitySet {
    CapabilitySet {
        capabilities: vec![
            BackendCapability::Screenshot,
            BackendCapability::WindowList,
            BackendCapability::WindowFocus,
            BackendCapability::ClipboardText,
        ],
    }
}
