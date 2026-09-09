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
    KnownNetwork, KnownNetworkState, MacAddr, Security, WiFiManagerAction,
    WiFiManagerEvent, WpaState, ensure, error::NetworkDaemonError, ignore,
};
use nix::fcntl::{Flock, FlockArg};
use tracing::{debug, error, info};

use crate::{
    daemon::{client::ClientHandler, server::StreamdListener},
    dhcp::DhcpManager,
    interface::InterfaceManager,
    route::RouteManager,
    storage::{Credential, StorageManager, StoragePaths, StorageResult},
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
    /// Credential store (SQLite, encrypted at rest)
    storage: ActorRef<StorageManager>,
    /// Path to the user-editable config.toml (non-sensitive settings).
    config_path: PathBuf,
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

        // Restrict the control socket to the `network` group (owned by
        // network:network, mode 0660). The daemon runs as root so it can
        // chown. If the group is absent we still set 0660 so only root /
        // group members (or the owner) can connect.
        {
            use std::os::unix::fs::PermissionsExt;
            let set_group = match nix::unistd::Group::from_name("network") {
                Ok(Some(g)) => {
                    nix::unistd::chown(&socket_path, None, Some(g.gid)).is_ok()
                }
                _ => false,
            };
            if let Ok(meta) = std::fs::metadata(&socket_path) {
                let mut perms = meta.permissions();
                perms.set_mode(0o660);
                std::fs::set_permissions(&socket_path, perms).ok();
            }
            if !set_group {
                info!(
                    "Socket restricted to mode 0660; 'network' group not applied"
                );
            }
        }

        actor_ref.attach_stream(listener, (), ());

        info!("NetworkDaemon started, listening on {:?}", socket_path);

        // Start the credential storage manager (SQLite + encrypted secrets).
        // Config dir keeps credentials apart from the runtime socket dir.
        let config_dir = PathBuf::from("/var/db/network-daemon");
        let storage_paths = StoragePaths {
            runtime_dir: args.clone(),
            config_dir,
        };
        let storage =
            StorageManager::new(&storage_paths).await.map_err(|e| {
                error!("Failed to open credential store: {}", e);
                NetworkDaemonError::InvalidParameter(
                    "Failed to open credential store".to_string(),
                )
            })?;
        let storage =
            StorageManager::spawn_with_mailbox(storage, mailbox::unbounded());
        actor_ref.link(&storage).await;

        info!("NetworkDaemon started, listening on {:?}", socket_path);

        Ok(Self {
            working_dir: args,
            ifmgr,
            wifi_supervisors: HashMap::new(),
            dhcp_supervisors: HashMap::new(),
            storage,
            config_path: PathBuf::from("/var/db/network-daemon/config.toml"),
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
                    (
                        stream,
                        self.ifmgr.clone(),
                        self.storage.clone(),
                        self.config_path.clone(),
                    ),
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

                        let networks =
                            self.load_persisted_networks(info.mac_addr).await;
                        ignore!(
                            self.wifi_supervisors
                                .get(&iface_name)
                                .expect("Wi-Fi supervisor was just inserted")
                                .ask(WiFiManagerAction::LoadNetworks {
                                    networks
                                })
                                .await
                        );

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
                    } else if matches!(
                        info.state,
                        ConnectionState::Up | ConnectionState::Connected
                    ) && !self
                        .dhcp_supervisors
                        .contains_key(&info.name)
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

impl<W> NetworkDaemon<W>
where
    W: WifiManagerBackend,
{
    /// Load the saved AP profiles belonging to one WLAN MAC. Entries written
    /// by older daemon versions have no MAC and are used only when that
    /// interface has no newer, MAC-scoped entries in the corresponding store.
    async fn load_persisted_networks(
        &self,
        interface_mac: Option<MacAddr>,
    ) -> Vec<KnownNetwork> {
        let credentials = match self.storage.ask(()).await {
            Ok(StorageResult::Credentials(credentials)) => credentials,
            Ok(StorageResult::Ok) | Err(_) => Vec::new(),
        };
        let config = crate::config::DaemonConfig::load(&self.config_path)
            .unwrap_or_default();
        persisted_networks(&config, &credentials, interface_mac)
    }
}

fn persisted_networks(
    config: &crate::config::DaemonConfig,
    credentials: &[Credential],
    interface_mac: Option<MacAddr>,
) -> Vec<KnownNetwork> {
    let mac = interface_mac.map(|value| value.to_string());
    let has_scoped_config = mac.as_deref().is_some_and(|mac| {
        config
            .networks
            .iter()
            .any(|network| network.interface_mac.as_deref() == Some(mac))
    });
    let has_scoped_credentials = mac.as_deref().is_some_and(|mac| {
        credentials
            .iter()
            .any(|credential| credential.interface_mac.as_deref() == Some(mac))
    });

    let config_in_scope = |network: &crate::config::NetworkConfig| {
        network.interface_mac.as_deref() == mac.as_deref()
            || (mac.is_some()
                && !has_scoped_config
                && network.interface_mac.is_none())
    };
    let credential_in_scope = |credential: &Credential| {
        credential.interface_mac.as_deref() == mac.as_deref()
            || (mac.is_some()
                && !has_scoped_credentials
                && credential.interface_mac.is_none())
    };

    let mut networks: HashMap<
        (String, Option<MacAddr>, Security),
        KnownNetwork,
    > = HashMap::new();
    for metadata in config
        .networks
        .iter()
        .filter(|network| config_in_scope(network))
    {
        let bssid = metadata.bssid.as_deref().and_then(MacAddr::parse);
        let security = Security::from_name(&metadata.security);
        networks.insert(
            (metadata.ssid.clone(), bssid, security),
            KnownNetwork {
                id: None,
                priority: metadata.priority,
                hidden: metadata.hidden,
                security,
                state: if metadata.enabled {
                    KnownNetworkState::Enabled
                } else {
                    KnownNetworkState::Disabled
                },
                ssid: metadata.ssid.clone(),
                bssid,
                password: None,
                identity: None,
                autoconnect: metadata.enabled,
            },
        );
    }

    for credential in credentials
        .iter()
        .filter(|credential| credential_in_scope(credential))
    {
        let bssid = credential.bssid.as_deref().and_then(MacAddr::parse);
        let security = Security::from_name(&credential.security);
        let key = (credential.ssid.clone(), bssid, security);
        let network = networks.entry(key).or_insert_with(|| KnownNetwork {
            id: None,
            priority: 0,
            hidden: false,
            security,
            state: KnownNetworkState::Enabled,
            ssid: credential.ssid.clone(),
            bssid,
            password: None,
            identity: None,
            autoconnect: true,
        });
        network.password = credential.psk.clone();
        network.identity = credential.identity.clone();
    }

    let mut networks = networks.into_values().collect::<Vec<_>>();
    networks.sort_by_key(|network| {
        (
            network.ssid.clone(),
            network.security.to_string(),
            network
                .bssid
                .map(|bssid| bssid.to_string())
                .unwrap_or_default(),
        )
    });
    networks
}
