mod types;
mod utils;

pub use types::*;
pub use utils::*;

pub fn get_factory_ipc_socket_path(factory_name: &str) -> String {
    format!("termorio_factory_{factory_name}")
}
