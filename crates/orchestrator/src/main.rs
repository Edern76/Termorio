use termorio_common::constants as Constants;
use termorio_orchestrator::{Orchestrator, OrchestratorConfig};
use tokio::select;

mod display;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let conf = OrchestratorConfig {
        registration_socket: String::from(Constants::ORCHESTRATOR_SOCKET_NAME),
    };

    let mut orchestrator = Orchestrator::new(conf);
    let (reg_handle, statuses_handle) = orchestrator.run().await;
    let display_handle = display::display_task(orchestrator.get_factories_ref())?;

    println!("Orchestrator running, waiting for connections...");

    // Add error handling for the join handle
    let result = select! {
        registration_result = reg_handle => {
            match registration_result {
                ok @ Ok(_) => {
                    println!("Registration task completed first (should not happen)");
                    ok
                }
                 Err(e) => {
                    println!("Registration task failed: {}", &e);
                    Err(e)
                }
            }
        }
        status_result = statuses_handle => {
            match status_result {
                ok @ Ok(_) => {
                    println!("Status task completed first (should not happen)");
                    ok
                }
                 Err(e) => {
                    println!("Status task failed: {}", &e);
                    Err(e)
                }
            }
        }
        display_result = display_handle => {
            match display_result {
                 Ok(_) => {
                    println!("Display task completed first (should not happen)");
                    Ok(())
                }
                 Err(e) => {
                    println!("Display task crashed: {}", &e);
                    Err(e)
                }
            }
        }
    };

    match result {
        Ok(_) => {
            println!("Orchestrator exiting");
            Ok(())
        }
        Err(e) => {
            println!("Orchestrator exiting due to error: {}", &e);
            Err(e.into())
        }
    }
}
