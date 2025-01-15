use bytes::BytesMut;
use dashmap::mapref::one::{Ref, RefMut};
use dashmap::DashMap;
use interprocess::local_socket::tokio::Listener;
use interprocess::local_socket::tokio::Stream;
use interprocess::local_socket::traits::tokio::Listener as ListenerTrait;
use interprocess::local_socket::{prelude::*, GenericNamespaced, ListenerOptions};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io;
use std::io::ErrorKind;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use termorio_common::constants as Constants;
use termorio_common::ipc::{
    get_factory_ipc_socket_path, open_socket, RegistrationMessage, SocketRole, StatusUpdateMessage,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::broadcast;
use tokio::task::JoinHandle as TokioJoinHandle;
use tokio::time::{sleep, timeout};
use tokio::{join, select};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
enum FactoryStatus {
    Running,
    Stopped,
    Dead,
    Uninitialized,
}

#[derive(Debug, Clone)]
enum FactoryChangeEvent {
    Added(Uuid),
    Removed(Uuid),
    StatusChanged(Uuid, FactoryStatus),
}

struct FactoryConnection {
    name: String,
    socket: Option<Stream>,
    status: FactoryStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorConfig {
    pub registration_socket: String,
}

pub struct Orchestrator {
    factories: Arc<DashMap<Uuid, FactoryConnection>>,
    factories_events: Arc<broadcast::Sender<FactoryChangeEvent>>,
    config: OrchestratorConfig,
}

impl Orchestrator {
    pub fn new(config: OrchestratorConfig) -> Self {
        let (tx, _) =
            broadcast::channel::<FactoryChangeEvent>(Constants::ORCHESTRATOR_EVENT_QUEUE_SIZE);

        Self {
            factories: Arc::new(DashMap::new()),
            factories_events: Arc::new(tx),
            config,
        }
    }

    pub async fn run(&mut self) -> (TokioJoinHandle<()>, TokioJoinHandle<()>) {
        let registration_socket = self.config.registration_socket.clone();
        let registration_factories_arc = Arc::clone(&self.factories);
        let factories_events_arc_registration = Arc::clone(&self.factories_events);

        let status_factories_arc = Arc::clone(&self.factories);
        let factories_events_arc_status = Arc::clone(&self.factories_events);

        let registration_handle = tokio::spawn(async move {
            Self::registration_task(
                registration_socket,
                registration_factories_arc,
                factories_events_arc_registration,
            )
            .await
            .expect("Uncaught error during registration loop");
        });
        let status_spawner_handle = tokio::spawn(async move {
            Self::status_tasks_spawner_task(factories_events_arc_status, status_factories_arc)
                .await
                .expect("Uncaught error during status loop");
        });

        return (registration_handle, status_spawner_handle);
    }

    async fn status_tasks_spawner_task(
        factories_events_arc: Arc<broadcast::Sender<FactoryChangeEvent>>,
        factories_arc: Arc<DashMap<Uuid, FactoryConnection>>,
    ) -> Result<(), io::Error> {
        let sender_status = factories_events_arc.clone();
        let mut rx = factories_events_arc.subscribe();

        let mut active_tasks = HashMap::<Uuid, TokioJoinHandle<()>>::new();

        eprintln!("Status task spawner listening...");
        loop {
            let event = rx.recv().await.unwrap_or_else(|e| {
                match e {
                    broadcast::error::RecvError::Lagged(n) => {
                        panic!("Lagged {} events", n); // This puts the orchestrator in a bad state, but it'
                    }
                    _ => {
                        panic!("Failed to receive event: {}", e);
                    }
                }
            });

            match event {
                FactoryChangeEvent::Added(factory_id) => {
                    let factories_arc = Arc::clone(&factories_arc);
                    let factories_events_arc = Arc::clone(&sender_status);

                    let handle = tokio::spawn(async move {
                        if let Some(factory) = factories_arc.get_mut(&factory_id) {
                            Self::handle_factory_status_task(factory, factories_events_arc)
                                .await
                                .expect("Uncaught error during factory creation");
                        }
                    });
                    active_tasks.insert(factory_id, handle);
                }
                FactoryChangeEvent::Removed(factory_id) => {
                    let handle = active_tasks.remove(&factory_id).unwrap();
                    handle.abort();
                }
                _ => {} // Not this task's business
            }
        }
    }

    async fn handle_factory_status_task(
        mut factory: RefMut<'_, Uuid, FactoryConnection>,
        factory_sender: Arc<broadcast::Sender<FactoryChangeEvent>>,
    ) -> Result<(), io::Error> {
        let mut mut_factory = factory.value_mut();
        let name = mut_factory.name.clone();
        let mut inc = if let Some(mut socket) = mut_factory.socket.take() {
            socket
        } else {
            panic!("Factory {} socket is None", &factory.key().to_string());
        };

        let mut buffer = BytesMut::with_capacity(Constants::STATUS_UPDATE_BUFFER_SIZE);

        eprintln!("Status task listening for updates for {}", name);

        loop {
            let message = bincode::serialize(&StatusUpdateMessage::RequestStatus).unwrap();

            match inc.write_all(&message).await {
                Ok(_) => {
                    if let Err(e) = timeout(
                        Constants::STATUS_MESSAGE_REQUEST_TIMEOUT,
                        inc.read(&mut buffer),
                    )
                    .await
                    {
                        mut_factory.status = FactoryStatus::Stopped;
                        return Err(io::Error::new(
                            ErrorKind::TimedOut,
                            format!("Status update message timed out: {}", e),
                        ));
                    } else {
                        let message = match bincode::deserialize::<StatusUpdateMessage>(&buffer) {
                            Ok(message) => message,
                            Err(e) => {
                                mut_factory.status = FactoryStatus::Dead;
                                return Err(io::Error::new(
                                    ErrorKind::Other,
                                    format!("Failed to deserialize status update message: {}", e),
                                ));
                            }
                        };

                        let status = match message {
                            StatusUpdateMessage::Status { status } => match status {
                                Ok(_) => FactoryStatus::Running,
                                Err(_) => FactoryStatus::Dead,
                            },
                            _ => FactoryStatus::Dead,
                        };

                        mut_factory.status = status.clone();
                        if status != FactoryStatus::Running {
                            return Err(io::Error::new(
                                ErrorKind::Other,
                                format!("Factory {} is not running", name),
                            ));
                        }
                    }
                    sleep(Constants::STATUS_MESSAGE_REQUEST_INTERVAL).await;
                }
                Err(e) => {
                    eprintln!("Failed to send status update message: {}", e);
                    sleep(Constants::STATUS_MESSAGE_REQUEST_RETRY_INTERVAL).await;
                    continue;
                }
            }
        }
    }

    async fn registration_task(
        registration_socket: String,
        registration_factories_arc: Arc<DashMap<Uuid, FactoryConnection>>,
        factories_events_arc: Arc<broadcast::Sender<FactoryChangeEvent>>,
    ) -> Result<(), io::Error> {
        eprintln!("Starting registration task...");
        let mut stream = match open_socket(registration_socket, SocketRole::Server).await {
            Err(e) => {
                panic!("Shutting down registering task due to error : {e}");
            }
            Ok(stream) => stream,
        };

        eprintln!("Registration thread listening...");

        loop {
            let mut len_buf = [0u8; 4];
            stream.read_exact(&mut len_buf).await?;
            let response_len = u32::from_le_bytes(len_buf);
            eprintln!("Received registration message of length {}", &response_len);

            let mut buffer = vec![0u8; response_len as usize];

            match stream.read_exact(&mut buffer).await {
                Ok(_) => {
                    eprintln!("Received registration message inside match",);
                    let message = match bincode::deserialize::<RegistrationMessage>(&buffer) {
                        Ok(message) => message,
                        Err(e) => {
                            panic!("Failed to deserialize incoming registration message: {}", e)
                        }
                    };
                    let result = Self::handle_message(
                        message,
                        registration_factories_arc.clone(),
                        factories_events_arc.clone(),
                    )
                    .await
                    .unwrap_or_else(|message| message);

                    match bincode::serialize(&result) {
                        Ok(bytes) => {
                            stream.write_all(&bytes).await?;
                        }
                        Err(e) => {
                            eprintln!("Failed to serialize registration message: {}", e);
                            continue;
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to read from connection: {}", e);
                    continue;
                }
            }
            buffer.clear();
        }
    }

    async fn handle_message(
        message: RegistrationMessage,
        arc_factories: Arc<DashMap<Uuid, FactoryConnection>>,
        factories_events_arc: Arc<broadcast::Sender<FactoryChangeEvent>>,
    ) -> Result<RegistrationMessage, RegistrationMessage> {
        match message {
            RegistrationMessage::Register { factory_name } => {
                eprintln!("Registering factory {}", &factory_name);
                let uuid = Uuid::new_v4();
                let inc = match open_socket(
                    get_factory_ipc_socket_path(&factory_name),
                    SocketRole::Server,
                )
                .await
                {
                    Ok(stream) => stream,
                    Err(e) => {
                        return Err(RegistrationMessage::Error {
                            error: format!("Failed to open socket: {}", e),
                        })
                    }
                };

                arc_factories.insert(
                    uuid,
                    FactoryConnection {
                        name: factory_name.clone(),
                        socket: Some(inc),
                        status: FactoryStatus::Running,
                    },
                );

                if let Err(e) = factories_events_arc.send(FactoryChangeEvent::Added(uuid)) {
                    arc_factories.get_mut(&uuid).unwrap().status = FactoryStatus::Dead;
                    return Err(RegistrationMessage::Error {
                        error: format!("Failed to send registration event: {}", e),
                    });
                }

                Ok(RegistrationMessage::Registered {
                    factory_id: uuid,
                    socket_name: get_factory_ipc_socket_path(&factory_name),
                })
            }
            RegistrationMessage::Unregister { factory_id } => {
                arc_factories
                    .get_mut(&factory_id)
                    .and_then(|mut factory| factory.socket.take());

                if let Err(e) = factories_events_arc.send(FactoryChangeEvent::Removed(factory_id)) {
                    eprintln!("Failed to send unregistration event: {}", e); // Should be recoverable, since the socket got dropped anyway
                }

                Ok(RegistrationMessage::Unregistered)
            }
            err @ RegistrationMessage::Error { .. } => Err(err),
            _ => Err(RegistrationMessage::Error {
                error: String::from("Unknown message"),
            }),
        }
    }
}
