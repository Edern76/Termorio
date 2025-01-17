pub mod types;

use crate::types::{FactoriesMap, FactoryConnection, FactoryStatus};
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
    get_factory_ipc_socket_path, get_raw_socket, open_socket, receive_from_socket, send_in_socket,
    RegistrationMessage, SocketRole, StatusUpdateMessage,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::broadcast;
use tokio::task::JoinHandle as TokioJoinHandle;
use tokio::time::{sleep, timeout};
use tokio::{join, select};
use uuid::Uuid;

#[derive(Debug, Clone)]
enum FactoryChangeEvent {
    Added(Uuid),
    Removed(Uuid),
    StatusChanged(Uuid, FactoryStatus),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorConfig {
    pub registration_socket: String,
}

pub struct Orchestrator {
    pub factories: FactoriesMap,
    factories_events: Arc<broadcast::Sender<FactoryChangeEvent>>,
    config: OrchestratorConfig,
}

impl Orchestrator {
    pub fn new(config: OrchestratorConfig) -> Self {
        let (tx, _) =
            broadcast::channel::<FactoryChangeEvent>(Constants::ORCHESTRATOR_EVENT_QUEUE_SIZE);

        Self {
            factories: Arc::new(papaya::HashMap::new()),
            factories_events: Arc::new(tx),
            config,
        }
    }

    pub fn get_factories_ref(&self) -> FactoriesMap {
        Arc::clone(&self.factories)
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
        factories_arc: FactoriesMap,
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
                        let factories_arc = factories_arc.pin_owned();
                        if let Some(factory) = factories_arc.get_key_value(&factory_id) {
                            let name = factory.1.name.clone();
                            match Self::handle_factory_status_task(factory, factories_events_arc)
                                .await
                            {
                                Ok(_) => {}
                                Err(e) if e.kind() == io::ErrorKind::TimedOut => {
                                    eprintln!("Factory {name} timed out",);
                                }
                                Err(e) => {
                                    panic!("Uncaught error during factory creation: {}", e);
                                }
                            }
                        }
                    });
                    active_tasks.insert(factory_id, handle);
                }
                FactoryChangeEvent::Removed(factory_id) => {
                    let handle = active_tasks.remove(&factory_id).unwrap();
                    handle.abort();
                }
                FactoryChangeEvent::StatusChanged(factory_id, new_status) => {
                    let factories_arc = factories_arc.pin_owned();
                    factories_arc.update(factory_id, |conn| {
                        conn.get_copy_with_different_status(new_status)
                    });
                } // Not this task's business
            }
        }
    }

    // TODO : Replace some Err(e) with Ok(), or handle them upstream
    // If the factory dies and this catches it, then it is intended behaviour and we should gracefully end the task
    async fn handle_factory_status_task(
        mut factory: (&Uuid, &FactoryConnection),
        factory_sender: Arc<broadcast::Sender<FactoryChangeEvent>>,
    ) -> Result<(), io::Error> {
        let (uuid_ref) = factory.0;
        let (factory_ref) = factory.1;

        let name;
        let uuid;
        let mut listener;

        uuid = uuid_ref.clone().to_owned();

        let immut_factory = factory_ref; // TODO : Other name for that
        name = immut_factory.name.clone();
        let mut lock = immut_factory.socket.lock().await;
        listener = if let Some(mut socket) = lock.as_mut() {
            socket
        } else {
            panic!("Factory {} socket is None", &uuid);
        };

        eprintln!("Status task listening for updates for {}", name);
        let mut inc = listener.accept().await?;

        loop {
            let message = bincode::serialize(&StatusUpdateMessage::RequestStatus).unwrap();

            match send_in_socket(&mut inc, &message).await {
                Ok(_) => {
                    match timeout(
                        Constants::STATUS_MESSAGE_REQUEST_TIMEOUT,
                        receive_from_socket(&mut inc),
                    )
                    .await
                    {
                        Err(e) => {
                            {
                                if let Err(e) = factory_sender.send(
                                    FactoryChangeEvent::StatusChanged(uuid, FactoryStatus::Stopped),
                                ) {
                                    eprintln!("Failed to send status change event: {}", e);
                                }
                            }
                            return Err(io::Error::new(
                                ErrorKind::TimedOut,
                                format!("Status update message timed out: {}", e),
                            ));
                        }
                        Ok(res) => match res {
                            Ok(buffer) => {
                                let message =
                                    match bincode::deserialize::<StatusUpdateMessage>(&buffer) {
                                        Ok(message) => message,
                                        Err(e) => {
                                            {
                                                if let Err(e) = factory_sender.send(
                                                    FactoryChangeEvent::StatusChanged(
                                                        uuid,
                                                        FactoryStatus::Dead,
                                                    ),
                                                ) {
                                                    eprintln!(
                                                        "Failed to send status change event : {}",
                                                        e
                                                    )
                                                }
                                            }
                                            return Err(io::Error::new(
                                                ErrorKind::Other,
                                                format!(
                                                "Failed to deserialize status update message: {}",
                                                e
                                            ),
                                            ));
                                        }
                                    };

                                let status = match message {
                                    StatusUpdateMessage::Status { status } => match status {
                                        Ok(_) => FactoryStatus::Running,
                                        Err(_) => FactoryStatus::Dead,
                                    },
                                    StatusUpdateMessage::Stopping => FactoryStatus::Stopped,
                                    _ => FactoryStatus::Dead,
                                };

                                {
                                    let mut mut_factory = factory_ref; // TODO: Rename this, maybe remove reassignment, I'm just too done with this shit to care right now
                                    let new_status = status.clone();
                                    let factory_status = mut_factory.status.clone();
                                    if new_status != factory_status {
                                        if let Err(e) = factory_sender.send(
                                            FactoryChangeEvent::StatusChanged(uuid, new_status),
                                        ) {
                                            eprintln!("Failed to send unregistration event: {}", e);
                                            // Should be recoverable, since the socket got dropped anyway
                                        }
                                    }
                                }

                                match status {
                                    FactoryStatus::Running => {}
                                    FactoryStatus::Stopped => {
                                        eprintln!("Factory {} stopped", &name);
                                        return Ok(());
                                    }
                                    FactoryStatus::Dead => {
                                        return Err(io::Error::new(
                                            ErrorKind::Other,
                                            format!("Factory {} is not running", &name),
                                        ));
                                    }
                                    FactoryStatus::Uninitialized => {
                                        unreachable!("How the fuck is this even possible ?");
                                    }
                                }
                                sleep(Constants::STATUS_MESSAGE_REQUEST_INTERVAL).await;
                            }
                            Err(e) => {
                                return Err(io::Error::new(
                                    ErrorKind::ConnectionAborted,
                                    format!("Connection error during factory status check : {}", e),
                                ));
                            }
                        },
                    }
                }
                Err(e) => {
                    if e.kind() == io::ErrorKind::BrokenPipe {
                        eprintln!("Connection to factory {} closed", name);
                        {
                            if let Err(e) = factory_sender
                                .send(FactoryChangeEvent::StatusChanged(uuid, FactoryStatus::Dead))
                            {
                                eprintln!("Failed to send unregistration event: {}", e);
                                // Should be recoverable, since the socket got dropped anyway
                            }
                        }
                        return Ok(());
                    }
                    sleep(Constants::STATUS_MESSAGE_REQUEST_RETRY_INTERVAL).await;
                    continue;
                }
            }
        }
    }

    async fn registration_task(
        registration_socket: String,
        registration_factories_arc: FactoriesMap,
        factories_events_arc: Arc<broadcast::Sender<FactoryChangeEvent>>,
    ) -> Result<(), io::Error> {
        eprintln!("Starting registration task...");
        let mut listener = match get_raw_socket(registration_socket, SocketRole::Server).await {
            Err(e) => {
                panic!("Shutting down registering task due to error : {e}");
            }
            Ok(listen) => listen,
        };

        eprintln!("Registration thread listening...");

        loop {
            eprintln!("Ready to accept a new connection");
            let mut stream = listener.accept().await?;
            eprintln!("Ready to receive another message");
            match receive_from_socket(&stream).await {
                Ok(buf) => {
                    let buffer = buf;
                    eprintln!("Received registration message inside match",);
                    let message = match bincode::deserialize::<RegistrationMessage>(&buffer) {
                        Ok(message) => message,
                        Err(e) => {
                            panic!("Failed to deserialize incoming registration message: {}", e)
                        }
                    };
                    let result = Self::handle_message(
                        message,
                        Arc::clone(&registration_factories_arc),
                        Arc::clone(&factories_events_arc),
                    )
                    .await
                    .unwrap_or_else(|message| message);

                    eprintln!(
                        "Factories map length outside message handler : {}",
                        registration_factories_arc.len()
                    );
                    match bincode::serialize(&result) {
                        Ok(bytes) => {
                            eprintln!("Sending registration response");
                            send_in_socket(&stream, &bytes).await?;
                            eprintln!("Response sent");
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
        }
    }

    async fn handle_message(
        message: RegistrationMessage,
        arc_factories: FactoriesMap,
        factories_events_arc: Arc<broadcast::Sender<FactoryChangeEvent>>,
    ) -> Result<RegistrationMessage, RegistrationMessage> {
        match message {
            RegistrationMessage::Register { factory_name } => {
                eprintln!("Registering factory {}", &factory_name);
                let uuid = Uuid::new_v4();
                let socket_path = get_factory_ipc_socket_path(&uuid.to_string());
                eprintln!("Attempting to create socket at : {}", &socket_path);
                let _ = std::fs::remove_file(&socket_path);
                eprintln!("Done with removing file");
                let listener = match get_raw_socket(socket_path.clone(), SocketRole::Server).await {
                    Ok(listen) => listen,
                    Err(e) => {
                        return Err(RegistrationMessage::Error {
                            error: format!("Failed to open socket: {}", e),
                        })
                    }
                };

                eprintln!("Status socket opened for factory {}", &factory_name);

                let map = arc_factories.pin_owned();
                let socket = Arc::new(tokio::sync::Mutex::new(Some(listener)));
                map.insert(
                    uuid,
                    FactoryConnection {
                        name: factory_name.clone(),
                        socket,
                        status: FactoryStatus::Running,
                    },
                );

                eprintln!(
                    "Inserted factory at map address: {:p}",
                    Arc::as_ptr(&arc_factories)
                );
                eprintln!("Factory {} registered", &factory_name);
                eprintln!("Arc factories new length : {}", arc_factories.len());

                if let Err(e) = factories_events_arc.send(FactoryChangeEvent::Added(uuid)) {
                    map.update(uuid, |conn| {
                        conn.get_copy_with_different_status(FactoryStatus::Dead)
                    });
                    return Err(RegistrationMessage::Error {
                        error: format!("Failed to send registration event: {}", e),
                    });
                }

                Ok(RegistrationMessage::Registered {
                    factory_id: uuid,
                    socket_name: socket_path.to_owned(),
                })
            }
            RegistrationMessage::Unregister { factory_id } => {
                eprintln!("Unregistering factory {}", &factory_id);

                let map = arc_factories.pin_owned();
                map.update(factory_id, |conn| {
                    conn.get_copy_with_different_status(FactoryStatus::Stopped)
                });

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
