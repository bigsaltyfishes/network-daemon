use std::{collections::HashMap, fs::File, io::Write, path::PathBuf};

use async_net::unix::{UnixListener, UnixStream};
use kameo::{
    Actor,
    actor::{ActorRef, Spawn},
    mailbox,
    message::StreamMessage,
    prelude::{Context, Message},
};
use libnetwork_daemon::{
    ConnectionState, InterfaceManagerAction, InterfaceManagerEvent,
    WiFiManagerEvent, WpaState, ensure, error::NetworkDaemonError, ignore,
};
use nix::fcntl::{Flock, FlockArg};
use tracing::{debug, error, info};

use crate::{
    daemon::{client::ClientHandler, server::StreamdListener},
    dhcp::DhcpManager,
    interface::InterfaceManager,
    route::RouteManager,
    wifi::{WifiManager, WifiManagerBackend},
};

pub struct NetworkDaemon<W>
where
    W: WifiManagerBackend,
{
    /// Working directory for network daemon
    working_dir: PathBuf,
    /// Interface Manager
    ifmgr: ActorRef<InterfaceManager>,
    /// Active WiFi Supervisors
    wifi_supervisors: HashMap<String, ActorRef<WifiManager<W>>>,
    /// Active Dhcp Supervisors
    dhcp_supervisors: HashMap<String, ActorRef<DhcpManager>>,
    /// Flock Handle
    _pid_lock: Flock<File>,
    /// Phantom Data
    _phantom: std::marker::PhantomData<W>,
}

impl<W> Actor for NetworkDaemon<W>
where
    W: WifiManagerBackend,
{
    type Args = PathBuf;
    type Error = NetworkDaemonError;

    async fn on_start(
        args: Self::Args,
        actor_ref: ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        ensure!(
            actor_ref.register("NetworkDaemon"),
            "Failed to register NetworkDaemon"
        );
        // Checking if there are running instance of network-daemon
        match tokio::fs::metadata(&args).await {
            Ok(meta) => {
                if !meta.is_dir() {
                    error!("Working directory is not a directory");
                    return Err(NetworkDaemonError::InvalidParameter(
                        "Working directory is not a directory".to_string(),
                    ));
                }
            }
            Err(e) => {
                if e.kind() == std::io::ErrorKind::NotFound {
                    tokio::fs::create_dir_all(&args).await.map_err(|e| {
                        error!("Failed to create working directory: {}", e);
                        NetworkDaemonError::InvalidParameter(
                            "Failed to create working directory".to_string(),
                        )
                    })?;
                } else {
                    error!("Failed to access working directory: {}", e);
                    return Err(NetworkDaemonError::InvalidParameter(
                        "Failed to access working directory".to_string(),
                    ));
                }
            }
        };

        // Check pid file and try to obtain lock
        let pid_file_path = args.join("network-daemon.pid");
        let pid_file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&pid_file_path)
            .map_err(|e| {
                error!("Failed to open pid file: {}", e);
                NetworkDaemonError::InvalidParameter(
                    "Failed to open pid file".to_string(),
                )
            })?;

        let pid_lock =
            match Flock::lock(pid_file, FlockArg::LockExclusiveNonblock) {
                Ok(mut l) => {
                    // Successfully obtained lock, write pid to file
                    let pid = std::process::id();
                    l.write(pid.to_string().as_bytes()).map_err(|e| {
                        error!("Failed to write pid to pid file: {}", e);
                        NetworkDaemonError::InvalidParameter(
                            "Failed to write pid to pid file".to_string(),
                        )
                    })?;
                    l
                }
                Err(_) => {
                    error!(
                        "Another instance of network-daemon is already running"
                    );
                    return Err(NetworkDaemonError::InvalidParameter(
                        "Another instance of network-daemon is already running"
                            .to_string(),
                    ));
                }
            };

        let ifmgr = InterfaceManager::new();
        let ifmgr_event_self = Box::pin(ifmgr.subscribe());
        let ifmgr_event_rt = ifmgr.subscribe();
        actor_ref.attach_stream(ifmgr_event_self, (), ());

        // Start Route Manager and Interface Manager
        // Route Manager needs to be started first to handle initial route setup
        let route_mgr = RouteManager::spawn_with_mailbox(
            ifmgr_event_rt,
            mailbox::unbounded(),
        );
        let ifmgr =
            InterfaceManager::spawn_with_mailbox(ifmgr, mailbox::unbounded());
        actor_ref.link(&ifmgr).await;
        actor_ref.link(&route_mgr).await;

        // Start Unix Domain Socket server for clients
        let socket_path = args.join("network-daemon.sock");
        let _ = tokio::fs::remove_file(&socket_path).await;

        let listener = Box::pin(StreamdListener::from(
            UnixListener::bind(&socket_path).map_err(|e| {
                error!("Failed to bind to socket: {}", e);
                NetworkDaemonError::InvalidParameter(
                    "Failed to bind to socket".to_string(),
                )
            })?,
        ));

        actor_ref.attach_stream(listener, (), ());

        info!("NetworkDaemon started, listening on {:?}", socket_path);

        Ok(Self {
            working_dir: args,
            ifmgr,
            wifi_supervisors: HashMap::new(),
            dhcp_supervisors: HashMap::new(),
            _pid_lock: pid_lock,
            _phantom: std::marker::PhantomData,
        })
    }
}

impl<W> Message<WiFiManagerEvent> for NetworkDaemon<W>
where
    W: WifiManagerBackend,
{
    type Reply = ();

    async fn handle(
        &mut self,
        msg: WiFiManagerEvent,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        debug!("Received WiFi Manager Event: {:?}", msg);
        if let WiFiManagerEvent::StatusUpdated { iface, status } = msg {
            let state = match status.state {
                Some(WpaState::InterfaceDisabled) => {
                    Some(ConnectionState::Disabled)
                }
                Some(WpaState::Unknown) => Some(ConnectionState::NotApplicable),
                Some(WpaState::Completed) => Some(ConnectionState::Connected),
                Some(WpaState::Disconnected)
                | Some(WpaState::Inactive)
                | Some(WpaState::Scanning) => Some(ConnectionState::NoCarrier),
                Some(_) => Some(ConnectionState::Up),
                _ => None,
            };

            if let Some(state) = state {
                ignore!(
                    self.ifmgr
                        .tell(InterfaceManagerAction::UpdateExternalInterfaceState {
                            name: iface,
                            state
                        })
                        .await
                );
            }
        }
    }
}

impl<W> Message<StreamMessage<WiFiManagerEvent, (), ()>> for NetworkDaemon<W>
where
    W: WifiManagerBackend,
{
    type Reply = ();

    async fn handle(
        &mut self,
        msg: StreamMessage<WiFiManagerEvent, (), ()>,
        ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        if let StreamMessage::Next(event) = msg {
            ignore!(self.handle(event, ctx).await);
        }
    }
}

impl<W> Message<StreamMessage<Result<UnixStream, std::io::Error>, (), ()>>
    for NetworkDaemon<W>
where
    W: WifiManagerBackend,
{
    type Reply = ();

    async fn handle(
        &mut self,
        msg: StreamMessage<Result<UnixStream, std::io::Error>, (), ()>,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        match msg {
            StreamMessage::Next(Ok(stream)) => {
                info!("New client connected");
                ClientHandler::<W>::spawn_with_mailbox(
                    (stream, self.ifmgr.clone()),
                    mailbox::unbounded(),
                );
            }
            StreamMessage::Next(Err(e)) => {
                error!("Error accepting client connection: {}", e);
            }
            _ => {}
        }
    }
}

impl<W> Message<StreamMessage<InterfaceManagerEvent, (), ()>>
    for NetworkDaemon<W>
where
    W: WifiManagerBackend,
{
    type Reply = ();

    async fn handle(
        &mut self,
        msg: StreamMessage<InterfaceManagerEvent, (), ()>,
        ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        if let StreamMessage::Next(event) = msg {
            debug!("Received Interface Manager Event: {:?}", event);
            // Handle interface manager events here
            match event {
                InterfaceManagerEvent::InterfaceAdded(info) => {
                    info!("Interface added: {:?}", info);
                    if info.is_wlan() {
                        let iface_name = info.name.clone();
                        // Create WifiManager Supervisor for the interface
                        let wifi_mgr = WifiManager::<W>::new(
                            self.working_dir.clone(),
                            iface_name.clone(),
                        );

                        // Subscribe to events
                        let event_receiver = wifi_mgr.subscribe();
                        ctx.actor_ref().attach_stream(
                            Box::pin(event_receiver),
                            (),
                            (),
                        );

                        // Start WifiManager supervisor (Args=Self)
                        let supervisor_ref =
                            WifiManager::<W>::spawn_with_mailbox(
                                wifi_mgr,
                                mailbox::unbounded(),
                            );
                        self.wifi_supervisors
                            .insert(iface_name.clone(), supervisor_ref);

                        info!(
                            "Started WiFi Manager for interface: {}",
                            info.name
                        );
                    }

                    if info.dhcpv4_enabled
                        && info.state == ConnectionState::Up
                        && !self.dhcp_supervisors.contains_key(&info.name)
                        && let Some(mac) = info.mac_addr
                    {
                        let iface_name = info.name.clone();
                        // Create DhcpManager Supervisor for the interface
                        let dhcp_mgr = DhcpManager::new(
                            iface_name.clone(),
                            mac,
                            self.ifmgr.clone(),
                        );

                        // Start DhcpManager supervisor (Args=Self)
                        let supervisor_ref = DhcpManager::spawn_with_mailbox(
                            dhcp_mgr,
                            mailbox::unbounded(),
                        );
                        self.dhcp_supervisors
                            .insert(iface_name.clone(), supervisor_ref);

                        info!(
                            "Started DHCP Manager for interface: {}",
                            info.name
                        );
                    }
                }
                InterfaceManagerEvent::InterfaceRemoved(info) => {
                    info!("Interface removed: {:?}", info);
                    if info.is_wlan() {
                        let iface_name = info.name.clone();
                        if let Some(w) =
                            self.wifi_supervisors.remove(&iface_name)
                            && let Err(e) = w.stop_gracefully().await
                        {
                            error!(
                                "Failed to stop WiFi Manager for \
                                 interface {}: {}, Killing actor",
                                iface_name, e
                            );
                            w.kill();
                        }
                    }
                }
                InterfaceManagerEvent::InterfaceChanged(info) => {
                    info!("Interface changed: {:?}", info);
                    if !info.dhcpv4_enabled {
                        if let Some(d) =
                            self.dhcp_supervisors.remove(&info.name)
                        {
                            if let Err(e) = d.stop_gracefully().await {
                                error!(
                                    "Failed to stop DHCP Manager for \
                                     interface {}: {}, Killing actor",
                                    info.name, e
                                );
                                d.kill();
                            } else {
                                info!(
                                    "Stopped DHCP Manager for interface: \
                                     {}",
                                    info.name
                                );
                            }
                        }
                    } else if !matches!(
                        info.state,
                        ConnectionState::Up | ConnectionState::Connected
                    ) {
                        if let Some(d) =
                            self.dhcp_supervisors.remove(&info.name)
                        {
                            if let Err(e) = d.stop_gracefully().await {
                                error!(
                                    "Failed to stop DHCP Manager for \
                                     interface {}: {}, Killing actor",
                                    info.name, e
                                );
                                d.kill();
                            } else {
                                info!(
                                    "Stopped DHCP Manager for interface: \
                                     {}",
                                    info.name
                                );
                            }
                        }
                    } else if info.state == ConnectionState::Up
                        && !self.dhcp_supervisors.contains_key(&info.name)
                        && let Some(mac) = info.mac_addr
                    {
                        let iface_name = info.name.clone();
                        // Create DhcpManager Supervisor for the interface
                        let dhcp_mgr = DhcpManager::new(
                            iface_name.clone(),
                            mac,
                            self.ifmgr.clone(),
                        );

                        // Start DhcpManager supervisor (Args=Self)
                        let supervisor_ref = DhcpManager::spawn_with_mailbox(
                            dhcp_mgr,
                            mailbox::unbounded(),
                        );
                        self.dhcp_supervisors
                            .insert(iface_name.clone(), supervisor_ref);

                        info!(
                            "Started DHCP Manager for interface: {}",
                            info.name
                        );
                    }
                }
            }
        }
    }
}
