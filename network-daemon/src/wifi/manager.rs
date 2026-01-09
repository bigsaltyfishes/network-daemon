use std::{marker::PhantomData, ops::ControlFlow, path::PathBuf};

use async_channel::Receiver;
use kameo::{
    Actor,
    actor::{ActorId, ActorRef, Spawn, WeakActorRef},
    error::ActorStopReason,
    mailbox,
    prelude::{Context, Message},
};
use libnetwork_daemon::{
    WiFiManagerAction, WiFiManagerEvent, WiFiManagerResponse, error::WifiError,
    utils::Broadcast,
};
use tracing::{error, info, warn};

use super::WifiManagerBackend;

pub struct WifiManager<B: WifiManagerBackend> {
    iface: String,
    workdir: PathBuf,
    backend: Option<ActorRef<B>>,
    broadcast: Broadcast<WiFiManagerEvent>,
    retry_cnt: u8,
    _phantom: PhantomData<B>,
}

impl<B: WifiManagerBackend> WifiManager<B> {
    pub fn new(workdir: PathBuf, iface: String) -> Self {
        Self {
            iface,
            workdir,
            backend: None,
            broadcast: Broadcast::new(),
            retry_cnt: 0,
            _phantom: PhantomData,
        }
    }

    pub fn subscribe(&self) -> Receiver<WiFiManagerEvent> {
        self.broadcast.subscribe()
    }

    async fn start_backend(
        &mut self,
        actor_ref: &ActorRef<Self>,
    ) -> Result<(), WifiError> {
        info!("Starting WiFi backend for {}", self.iface);

        let backend =
            B::new(&self.workdir, &self.iface, actor_ref.clone()).await?;

        // Spawn backend (B implements Actor<Args=Self>)
        // kameo::actor::Spawn::spawn returns ActorRef<A> in 0.19 (?)
        // Checked via error message: "this expression has type ActorRef<B>"
        let backend_ref = B::spawn_with_mailbox(backend, mailbox::unbounded());

        // Link backend to supervisor
        actor_ref.link(&backend_ref).await;

        self.backend = Some(backend_ref);
        Ok(())
    }
}

impl<B: WifiManagerBackend> Actor for WifiManager<B> {
    type Args = Self;
    type Error = WifiError;

    async fn on_start(
        mut args: Self,
        actor_ref: ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        // Register supervisor
        let name = format!("WiFiManager-{}", args.iface);
        if let Err(e) = actor_ref.register(name) {
            // Pass name by value
            error!(
                "Failed to register WiFi Supervisor for {}: {}",
                args.iface, e
            );
        }

        while let Err(e) = args.start_backend(&actor_ref).await {
            error!(
                "Failed to start WiFi backend: {}, retrying ({}/3)",
                e,
                args.retry_cnt + 1
            );
            args.retry_cnt += 1;
            if args.retry_cnt >= 3 {
                error!("Exceeded maximum retry attempts to start WiFi backend");
                return Err(e);
            }
        }

        args.retry_cnt = 0;

        Ok(args)
    }

    async fn on_stop(
        &mut self,
        actor_ref: WeakActorRef<Self>,
        _reason: ActorStopReason,
    ) -> Result<(), Self::Error> {
        if let Some(backend) = &self.backend {
            // Unlink backend
            actor_ref
                .upgrade()
                .ok_or_else(|| {
                    WifiError::NotSupported(
                        "Supervisor is dead, cannot unlink backend".to_string(),
                    )
                })?
                .unlink(backend)
                .await;
            // Stop backend
            if let Err(_e) = backend.stop_gracefully().await {
                backend.kill();
            }
        }

        Ok(())
    }

    async fn on_link_died(
        &mut self,
        actor_ref: WeakActorRef<Self>,
        _dead_actor: ActorId,
        reason: ActorStopReason,
    ) -> Result<ControlFlow<ActorStopReason>, Self::Error> {
        warn!("WiFi backend died: {:?}. Restarting...", reason);
        self.backend = None;

        if let Some(actor_ref) = actor_ref.upgrade() {
            while let Err(e) = self.start_backend(&actor_ref).await {
                error!(
                    "Failed to restart WiFi backend: {}, retrying ({}/3)",
                    e,
                    self.retry_cnt + 1
                );
                self.retry_cnt += 1;
                if self.retry_cnt >= 3 {
                    error!(
                        "Exceeded maximum retry attempts to restart WiFi \
                         backend"
                    );
                    return Err(e);
                }
            }
        } else {
            error!("Supervisor is dead, cannot restart backend");
            return Ok(ControlFlow::Break(ActorStopReason::Normal));
        }

        self.retry_cnt = 0;

        Ok(ControlFlow::Continue(()))
    }
}

impl<B: WifiManagerBackend> Message<WiFiManagerEvent> for WifiManager<B> {
    type Reply = ();

    /// Handle WiFiManagerEvent messages by broadcasting them to subscribers
    async fn handle(
        &mut self,
        msg: WiFiManagerEvent,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.broadcast.broadcast(msg).await;
    }
}

impl<B: WifiManagerBackend> Message<WiFiManagerAction> for WifiManager<B> {
    type Reply = Result<WiFiManagerResponse, WifiError>;

    async fn handle(
        &mut self,
        msg: WiFiManagerAction,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        if let WiFiManagerAction::SubscribeEvents = msg {
            return Ok(WiFiManagerResponse::EventReceiver(
                self.broadcast.subscribe(),
            ));
        }

        if let Some(backend) = &self.backend {
            Ok(backend.ask(msg).await.map_err(|e| {
                e.err().unwrap_or(WifiError::Other(format!("Backend Died")))
            })?)
        } else {
            Err(WifiError::NotSupported("Backend not available".to_string()))
        }
    }
}
