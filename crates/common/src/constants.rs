use std::time::Duration;

pub static REGISTRATION_BUFFER_SIZE: usize = 1024;
pub static STATUS_UPDATE_BUFFER_SIZE: usize = 1024;
pub static ORCHESTRATOR_EVENT_QUEUE_SIZE: usize = 100;

pub static STATUS_MESSAGE_REQUEST_RETRY_INTERVAL: Duration = Duration::from_millis(250);
pub static STATUS_MESSAGE_REQUEST_INTERVAL: Duration = Duration::from_millis(500);
pub static STATUS_MESSAGE_REQUEST_TIMEOUT: Duration = Duration::from_millis(1000);

pub static ORCHESTRATOR_SOCKET_NAME: &str = "termorio_orchestrator";
pub static STATUS_DISPLAY_INTERVAL: Duration = Duration::from_millis(200);
