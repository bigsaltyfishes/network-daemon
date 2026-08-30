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
use lazy_static::lazy_static;
use libnetwork_daemon::{
    DaemonCommand, DaemonResponse, GlobalDaemonAction, GlobalDaemonResponse,
    InterfaceManagerAction, InterfaceResponse, WiFiManagerAction,
    WiFiManagerResponse, ensure, error::NetworkDaemonError, ignore,
};
use tracing::error;

use crate::{
    daemon::subscriber::Subscriber,
    interface::InterfaceManager,
    wifi::{WifiManager, WifiManagerBackend},
};

lazy_static! {
    static ref ESTABLISHED: String =
        ensure!(serde_json::to_string(&DaemonResponse::Global {
            response: GlobalDaemonResponse::Established
        }));
}

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
    _marker: std::marker::PhantomData<W>,
}

impl<W> Actor for ClientHandler<W>
where
    W: WifiManagerBackend,
{
    type Args = (UnixStream, ActorRef<InterfaceManager>);
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
