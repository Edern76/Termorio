use interprocess::local_socket::tokio::{Listener, Stream};
use interprocess::local_socket::traits::tokio::{Listener as ListenerTrait, Stream as StreamTrait};
use interprocess::local_socket::{GenericNamespaced, ListenerOptions, ToNsName};
use std::io;

pub enum SocketRole {
    Server,
    Client,
}

pub async fn open_socket(name: String, role: SocketRole) -> Result<Stream, io::Error> {
    let result = match role {
        SocketRole::Server => {
            let listener_options =
                ListenerOptions::new().name(name.to_ns_name::<GenericNamespaced>()?);

            let listener = match listener_options.create_tokio() {
                Err(e) if e.kind() == io::ErrorKind::AddrInUse => Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "Address already in use",
                )),
                Err(e) => Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    format!("Failed to create listener: {}", e),
                )),
                ok_res @ Ok(_) => ok_res,
            };

            match listener {
                Ok(listener) => Ok(listener.accept().await?),
                Err(e) => Err(e),
            }
        }
        SocketRole::Client => {
            match Stream::connect(name.to_ns_name::<GenericNamespaced>()?).await {
                Ok(stream) => Ok(stream),
                Err(e) => Err(io::Error::new(
                    io::ErrorKind::ConnectionRefused,
                    format!("Failed to connect to socket: {}", e),
                )),
            }
        }
    };

    return result;
}
