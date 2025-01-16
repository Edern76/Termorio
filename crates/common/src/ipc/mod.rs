mod types;
mod utils;

use std::path;
pub use types::*;
pub use utils::*;

pub fn get_factory_ipc_socket_path(factory_name: &str) -> String {
    let clean_name = factory_name.split('.').next().unwrap();
    let path = path::Path::new(&std::env::temp_dir())
        .join(clean_name)
        .to_str()
        .unwrap()
        .to_owned();
    path
}
