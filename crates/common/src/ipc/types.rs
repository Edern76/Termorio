use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RegistrationMessage {
    Register {
        factory_name: String,
    },
    Registered {
        factory_id: Uuid,
        socket_name: String,
    },
    Unregister {
        factory_id: Uuid,
    },
    Unregistered,
    Error {
        error: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StatusUpdateMessage {
    RequestStatus,
    Status { status: Result<(), ()> },
    Stopping,
}
