use interprocess::local_socket::tokio::{Listener, Stream};
use interprocess::local_socket::traits::tokio::{Listener as ListenerTrait, Stream as StreamTrait};
use interprocess::local_socket::{GenericNamespaced, ListenerOptions, ToNsName};
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub enum SocketRole {
    Server,
    Client,
}

pub async fn send_in_socket<S>(mut socket: S, message: &[u8]) -> Result<(), io::Error>
where
    S: AsyncWrite + Unpin,
{
    let message_len = message.len() as u32;
    socket.write_all(&message_len.to_le_bytes()).await?;
    socket.write_all(message).await?;
    Ok(())
}

pub async fn receive_from_socket<S>(mut socket: S) -> Result<Vec<u8>, io::Error>
where
    S: AsyncRead + Unpin,
{
    let mut buffer = [0u8; 4];
    socket.read_exact(&mut buffer).await?;
    let message_len = u32::from_le_bytes(buffer);
    let mut buffer = vec![0u8; message_len as usize];
    socket.read_exact(&mut buffer).await?;
    Ok(buffer)
}

pub async fn get_raw_socket(name: String, role: SocketRole) -> Result<Listener, io::Error> {
    let result = match role {
        SocketRole::Client => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Client raw sockets are not supported",
        )),
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

            listener
        }
    };

    result
}
pub async fn open_socket(name: String, role: SocketRole) -> Result<Stream, io::Error> {
    let result = match role {
        SocketRole::Server => {
            let listener = get_raw_socket(name.clone(), SocketRole::Server).await?;

            return listener.accept().await;
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
