use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use libplasma_pilot::{DaemonRequest, DaemonResponse, ScreenshotRequest, default_socket_path};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

#[derive(Debug, Parser)]
#[command(version, about = "PlasmaPilot diagnostics and manual control CLI")]
struct Cli {
    #[arg(long, env = "PLASMA_PILOT_SOCKET")]
    socket: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Doctor,
    Capabilities,
    PolicyStatus,
    Monitors,
    Screenshot {
        #[arg(long)]
        output: String,
        #[arg(long, default_value_t = 1600)]
        max_edge: u32,
        #[arg(long)]
        full_resolution: bool,
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
    let socket = match cli.socket {
        Some(path) => path,
        None => default_socket_path().context("resolve default socket path")?,
    };

    match cli.command {
        Command::Doctor => print_daemon_response(&socket, DaemonRequest::Health)?,
        Command::Capabilities => print_daemon_response(&socket, DaemonRequest::Capabilities)?,
        Command::PolicyStatus => print_daemon_response(&socket, DaemonRequest::PolicyStatus)?,
        Command::Monitors => print_daemon_response(&socket, DaemonRequest::ListMonitors)?,
        Command::Screenshot {
            output,
            max_edge,
            full_resolution,
        } => {
            print_daemon_response(
                &socket,
                DaemonRequest::Screenshot(ScreenshotRequest {
                    output: output.into(),
                    max_edge: if full_resolution {
                        None
                    } else {
                        Some(max_edge)
                    },
                    full_resolution,
                }),
            )?;
        }
        Command::Windows => println!("window backend is not implemented yet"),
        Command::ActiveWindow => println!("active-window backend is not implemented yet"),
        Command::Journal {
            command: JournalCommand::Tail,
        } => println!("journal backend is not implemented yet"),
    }

    Ok(())
}

fn print_daemon_response(socket: &PathBuf, request: DaemonRequest) -> Result<()> {
    let response = send_request(socket, request)?;
    match response {
        DaemonResponse::Error { message } => bail!("daemon returned error: {message}"),
        response => println!("{}", serde_json::to_string_pretty(&response)?),
    }
    Ok(())
}

fn send_request(socket: &PathBuf, request: DaemonRequest) -> Result<DaemonResponse> {
    let mut stream =
        UnixStream::connect(socket).with_context(|| format!("connect to {}", socket.display()))?;
    let request_line = serde_json::to_string(&request).context("serialize daemon request")?;
    stream
        .write_all(request_line.as_bytes())
        .context("write request")?;
    stream.write_all(b"\n").context("write request newline")?;

    let mut reader = BufReader::new(stream);
    let mut response_line = String::new();
    reader
        .read_line(&mut response_line)
        .context("read daemon response")?;
    serde_json::from_str(&response_line).context("parse daemon response")
}
