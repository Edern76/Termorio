use interprocess::local_socket::tokio::Listener;
use papaya::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Copy, Clone, PartialEq)]
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

impl FactoryConnection {
    pub fn get_copy_with_different_status(&self, new_status: FactoryStatus) -> Self {
        Self {
            name: self.name.to_owned(),
            socket: Arc::clone(&self.socket),
            status: new_status,
        }
    }
}

pub type FactoriesMap = Arc<HashMap<Uuid, FactoryConnection>>;
