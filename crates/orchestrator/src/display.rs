use crate::Constants;
use std::sync::Arc;
use termorio_orchestrator::types::FactoriesMap;
use tokio::task::JoinHandle;
use tokio::time::sleep;

pub fn display_task(factories: FactoriesMap) -> Result<JoinHandle<()>, Box<dyn std::error::Error>> {
    eprintln!("Starting display status task...");
    let factories = Arc::clone(&factories);
    eprintln!("Display task map address: {:p}", Arc::as_ptr(&factories));

    let result = tokio::spawn(async move {
        loop {
            eprintln!(
                "Display loop iteration starting, map size: {}, ptr: {:p}",
                factories.len(),
                Arc::as_ptr(&factories)
            );
            let snapshot = factories.pin_owned();
            for (uuid, conn) in snapshot.iter() {
                eprintln!("- {} ({}) : {:?}", &conn.name, &uuid, &conn.status);
            }
            eprintln!("Display loop iteration complete");

            sleep(Constants::STATUS_DISPLAY_INTERVAL).await;
        }
    });

    eprintln!("Display status task started");
    Ok(result)
}
