mod ifconfig;
mod query;
mod table;

use std::{
    collections::{HashMap, HashSet},
    ops::ControlFlow,
    path::Path,
};

use async_channel::Receiver;
use kameo::{
    Actor,
    actor::{ActorId, ActorRef, WeakActorRef},
    error::ActorStopReason,
    message::StreamMessage,
    prelude::{Context, Message},
};
use libnetwork_daemon::{
    ConnectionState, InterfaceInfo, InterfaceManagerAction,
    InterfaceManagerEvent, InterfaceResponse, InterfaceType, IntoPrefixed,
    LinkOptions, Modification, PrefixedIpAddr, WlanLinkOptions, ensure,
    error::InterfaceError, utils::Broadcast,
};
use netlink_packet_core::{DecodeError, NetlinkMessage, NetlinkPayload};
use netlink_packet_route::{RouteNetlinkMessage, link::LinkFlags};
use table::InterfaceTable;
use tracing::{error, info, warn};

use self::ifconfig::Ifconfig;
use crate::{
    config::{ConfigError, DaemonConfig},
    ffi,
    netlink::{NetlinkListener, NetlinkModifier, NetlinkQuery},
};

const CONFIG_PATH: &str = "/var/db/network-daemon/config.toml";

/// Interface manager using FreeBSD kernel interfaces.
///
/// Manages network interfaces and their states
pub struct InterfaceManager {
    ifconfig: Ifconfig,
    table: InterfaceTable,
    event_broadcaster: Broadcast<InterfaceManagerEvent>,
}

impl InterfaceManager {
    /// Create a new InterfaceManager
    pub fn new() -> Self {
        let ifconfig = Ifconfig::new();

        Self {
            ifconfig,
            table: InterfaceTable::new(),
            event_broadcaster: Broadcast::new(),
        }
    }

    /// Subscribe to interface events
    pub fn subscribe(&self) -> Receiver<InterfaceManagerEvent> {
        self.event_broadcaster.subscribe()
    }

    /// Refresh the list of interfaces and their states
    pub async fn refresh_interfaces(
        &mut self,
        _actor_ref: &ActorRef<Self>,
    ) -> Result<(), InterfaceError> {
        let mut query =
            NetlinkQuery::new().map_err(InterfaceError::NetlinkQueryError)?;
        let old_interfaces_set =
            self.table.all().into_iter().collect::<HashSet<_>>();
        let mut interfaces = query
            .query_links()
            .await
            .map_err(InterfaceError::NetlinkQueryError)?;

        if self.create_configured_links(&interfaces).await {
            interfaces = query
                .query_links()
                .await
                .map_err(InterfaceError::NetlinkQueryError)?;
        }

        if self.create_auto_wlans(&interfaces).await {
            interfaces = query
                .query_links()
                .await
                .map_err(InterfaceError::NetlinkQueryError)?;
        }

        self.apply_saved_policies(&mut interfaces).await;

        let new_interfaces_set =
            interfaces.into_values().collect::<HashSet<_>>();

        // Determine added and removed interfaces
        let added_interfaces: Vec<InterfaceInfo> = new_interfaces_set
            .difference(&old_interfaces_set)
            .cloned()
            .collect();
        let removed_interfaces: Vec<InterfaceInfo> = old_interfaces_set
            .difference(&new_interfaces_set)
            .cloned()
            .collect();

        // Update internal state
        self.table.clear();

        for iface in new_interfaces_set {
            self.table.insert(iface);
        }

        // Send to servers
        for iface in removed_interfaces {
            self.event_broadcaster
                .broadcast(InterfaceManagerEvent::InterfaceRemoved(
                    iface.clone(),
                ))
                .await;
        }

        for iface in added_interfaces {
            self.event_broadcaster
                .broadcast(InterfaceManagerEvent::InterfaceAdded(iface.clone()))
                .await;
        }

        Ok(())
    }

    /// Apply persisted DHCPv4/SLAAC policy to the live interface snapshot.
    async fn apply_saved_policies(
        &self,
        interfaces: &mut HashMap<u32, InterfaceInfo>,
    ) {
        let config = match DaemonConfig::load(Path::new(CONFIG_PATH)) {
            Ok(config) => config,
            Err(ConfigError::NotFound(_)) => return,
            Err(error) => {
                warn!(
                    "Skipping persisted interface policy: config.toml could not be loaded: {error}"
                );
                return;
            }
        };
        for interface in interfaces.values_mut() {
            let Some(policy) = config.interface.get(&interface.name) else {
                continue;
            };
            interface.dhcpv4_enabled = policy.dhcpv4;
            if interface.slaac_enabled == policy.slaac {
                continue;
            }
            match self
                .ifconfig
                .set_slaac_state(&interface.name, policy.slaac)
                .await
            {
                Ok(()) => interface.slaac_enabled = policy.slaac,
                Err(error) => warn!(
                    "Could not apply SLAAC policy to {}: {}",
                    interface.name, error
                ),
            }
        }
    }

    /// Recreate explicitly persisted logical links that are not present.
    async fn create_configured_links(
        &self,
        interfaces: &HashMap<u32, InterfaceInfo>,
    ) -> bool {
        let config = match DaemonConfig::load(Path::new(CONFIG_PATH)) {
            Ok(config) => config,
            Err(ConfigError::NotFound(_)) => return false,
            Err(error) => {
                warn!(
                    "Skipping persisted interface creation: config.toml could not be loaded: {error}"
                );
                return false;
            }
        };
        let mut names = interfaces
            .values()
            .map(|interface| interface.name.clone())
            .collect::<HashSet<_>>();
        let mut created = false;
        let mut pending = config
            .interface
            .into_iter()
            .filter_map(|(name, interface_config)| {
                interface_config.creation.map(|options| (name, options))
            })
            .collect::<Vec<_>>();

        // Retry deferred links so a persisted VLAN/bridge/lagg can depend on
        // another persisted logical link that sorts later in the TOML map.
        loop {
            let mut deferred = Vec::new();
            let mut progress = false;
            for (name, options) in pending {
                if names.contains(&name) {
                    continue;
                }
                match self.ifconfig.create_link(&name, &options).await {
                    Ok(()) => {
                        info!("Restored persisted interface {}", name);
                        names.insert(name);
                        created = true;
                        progress = true;
                    }
                    Err(error) => deferred.push((name, options, error)),
                }
            }

            if deferred.is_empty() || !progress {
                for (name, _options, error) in deferred {
                    warn!(
                        "Could not restore persisted interface {}: {}",
                        name, error
                    );
                }
                break;
            }
            pending = deferred
                .into_iter()
                .map(|(name, options, _error)| (name, options))
                .collect();
        }
        created
    }

    /// Create default WLAN interfaces for unconfigured wireless devices.
    async fn create_auto_wlans(
        &self,
        interfaces: &HashMap<u32, InterfaceInfo>,
    ) -> bool {
        let config = match DaemonConfig::load(Path::new(CONFIG_PATH)) {
            Ok(config) => config,
            Err(ConfigError::NotFound(_)) => DaemonConfig::default(),
            Err(error) => {
                warn!(
                    "Skipping automatic WLAN creation: config.toml could not be loaded: {error}"
                );
                return false;
            }
        };
        let devices = match self.ifconfig.wireless_devices().await {
            Ok(devices) => devices,
            Err(error) => {
                warn!("Could not enumerate wireless devices: {}", error);
                return false;
            }
        };
        let existing_parents = interfaces
            .values()
            .filter(|interface| interface.is_wlan())
            .filter_map(|interface| interface.parent.as_deref())
            .collect::<HashSet<_>>();
        let configured_wlan_parents = config
            .interface
            .values()
            .filter_map(|interface_config| match &interface_config.creation {
                Some(LinkOptions::Wlan { parent, .. }) => Some(parent.as_str()),
                _ => None,
            })
            .collect::<HashSet<_>>();
        let mut interface_names = interfaces
            .values()
            .map(|interface| interface.name.clone())
            .collect::<HashSet<_>>();
        let mut next_index = 0u32;
        let mut created = false;

        for parent in devices {
            if existing_parents.contains(parent.as_str())
                || configured_wlan_parents.contains(parent.as_str())
            {
                continue;
            }
            if let Some(interface_config) = config.interface.get(&parent)
                && (!interface_config.create_wlan
                    || interface_config.wlan.is_some())
            {
                continue;
            }

            let name = loop {
                let candidate = format!("wlan{next_index}");
                next_index += 1;
                if interface_names.insert(candidate.clone()) {
                    break candidate;
                }
            };
            let options = WlanLinkOptions::default();
            let link = LinkOptions::Wlan {
                parent: parent.clone(),
                options,
            };
            match self.ifconfig.create_link(&name, &link).await {
                Ok(()) => {
                    info!(
                        "Created WLAN interface {} for wireless device {}",
                        name, parent
                    );
                    created = true;
                }
                Err(error) => {
                    warn!(
                        "Could not create WLAN interface {} for {}: {}",
                        name, parent, error
                    );
                }
            }
        }
        created
    }

    fn persist_creation(
        &self,
        name: &str,
        creation: &LinkOptions,
    ) -> Result<(), InterfaceError> {
        let mut config = match DaemonConfig::load(Path::new(CONFIG_PATH)) {
            Ok(config) => config,
            Err(ConfigError::NotFound(_)) => DaemonConfig::default(),
            Err(error) => {
                return Err(InterfaceError::Other(format!(
                    "could not load interface configuration: {error}"
                )));
            }
        };
        config
            .interface
            .entry(name.to_string())
            .or_default()
            .creation = Some(creation.clone());
        config.save(Path::new(CONFIG_PATH)).map_err(|error| {
            InterfaceError::Other(format!(
                "could not persist interface {name}: {error}"
            ))
        })
    }

    fn remove_persisted_creation(
        &self,
        name: &str,
    ) -> Result<(), InterfaceError> {
        let mut config = match DaemonConfig::load(Path::new(CONFIG_PATH)) {
            Ok(config) => config,
            Err(ConfigError::NotFound(_)) => return Ok(()),
            Err(error) => {
                return Err(InterfaceError::Other(format!(
                    "could not load interface configuration: {error}"
                )));
            }
        };
        let changed = config
            .interface
            .get_mut(name)
            .and_then(|interface| interface.creation.take())
            .is_some();
        if changed {
            config.save(Path::new(CONFIG_PATH)).map_err(|error| {
                InterfaceError::Other(format!(
                    "could not persist interface deletion {name}: {error}"
                ))
            })?;
        }
        Ok(())
    }

    fn persist_policy(
        &self,
        name: &str,
        slaac: Modification<bool>,
        dhcpv4: Modification<bool>,
    ) -> Result<(), InterfaceError> {
        let slaac_value = match slaac {
            Modification::Replace(value) => Some(value),
            _ => None,
        };
        let dhcpv4_value = match dhcpv4 {
            Modification::Replace(value) => Some(value),
            _ => None,
        };
        if slaac_value.is_none() && dhcpv4_value.is_none() {
            return Ok(());
        }
        let mut config = match DaemonConfig::load(Path::new(CONFIG_PATH)) {
            Ok(config) => config,
            Err(ConfigError::NotFound(_)) => DaemonConfig::default(),
            Err(error) => {
                return Err(InterfaceError::Other(format!(
                    "could not load interface configuration: {error}"
                )));
            }
        };
        let interface = config.interface.entry(name.to_string()).or_default();
        if let Some(value) = slaac_value {
            interface.slaac = value;
        }
        if let Some(value) = dhcpv4_value {
            interface.dhcpv4 = value;
        }
        config.save(Path::new(CONFIG_PATH)).map_err(|error| {
            InterfaceError::Other(format!(
                "could not persist interface policy {name}: {error}"
            ))
        })
    }
}

impl InterfaceManager {
    /// List all interfaces
    pub fn list_interfaces(&self) -> Vec<InterfaceInfo> {
        self.table.all()
    }

    /// Add an interface
    pub fn add_interface(&mut self, info: InterfaceInfo) {
        self.table.insert(info);
    }

    /// Get interface by name
    pub fn get_interface(&self, name: &str) -> Option<InterfaceInfo> {
        self.table.get(name).cloned()
    }

    /// Get interface by ifindex
    pub fn get_interface_by_index(
        &self,
        ifindex: u32,
    ) -> Option<InterfaceInfo> {
        self.table.get_by_index(ifindex).cloned()
    }

    /// List interfaces by type
    pub fn list_by_type(
        &self,
        interface_type: InterfaceType,
    ) -> Vec<InterfaceInfo> {
        self.table.list_by_type(&interface_type)
    }

    /// List interfaces by parent device
    pub fn list_by_parent(&self, parent: &str) -> Vec<InterfaceInfo> {
        self.table.list_by_parent(parent)
    }

    /// List interfaces by connection state
    pub fn list_by_state(&self, state: ConnectionState) -> Vec<InterfaceInfo> {
        self.table.list_by_state(&state)
    }

    /// Remove an interface by name
    pub fn remove_interface(
        &mut self,
        name: &str,
    ) -> Result<(), InterfaceError> {
        self.table
            .remove(name)
            .map(|_| ())
            .ok_or_else(|| InterfaceError::NotFound(name.to_string()))
    }
}

impl Actor for InterfaceManager {
    type Args = Self;

    type Error = InterfaceError;

    async fn on_start(
        args: Self::Args,
        actor_ref: ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        ensure!(
            actor_ref.register("InterfaceManager"),
            "Failed to register InterfaceManager"
        );
        ensure!(
            actor_ref.tell(InterfaceManagerAction::ForceRefresh).await,
            "Failed to send ForceRefresh to self"
        );

        let netlink_listener = Box::pin(
            NetlinkListener::new(
                ffi::RTMGRP_LINK
                    | ffi::RTMGRP_IPV4_IFADDR
                    | ffi::RTMGRP_IPV6_IFADDR,
            )
            .map_err(|e| InterfaceError::NetlinkListenerError(e.into()))?,
        );

        // Attach netlink subscriber
        actor_ref.attach_stream(netlink_listener, (), ());
        Ok(args)
    }

    async fn on_link_died(
        &mut self,
        _actor_ref: WeakActorRef<Self>,
        id: ActorId,
        reason: ActorStopReason,
    ) -> Result<ControlFlow<ActorStopReason>, Self::Error> {
        error!("Linked actor {:?} died: {:?}", id, reason);
        Ok(ControlFlow::Break(ActorStopReason::LinkDied {
            id,
            reason: Box::new(reason),
        }))
    }
}

impl Message<InterfaceManagerAction> for InterfaceManager {
    type Reply = Result<InterfaceResponse, InterfaceError>;

    async fn handle(
        &mut self,
        msg: InterfaceManagerAction,
        ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        match msg {
            InterfaceManagerAction::AddLink {
                name,
                kind,
                options,
            } => {
                if self.get_interface(&name).is_some() {
                    return Err(InterfaceError::Other(format!(
                        "interface already exists: {name}"
                    )));
                }

                let creation = match kind {
                    InterfaceType::Wlan => match options {
                        Some(LinkOptions::Wlan { parent, options }) => {
                            LinkOptions::Wlan { parent, options }
                        }
                        Some(_) => {
                            return Err(InterfaceError::Other(
                                "WLAN creation requires WLAN link options"
                                    .to_string(),
                            ));
                        }
                        None => {
                            let parent = self
                                .ifconfig
                                .wireless_devices()
                                .await?
                                .into_iter()
                                .next()
                                .ok_or_else(|| {
                                    InterfaceError::Other(
                                        "no wireless device is available"
                                            .to_string(),
                                    )
                                })?;
                            LinkOptions::Wlan {
                                parent,
                                options: WlanLinkOptions::default(),
                            }
                        }
                    },
                    InterfaceType::Bridge => match options {
                        None => LinkOptions::Bridge {
                            members: Vec::new(),
                        },
                        Some(LinkOptions::Bridge { members }) => {
                            LinkOptions::Bridge { members }
                        }
                        Some(_) => {
                            return Err(InterfaceError::Other(
                                "bridge creation requires bridge link options"
                                    .to_string(),
                            ));
                        }
                    },
                    InterfaceType::Lagg => match options {
                        None => LinkOptions::Lagg {
                            protocol: Default::default(),
                            members: Vec::new(),
                        },
                        Some(LinkOptions::Lagg { protocol, members }) => {
                            LinkOptions::Lagg { protocol, members }
                        }
                        Some(_) => {
                            return Err(InterfaceError::Other(
                                "lagg creation requires lagg link options"
                                    .to_string(),
                            ));
                        }
                    },
                    InterfaceType::Vlan => match options {
                        Some(LinkOptions::Vlan { parent, tag }) => {
                            LinkOptions::Vlan { parent, tag }
                        }
                        None => {
                            return Err(InterfaceError::Other(
                                "VLAN creation requires a parent and tag"
                                    .to_string(),
                            ));
                        }
                        Some(_) => {
                            return Err(InterfaceError::Other(
                                "VLAN creation requires VLAN link options"
                                    .to_string(),
                            ));
                        }
                    },
                    _ => {
                        return Err(InterfaceError::Other(format!(
                            "{kind:?} interfaces are provided by the kernel and can only be configured"
                        )));
                    }
                };

                self.ifconfig.create_link(&name, &creation).await?;
                if let Err(error) = self.persist_creation(&name, &creation) {
                    if let Err(cleanup) =
                        self.ifconfig.destroy_link(&name).await
                    {
                        warn!(
                            "Could not roll back interface {} after persistence failure: {}",
                            name, cleanup
                        );
                    }
                    return Err(error);
                }

                self.refresh_interfaces(ctx.actor_ref()).await?;
                Ok(InterfaceResponse::Success(()))
            }
            InterfaceManagerAction::DelLink { name } => {
                let mut modifier = NetlinkModifier::new()?;
                if let Some(info) = self.get_interface(&name) {
                    modifier.del_link(info.id).await?;
                } else {
                    return Err(InterfaceError::NotFound(name));
                }

                self.remove_persisted_creation(&name)?;

                Ok(InterfaceResponse::Success(()))
            }
            InterfaceManagerAction::ModLink {
                name,
                ipv4,
                ipv6,
                oper_state,
                slaac,
                dhcpv4,
            } => {
                async fn mod_ip<T>(
                    ifindex: u32,
                    modification: Modification<T>,
                    current_addresses: &[PrefixedIpAddr],
                ) -> Result<(), InterfaceError>
                where
                    T: Into<PrefixedIpAddr>,
                {
                    let mut modifier = NetlinkModifier::new()
                        .map_err(|e| InterfaceError::Other(e.to_string()))?;
                    match modification {
                        Modification::Remove(ip) => {
                            modifier.del_address(ifindex, ip.into()).await?;
                        }
                        Modification::Append(ip) => {
                            modifier.add_address(ifindex, ip.into()).await?;
                        }
                        Modification::Clear => {
                            for addr in current_addresses {
                                modifier
                                    .del_address(ifindex, addr.clone())
                                    .await?;
                            }
                        }
                        Modification::Replace(ip) => {
                            for addr in current_addresses {
                                modifier
                                    .del_address(ifindex, addr.clone())
                                    .await?;
                            }

                            modifier.add_address(ifindex, ip.into()).await?;
                        }
                        Modification::NoChange => {}
                    }

                    Ok(())
                }

                let info = match self.get_interface(&name) {
                    Some(i) => i,
                    None => {
                        return Err(InterfaceError::NotFound(name));
                    }
                };

                let ipv4_addresses = info
                    .ipv4_addrs
                    .iter()
                    .cloned()
                    .map(Into::into)
                    .collect::<Vec<PrefixedIpAddr>>();
                let ipv6_addresses = info
                    .ipv6_addrs
                    .iter()
                    .cloned()
                    .map(Into::into)
                    .collect::<Vec<PrefixedIpAddr>>();
                mod_ip(info.id, ipv4, &ipv4_addresses).await?;
                mod_ip(info.id, ipv6, &ipv6_addresses).await?;

                if let Modification::Replace(state) = oper_state {
                    let ifconfig = Ifconfig::new();
                    ifconfig.set_link_status(&name, state).await?;
                }

                // Apply/clear the SLAAC (kernel Router Advertisement) flag and
                // record it on the interface so status/TUI reflect the state.
                if let Modification::Replace(state) = slaac {
                    let ifconfig = Ifconfig::new();
                    ifconfig.set_slaac_state(&name, state).await?;
                    if let Some(mut info) = self.get_interface(&name) {
                        info.slaac_enabled = state;
                        self.add_interface(info);
                    }
                }

                if let Modification::Replace(enabled) = dhcpv4
                    && let Some(mut info) = self.get_interface(&name)
                    && info.dhcpv4_enabled != enabled
                {
                    info.dhcpv4_enabled = enabled;
                    self.add_interface(info.clone());
                    self.event_broadcaster
                        .broadcast(InterfaceManagerEvent::InterfaceChanged(
                            info,
                        ))
                        .await;
                }
                self.persist_policy(&name, slaac, dhcpv4)?;
                Ok(InterfaceResponse::Success(()))
            }
            InterfaceManagerAction::DhcpV4Set {
                name,
                old_lease,
                new_lease,
            } => {
                let (old_ip, old_gateway) = if let Some(lease) = old_lease {
                    let prefix_len =
                        lease.subnet_mask.to_bits().leading_ones() as u8;
                    let ip = lease.assigned_ip.into_prefixed(prefix_len);
                    (Some(ip), lease.router.map(|r| r.into_prefixed(32)))
                } else {
                    (None, None)
                };

                let (new_ip, new_gateway) = if let Some(lease) = new_lease {
                    let prefix_len =
                        lease.subnet_mask.to_bits().leading_ones() as u8;
                    let ip = lease.assigned_ip.into_prefixed(prefix_len);
                    (Some(ip), lease.router.map(|r| r.into_prefixed(32)))
                } else {
                    (None, None)
                };

                let mut info = self
                    .get_interface(&name)
                    .ok_or(InterfaceError::NotFound(name))?;
                let index = info.id;
                let mut modifier = NetlinkModifier::new()?;

                let gateway_changed = old_gateway != new_gateway;
                if gateway_changed {
                    info.gateway_ipv4 = new_gateway;

                    // Update gateway
                    self.add_interface(info.clone());
                }

                if old_ip != new_ip {
                    if let Some(old_ip) = old_ip {
                        modifier.del_address(index, old_ip.into()).await?;
                    }

                    if let Some(new_ip) = new_ip {
                        modifier.add_address(index, new_ip.into()).await?;
                    }
                } else if gateway_changed {
                    // Only gateway changed, broadcast the change
                    self.event_broadcaster
                        .broadcast(InterfaceManagerEvent::InterfaceChanged(
                            info,
                        ))
                        .await;
                }

                Ok(InterfaceResponse::Success(()))
            }
            InterfaceManagerAction::ForceRefresh => {
                self.refresh_interfaces(ctx.actor_ref()).await?;

                Ok(InterfaceResponse::Success(()))
            }
            InterfaceManagerAction::GetInterfaceInfo { name } => {
                let info = self.get_interface(&name);
                if let Some(info) = info {
                    Ok(InterfaceResponse::Info(info))
                } else {
                    Err(InterfaceError::NotFound(name))
                }
            }
            InterfaceManagerAction::UpdateInterfaceInfo { event } => {
                match event {
                    InterfaceManagerEvent::InterfaceAdded(info) => {
                        self.add_interface(info.clone());
                        info!("Interface added: {}", info.name);
                        self.event_broadcaster
                            .broadcast(InterfaceManagerEvent::InterfaceAdded(
                                info,
                            ))
                            .await;
                        Ok(InterfaceResponse::Success(()))
                    }
                    InterfaceManagerEvent::InterfaceRemoved(info) => {
                        if let Err(e) = self.remove_interface(&info.name)
                            && !matches!(e, InterfaceError::NotFound(_))
                        {
                            return Err(e);
                        }
                        Ok(InterfaceResponse::Success(()))
                    }
                    InterfaceManagerEvent::InterfaceChanged(mut info) => {
                        if let Some(current_info) = self.table.get(&info.name) {
                            if current_info.state == ConnectionState::Unmanaged
                            {
                                info.state = ConnectionState::Unmanaged;
                            } else if current_info.interface_type
                                == InterfaceType::Wlan
                            {
                                // The state of wireless interfaces is managed
                                // externally
                                info.state = current_info.state;
                            }

                            // TODO: Check whether the gateway is in the same
                            // subnet
                            info.gateway_ipv4 =
                                current_info.gateway_ipv4.clone();
                            info.gateway_ipv6 =
                                current_info.gateway_ipv6.clone();
                            info.dhcpv4_enabled = current_info.dhcpv4_enabled;
                            info.slaac_enabled = current_info.slaac_enabled;
                        }

                        self.table.update(info.clone());

                        self.event_broadcaster
                            .broadcast(InterfaceManagerEvent::InterfaceChanged(
                                info,
                            ))
                            .await;

                        Ok(InterfaceResponse::Success(()))
                    }
                }
            }
            InterfaceManagerAction::UpdateExternalInterfaceState {
                name,
                state,
            } => {
                if let Some(mut info) = self.table.get(&name).cloned() {
                    info.state = state;
                    self.table.update(info.clone());

                    self.event_broadcaster
                        .broadcast(InterfaceManagerEvent::InterfaceChanged(
                            info,
                        ))
                        .await;

                    Ok(InterfaceResponse::Success(()))
                } else {
                    Err(InterfaceError::NotFound(name))
                }
            }
            InterfaceManagerAction::MarkAsUnManaged { name } => {
                if let Some(mut info) = self.get_interface(&name) {
                    info.state = ConnectionState::Unmanaged;
                    self.table.update(info.clone());

                    self.event_broadcaster
                        .broadcast(InterfaceManagerEvent::InterfaceChanged(
                            info,
                        ))
                        .await;

                    Ok(InterfaceResponse::Success(()))
                } else {
                    Err(InterfaceError::NotFound(name))
                }
            }
            InterfaceManagerAction::GetAllInterfaces => {
                let list = self.list_interfaces();
                Ok(InterfaceResponse::InfoList(list))
            }
            InterfaceManagerAction::GetInterfacesByType { interface_type } => {
                let list = self.list_by_type(interface_type);
                Ok(InterfaceResponse::InfoList(list))
            }
            InterfaceManagerAction::GetInterfacesByParent { parent } => {
                let list = self.list_by_parent(&parent);
                Ok(InterfaceResponse::InfoList(list))
            }
            InterfaceManagerAction::GetInterfacesByState { state } => {
                let list = self.list_by_state(state);
                Ok(InterfaceResponse::InfoList(list))
            }
            InterfaceManagerAction::GetWirelessDevices => {
                Ok(InterfaceResponse::WirelessDevices(
                    self.ifconfig.wireless_devices().await?,
                ))
            }
            InterfaceManagerAction::SubscribeEvents => {
                let subscriber = self.event_broadcaster.subscribe();
                Ok(InterfaceResponse::EventReceiver(subscriber))
            }
        }
    }
}

impl
    Message<
        StreamMessage<
            Result<NetlinkMessage<RouteNetlinkMessage>, DecodeError>,
            (),
            (),
        >,
    > for InterfaceManager
{
    type Reply = ();

    async fn handle(
        &mut self,
        msg: StreamMessage<
            Result<NetlinkMessage<RouteNetlinkMessage>, DecodeError>,
            (),
            (),
        >,
        ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        let mut query = NetlinkQuery::new().unwrap();
        let actorref = ctx.actor_ref();
        let msg = if let StreamMessage::Next(item) = msg {
            match item {
                Ok(msg) => msg,
                Err(e) => {
                    error!("Netlink stream error: {}", e);
                    return;
                }
            }
        } else {
            return;
        };
        match msg.payload {
            NetlinkPayload::InnerMessage(inner) => {
                info!("Received netlink message: {:?}", inner);
                match inner {
                    RouteNetlinkMessage::DelLink(msg) => {
                        let index = msg.header.index;
                        if let Some(iface) = self.get_interface_by_index(index)
                        {
                            actorref
                                .tell(InterfaceManagerAction::UpdateInterfaceInfo {
                                    event: InterfaceManagerEvent::InterfaceRemoved(iface.clone()),
                                })
                                .await
                                .ok();
                        } else {
                            warn!(
                                "Received DelLink for unknown interface \
                                 index: {}",
                                index
                            );
                        }
                    }
                    RouteNetlinkMessage::NewLink(msg) => {
                        let index = msg.header.index;
                        // Get interface info
                        let iface = match query.query_link(index).await {
                            Ok(v) => v,
                            Err(e) => {
                                error!(
                                    "Failed to query interface info for index \
                                     {}: {}",
                                    index, e
                                );
                                return;
                            }
                        };
                        let dying = iface.is_none()
                            || msg.header.flags.contains(LinkFlags::Dying);
                        let cache = self.get_interface_by_index(index);

                        let evt = match (dying, cache, iface) {
                            (true, Some(info), _) => Some(
                                InterfaceManagerEvent::InterfaceRemoved(info),
                            ),
                            (false, Some(_), Some(info)) => Some(
                                InterfaceManagerEvent::InterfaceChanged(info),
                            ),
                            (false, None, Some(info)) => Some(
                                InterfaceManagerEvent::InterfaceAdded(info),
                            ),
                            _ => None,
                        };

                        if let Some(evt) = evt {
                            actorref
                                .tell(InterfaceManagerAction::UpdateInterfaceInfo { event: evt })
                                .await
                                .ok();
                        }
                    }
                    RouteNetlinkMessage::DelAddress(msg) => {
                        let index = msg.header.index;
                        let iface = match query.query_link(index).await {
                            Ok(v) => v,
                            Err(e) => {
                                error!(
                                    "Failed to query interface info for index \
                                     {}: {}",
                                    index, e
                                );
                                return;
                            }
                        };
                        if let Some(iface) = iface {
                            info!(
                                "Address removed on interface index: {}",
                                index
                            );
                            actorref
                                .tell(InterfaceManagerAction::UpdateInterfaceInfo {
                                    event: InterfaceManagerEvent::InterfaceChanged(iface),
                                })
                                .await
                                .ok();
                        } else {
                            warn!(
                                "Received DelAddress for unknown interface \
                                 index: {}",
                                index
                            );
                        }
                    }
                    RouteNetlinkMessage::NewAddress(msg) => {
                        let index = msg.header.index;
                        info!("New address on interface index: {}", index);
                        let iface = match query.query_link(index).await {
                            Ok(v) => v,
                            Err(e) => {
                                error!(
                                    "Failed to query interface info for index \
                                     {}: {}",
                                    index, e
                                );
                                return;
                            }
                        };
                        if let Some(iface) = iface {
                            actorref
                                .tell(InterfaceManagerAction::UpdateInterfaceInfo {
                                    event: InterfaceManagerEvent::InterfaceChanged(iface),
                                })
                                .await
                                .ok();
                        } else {
                            warn!(
                                "Received NewAddress for unknown interface \
                                 index: {}",
                                index
                            );
                        }
                    }
                    o => {
                        warn!("Unhandled netlink message: {:?}", o);
                    }
                }
            }
            NetlinkPayload::Error(err) => {
                error!("Netlink error received: {:?}", err);
            }
            _ => {}
        }
    }
}
