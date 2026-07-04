use anyhow::Result;
use clap::Parser;

#[derive(Debug, Parser)]
#[command(version, about = "PlasmaPilot MCP stdio server")]
struct Args {
    #[arg(long)]
    stdio: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.stdio {
        eprintln!("plasma-pilot-mcp stdio protocol is not implemented yet");
    }
    Ok(())
}
