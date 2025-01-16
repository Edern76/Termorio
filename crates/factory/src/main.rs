use crate::orchestrator_connection::types::OrchestratorConnection;
use std::time::Duration;
use tokio::signal;
use tokio::time::sleep;

mod orchestrator_connection;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let orchestrator_connection = OrchestratorConnection::init().await?;

    signal::ctrl_c().await.expect("failed to listen for event");
    eprintln!("Received Ctrl+C, exiting");
    drop(orchestrator_connection);
    Ok(())
}
