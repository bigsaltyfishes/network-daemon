use async_net::unix::UnixStream;
use futures_lite::{
    AsyncBufReadExt, AsyncWriteExt,
    io::{BufReader, BufWriter},
};
use kameo::{
    Actor,
    actor::{ActorRef, Spawn},
    mailbox,
    message::StreamMessage,
    prelude::{Context, Message},
};
use libnetwork_daemon::{
    DaemonCommand, DaemonResponse, GlobalDaemonAction, GlobalDaemonResponse,
    InterfaceManagerAction, InterfaceResponse, Security, WiFiManagerAction,
    WiFiManagerResponse, ensure, error::NetworkDaemonError, ignore,
};
use tracing::error;

use crate::{
    config::{DaemonConfig, NetworkConfig},
    daemon::subscriber::Subscriber,
    interface::InterfaceManager,
    storage::{Credential, StorageCommand, StorageManager},
    wifi::{WifiManager, WifiManagerBackend},
};

static ESTABLISHED: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| {
        ensure!(serde_json::to_string(&DaemonResponse::Global {
            response: GlobalDaemonResponse::Established
        }))
    });

#[derive(Actor)]
pub struct StreamWriter {
    writer: BufWriter<UnixStream>,
}

impl From<UnixStream> for StreamWriter {
    fn from(stream: UnixStream) -> Self {
        Self {
            writer: BufWriter::new(stream),
        }
    }
}

impl Message<String> for StreamWriter {
    type Reply = ();

    async fn handle(
        &mut self,
        mut msg: String,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        if !msg.ends_with('\n') {
            msg.push('\n');
        }

        if let Err(e) = self.writer.write_all(msg.as_bytes()).await {
            tracing::error!("Failed to write to stream: {}", e);
        }

        if let Err(e) = self.writer.flush().await {
            tracing::error!("Failed to flush stream: {}", e);
        }
    }
}

pub struct ClientHandler<W>
where
    W: WifiManagerBackend,
{
    _stream: UnixStream,
    writer: ActorRef<StreamWriter>,
    ifmgr: ActorRef<InterfaceManager>,
    storage: ActorRef<StorageManager>,
    config_path: std::path::PathBuf,
    _marker: std::marker::PhantomData<W>,
}

impl<W> Actor for ClientHandler<W>
where
    W: WifiManagerBackend,
{
    type Args = (
        UnixStream,
        ActorRef<InterfaceManager>,
        ActorRef<StorageManager>,
        std::path::PathBuf,
    );
    type Error = ();

    async fn on_start(
        args: Self::Args,
        actor_ref: ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        let reader = BufReader::new(args.0.clone());
        let lines = reader.lines();

        let writer_actor = StreamWriter::spawn_with_mailbox(
            StreamWriter::from(args.0.clone()),
            mailbox::unbounded(),
        );
        actor_ref.link(&writer_actor).await;
        actor_ref.attach_stream(lines, (), ());

        Ok(ClientHandler {
            _stream: args.0,
            writer: writer_actor,
            ifmgr: args.1,
            storage: args.2,
            config_path: args.3,
            _marker: std::marker::PhantomData,
        })
    }
}

impl<W> Message<StreamMessage<Result<String, std::io::Error>, (), ()>>
    for ClientHandler<W>
where
    W: WifiManagerBackend,
{
    type Reply = ();

    async fn handle(
        &mut self,
        msg: StreamMessage<Result<String, std::io::Error>, (), ()>,
        ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        let actor_ref = ctx.actor_ref();
        match msg {
            StreamMessage::Next(Ok(line)) => {
                let cmd: DaemonCommand =
                    if let Ok(cmd) = serde_json::from_str(&line) {
                        cmd
                    } else {
                        error!("Failed to parse command: {}", line);
                        let ret = DaemonResponse::Error(
                            NetworkDaemonError::InvalidParameter(
                                "Failed to parse command".to_string(),
                            ),
                        );
                        let resp = ensure!(serde_json::to_string(&ret));
                        ignore!(self.writer.tell(resp).await);
                        return;
                    };

                // Handle command here
                match cmd {
                    DaemonCommand::Global { action } => match action {
                        GlobalDaemonAction::Shutdown => {
                            let ret = DaemonResponse::Global {
                                response: GlobalDaemonResponse::ShutdownAck,
                            };
                            let resp = serde_json::to_string(&ret).unwrap();
                            if let Err(e) = self.writer.tell(resp).await {
                                error!("Failed to send shutdown ack: {}", e);
                            }
                            ctx.stop();
                        }
                    },
                    DaemonCommand::WiFiManager { iface, action } => {
                        // Check if wifi manager exists for interface
                        let wifi_mgr_ident = format!("WiFiManager-{}", iface);
                        let wifi_mgr = if let Ok(Some(mgr)) =
                            ActorRef::<WifiManager<W>>::lookup(
                                wifi_mgr_ident.as_str(),
                            ) {
                            mgr
                        } else {
                            let ret = DaemonResponse::Global {
                                    response: GlobalDaemonResponse::WiFiInterfaceNotFound {
                                        iface: iface.clone(),
                                    },
                                };
                            let resp = serde_json::to_string(&ret).unwrap();
                            if let Err(e) = self.writer.tell(resp).await {
                                error!("Failed to send response: {}", e);
                            }
                            return;
                        };

                        match action {
                            WiFiManagerAction::SubscribeEvents => match wifi_mgr
                                .ask(WiFiManagerAction::SubscribeEvents)
                                .await
                            {
                                Ok(ret) => {
                                    if let WiFiManagerResponse::EventReceiver(
                                        recv,
                                    ) = ret
                                    {
                                        let writer = self.writer.clone();
                                        let subscriber = Subscriber::new(
                                            writer,
                                            move |event| {
                                                DaemonResponse::WiFiManager {
                                                        iface: iface.clone(),
                                                        response: WiFiManagerResponse::Event(event),
                                                    }
                                            },
                                        );
                                        let actor_ref =
                                            Subscriber::spawn_with_mailbox(
                                                subscriber,
                                                mailbox::unbounded(),
                                            );
                                        actor_ref.link(&self.writer).await;
                                        actor_ref.attach_stream(
                                            Box::pin(recv),
                                            (),
                                            (),
                                        );
                                        ctx.actor_ref().link(&actor_ref).await;
                                    } else {
                                        error!(
                                            "Unexpected response from \
                                             WiFiManager"
                                        );
                                    }
                                }
                                Err(e) => {
                                    error!(
                                        "Failed to send subscribe action to \
                                         WiFi manager: {:?}",
                                        e
                                    );
                                    // Check if there's an inner application
                                    // error
                                    let error_msg = format!(
                                        "WiFi manager actor error: {}",
                                        e
                                    );
                                    let ret = if let Some(wifi_err) = e.err() {
                                        DaemonResponse::Error(
                                            NetworkDaemonError::from(wifi_err),
                                        )
                                    } else {
                                        DaemonResponse::Global {
                                            response:
                                                GlobalDaemonResponse::Error {
                                                    message: error_msg,
                                                },
                                        }
                                    };
                                    let resp =
                                        serde_json::to_string(&ret).unwrap();
                                    if let Err(e) = self.writer.tell(resp).await
                                    {
                                        error!(
                                            "Failed to send response: {}",
                                            e
                                        );
                                    }
                                }
                            },
                            action => {
                                // Persist known-network changes before forwarding.
                                match &action {
                                    WiFiManagerAction::AddNetwork {
                                        ssid,
                                        bssid,
                                        security,
                                        password,
                                        identity,
                                        hidden,
                                    } => {
                                        self.persist_network(
                                            ssid, bssid, security, password,
                                            identity, hidden,
                                        )
                                        .await;
                                    }
                                    WiFiManagerAction::RemoveNetwork {
                                        ssid,
                                        bssid,
                                    } => {
                                        self.remove_persisted(ssid, bssid)
                                            .await;
                                    }
                                    _ => {}
                                }
                                match wifi_mgr.ask(action).await {
                                    Ok(ret) => {
                                        // Wait for response
                                        let ret = DaemonResponse::WiFiManager {
                                            iface: iface.clone(),
                                            response: ret,
                                        };
                                        let resp = serde_json::to_string(&ret)
                                            .unwrap();
                                        let _ = self.writer.tell(resp).await;
                                    }
                                    Err(e) => {
                                        error!(
                                            "Failed to send action to WiFi \
                                             manager: {:?}",
                                            e
                                        );
                                        // Check if there's an inner application
                                        // error
                                        let error_msg = format!(
                                            "WiFi manager actor error: {}",
                                            e
                                        );
                                        let ret = if let Some(wifi_err) =
                                            e.err()
                                        {
                                            DaemonResponse::Error(
                                                NetworkDaemonError::from(
                                                    wifi_err,
                                                ),
                                            )
                                        } else {
                                            DaemonResponse::Global {
                                                response: GlobalDaemonResponse::Error {
                                                    message: error_msg,
                                                },
                                            }
                                        };
                                        let resp = serde_json::to_string(&ret)
                                            .unwrap();
                                        let _ = self.writer.tell(resp).await;
                                    }
                                }
                            }
                        }
                    }
                    DaemonCommand::InterfaceManager { action } => {
                        match action {
                            InterfaceManagerAction::SubscribeEvents => {
                                match self
                                    .ifmgr
                                    .ask(
                                        InterfaceManagerAction::SubscribeEvents,
                                    )
                                    .await
                                {
                                    Ok(ret) => {
                                        // Successfully subscribed
                                        // Handle events from rx
                                        if let InterfaceResponse::EventReceiver(rx) = ret {
                                            let writer = self.writer.clone();
                                            let subscriber = Subscriber::new(writer, |event| {
                                                DaemonResponse::InterfaceManager {
                                                    response: InterfaceResponse::Event(event),
                                                }
                                            });
                                            let actor_ref = Subscriber::spawn_with_mailbox(
                                                subscriber,
                                                mailbox::unbounded(),
                                            );
                                            actor_ref.link(&self.writer).await;
                                            actor_ref.attach_stream(Box::pin(rx), (), ());
                                            ctx.actor_ref().link(&actor_ref).await;
                                        }
                                    }
                                    Err(e) => {
                                        error!(
                                            "Failed to send subscribe action \
                                             to Interface manager: {:?}",
                                            e
                                        );
                                        // Check if there's an inner application
                                        // error
                                        let error_msg = format!(
                                            "Interface manager actor error: {}",
                                            e
                                        );
                                        let ret = if let Some(iface_err) =
                                            e.err()
                                        {
                                            DaemonResponse::Error(
                                                NetworkDaemonError::from(
                                                    iface_err,
                                                ),
                                            )
                                        } else {
                                            DaemonResponse::Global {
                                                response: GlobalDaemonResponse::Error {
                                                    message: error_msg,
                                                },
                                            }
                                        };
                                        let resp = serde_json::to_string(&ret)
                                            .unwrap();
                                        let _ = self.writer.tell(resp).await;
                                    }
                                }
                            }
                            action => {
                                match self.ifmgr.ask(action).await {
                                    Ok(ret) => {
                                        // Wait for response
                                        let ret =
                                            DaemonResponse::InterfaceManager {
                                                response: ret,
                                            };
                                        let resp = serde_json::to_string(&ret)
                                            .unwrap();
                                        let _ = self.writer.tell(resp).await;
                                    }
                                    Err(e) => {
                                        error!(
                                            "Failed to send action to \
                                             Interface manager: {:?}",
                                            e
                                        );
                                        // Check if there's an inner application
                                        // error
                                        let error_msg = format!(
                                            "Interface manager actor error: {}",
                                            e
                                        );
                                        let ret = if let Some(iface_err) =
                                            e.err()
                                        {
                                            DaemonResponse::Error(
                                                NetworkDaemonError::from(
                                                    iface_err,
                                                ),
                                            )
                                        } else {
                                            DaemonResponse::Global {
                                                response: GlobalDaemonResponse::Error {
                                                    message: error_msg,
                                                },
                                            }
                                        };
                                        let resp = serde_json::to_string(&ret)
                                            .unwrap();
                                        let _ = self.writer.tell(resp).await;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            StreamMessage::Next(Err(e)) => {
                tracing::error!("Error reading from client stream: {}", e);
                let _ = actor_ref.stop_gracefully().await;
            }
            StreamMessage::Finished(_) => {
                tracing::info!("Client disconnected");
                let _ = actor_ref.stop_gracefully().await;
            }
            StreamMessage::Started(_) => {
                tracing::info!("Established connection with new client");
                self.writer.tell(ESTABLISHED.clone()).await.unwrap();
            }
        }
    }
}

impl<W> ClientHandler<W>
where
    W: WifiManagerBackend,
{
    /// Persist a known network: metadata to config.toml, credential to SQLite.
    async fn persist_network(
        &self,
        ssid: &str,
        bssid: &Option<libnetwork_daemon::MacAddr>,
        security: &Security,
        password: &Option<String>,
        identity: &Option<String>,
        hidden: &bool,
    ) {
        // Sensitive credential → SQLite (encrypted at rest).
        let cred = Credential {
            ssid: ssid.to_string(),
            bssid: bssid.map(|m| m.to_string()),
            security: security.to_string(),
            psk: password.clone(),
            identity: identity.clone(),
        };
        if let Err(e) =
            self.storage.ask(StorageCommand::SaveCredential(cred)).await
        {
            error!("failed to persist credential: {:?}", e);
        }

        // Metadata → config.toml (non-sensitive).
        let mut cfg = DaemonConfig::load(&self.config_path).unwrap_or_default();
        cfg.upsert_network(NetworkConfig {
            ssid: ssid.to_string(),
            bssid: bssid.map(|m| m.to_string()),
            security: security.to_string(),
            hidden: *hidden,
            priority: 0,
            enabled: true,
        });
        if let Err(e) = cfg.save(&self.config_path) {
            error!("failed to save config: {}", e);
        }
    }

    /// Remove a persisted known network (SQLite + config.toml).
    async fn remove_persisted(
        &self,
        ssid: &str,
        bssid: &Option<libnetwork_daemon::MacAddr>,
    ) {
        let b = bssid.map(|m| m.to_string());
        if let Err(e) = self
            .storage
            .ask(StorageCommand::RemoveCredential {
                ssid: ssid.to_string(),
                bssid: b.clone(),
            })
            .await
        {
            error!("failed to remove credential: {:?}", e);
        }

        let mut cfg = DaemonConfig::load(&self.config_path).unwrap_or_default();
        cfg.remove_network(ssid, b.as_deref());
        if let Err(e) = cfg.save(&self.config_path) {
            error!("failed to save config: {}", e);
        }
    }
}
