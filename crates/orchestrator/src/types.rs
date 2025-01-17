use interprocess::local_socket::tokio::Listener;
use papaya::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub enum FactoryStatus {
    Running,
    Stopped,
    Dead,
    Uninitialized,
}

#[derive(Debug)]
pub struct FactoryConnection {
    pub name: String,
    pub socket: Arc<Mutex<Option<Listener>>>,
    pub status: FactoryStatus,
}

pub type FactoriesMap = Arc<HashMap<Uuid, FactoryConnection>>;
