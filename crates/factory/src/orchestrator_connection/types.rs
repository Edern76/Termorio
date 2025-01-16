use bytes::BytesMut;
use interprocess::local_socket::tokio::{Listener, Stream};
use interprocess::local_socket::traits::tokio::{Listener as ListenerTrait, Stream as StreamTrait};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use termorio_common::constants as Constants;
use termorio_common::ipc::{
    get_factory_ipc_socket_path, open_socket, receive_from_socket, send_in_socket,
    RegistrationMessage, SocketRole, StatusUpdateMessage,
};
use termorio_common::utils::get_program_name;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::join;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use uuid::Uuid;

pub struct OrchestratorConnection {
    pub id: Uuid,
    pub address: Arc<String>,
    pub status_task_handle: Option<JoinHandle<()>>,
    pub status_socket: Arc<RwLock<Option<Stream>>>,
}

impl OrchestratorConnection {
    pub async fn init() -> Result<Self, Box<dyn std::error::Error>> {
        let exe_name = get_program_name()?;
        let buffer: Vec<u8>;
        {
            let mut inc = open_socket(
                String::from(Constants::ORCHESTRATOR_SOCKET_NAME),
                SocketRole::Client,
            )
            .await?;

            eprintln!("Connection established !");

            let message =
                bincode::serialize::<RegistrationMessage>(&RegistrationMessage::Register {
                    factory_name: exe_name,
                })?;

            send_in_socket(&mut inc, &message).await?;
            buffer = receive_from_socket(&mut inc).await?;

            eprintln!("Registration message sent !");
            eprintln!("Response gotten : {buffer:?}");
        }

        let response = bincode::deserialize::<RegistrationMessage>(&buffer)?;
        if let RegistrationMessage::Registered {
            factory_id,
            socket_name,
        } = response
        {
            eprintln!("Successfully received registration confirmation");
            let mut orchestrator_connection = OrchestratorConnection {
                address: Arc::new(socket_name),
                id: factory_id,
                status_task_handle: None,
                status_socket: Arc::new(RwLock::new(None)),
            };
            let status_task_handle = orchestrator_connection.spawn_status_task();
            orchestrator_connection.update_status_task_handle(status_task_handle);
            Ok(orchestrator_connection)
        } else {
            return Err(format!(
                "Unexpected response from registration socket: {:?}",
                response
            )
            .into());
        }
    }

    fn spawn_status_task(&self) -> JoinHandle<()> {
        let address = Arc::clone(&self.address);
        eprintln!("Trying to connect to {}", &address);
        let status_socket = Arc::clone(&self.status_socket);
        tokio::spawn(async move {
            let mut inc = match open_socket((*address).to_owned(), SocketRole::Client).await {
                Ok(listener) => listener,
                Err(e) => {
                    panic!("Failed to open socket: {}", e);
                }
            };

            let status_socket = Arc::clone(&status_socket);
            let read_socket = Arc::clone(&status_socket);
            status_socket.write().await.replace(inc);
            let lock_guard = read_socket.read().await; // Okay because we're using Tokio's RwLock
            let mut inc = lock_guard.as_ref().unwrap();

            eprintln!("Connection to status socket established");

            loop {
                let buffer = match receive_from_socket(&mut inc).await {
                    Ok(buf) => buf,
                    Err(e) => {
                        eprintln!("Failed to read status request: {}", e);
                        continue;
                    }
                };

                let incoming_message = match bincode::deserialize::<StatusUpdateMessage>(&buffer) {
                    Ok(message) => message,
                    Err(e) => {
                        eprintln!("Failed to deserialize status request: {}", e);
                        continue;
                    }
                };

                match incoming_message {
                    StatusUpdateMessage::RequestStatus => {
                        let response = match bincode::serialize::<StatusUpdateMessage>(
                            &StatusUpdateMessage::Status { status: Ok(()) },
                        ) {
                            Ok(message) => message,
                            Err(e) => {
                                eprintln!("Failed to serialize status response: {}", e);
                                continue;
                            }
                        };

                        if let Err(e) = send_in_socket(&mut inc, &response).await {
                            eprintln!("Failed to write status response: {}", e);
                        }
                    }
                    _ => {
                        eprintln!("Received unknown status request");
                    }
                }
            }
        })
    }

    fn update_status_task_handle(&mut self, handle: JoinHandle<()>) {
        self.status_task_handle = Some(handle);
    }
}

impl Drop for OrchestratorConnection {
    fn drop(&mut self) {
        if let Some(status_task_handle) = self.status_task_handle.take() {
            status_task_handle.abort();
        }

        let status_socket = Arc::clone(&self.status_socket);
        let factory_id = self.id;

        eprintln!("Spawning stopping status task");

        let (tx, rx) = std::sync::mpsc::channel::<Result<(), Box<dyn std::error::Error + Send>>>();

        tokio::spawn(async move {
            let mut lock_guard = status_socket.write().await;
            let inc = lock_guard.take();
            if let Some(mut inc) = inc {
                eprintln!("Found socket, sending stopping message");
                let message =
                    bincode::serialize::<StatusUpdateMessage>(&StatusUpdateMessage::Stopping)
                        .unwrap();
                match timeout(Duration::from_millis(5), send_in_socket(&mut inc, &message)).await {
                    Ok(result) => {
                        if let Err(e) = result {
                            let _ = tx.send(Err(Box::new(e)));
                            panic!("Failed to send stopping message");
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(Box::new(e)));
                        panic!("Failed to send stopping message");
                    }
                }
            } else {
                eprintln!("No socket found, not sending stopping message");
            }

            let _ = tx.send(Ok(()));
        });

        rx.recv()
            .expect("Could not block on oneshot channel")
            .unwrap_or_else(|e| panic!("Failed to send stopping message : {}", e));
    }
}
