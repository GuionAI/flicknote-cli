use std::io::Read;

use clap::{Args, Subcommand};
use flicknote_client::dto::{NoteRouteProjectInput, NoteRouteProjectResult};
use flicknote_client::{AppRequest, DaemonClient};
use flicknote_core::error::CliError;

#[derive(Args)]
pub(crate) struct NoteArgs {
    #[command(subcommand)]
    command: NoteCommand,
}

#[derive(Subcommand)]
enum NoteCommand {
    /// Atomically apply automatic project-routing results from a JSON array on stdin
    RouteProject,
}

pub(crate) async fn run(daemon: &DaemonClient, args: &NoteArgs) -> Result<(), CliError> {
    match args.command {
        NoteCommand::RouteProject => route_project(daemon).await,
    }
}

async fn route_project(daemon: &DaemonClient) -> Result<(), CliError> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let routes: Vec<NoteRouteProjectInput> = serde_json::from_str(&input)?;
    let result: NoteRouteProjectResult = daemon.call(AppRequest::NoteRouteProject(routes)).await?;
    println!(
        "{}",
        serde_json::to_string(&result).map_err(CliError::Json)?
    );
    Ok(())
}
