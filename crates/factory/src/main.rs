use crate::orchestrator_connection::types::OrchestratorConnection;

mod orchestrator_connection;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let orchestrator_connection = OrchestratorConnection::init().await?;

    loop {
        todo!();
    }
    Ok(())
}
