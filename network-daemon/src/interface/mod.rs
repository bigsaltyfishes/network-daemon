mod ifconfig;
mod query;
mod table;

use std::{collections::HashSet, ops::ControlFlow};

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
    LinkOptions, Modification, PrefixedIpAddr, ensure, error::InterfaceError,
    utils::Broadcast,
};
use netlink_packet_core::{DecodeError, NetlinkMessage, NetlinkPayload};
use netlink_packet_route::{RouteNetlinkMessage, link::LinkFlags};
use table::InterfaceTable;
use tracing::{error, info, warn};

use self::ifconfig::Ifconfig;
use crate::{
    ffi,
    netlink::{NetlinkListener, NetlinkModifier, NetlinkQuery},
};

/// Interface manager using libifconfig
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
        let mut new_interfaces_set = HashSet::new();

        query
            .query_links()
            .await
            .map_err(InterfaceError::NetlinkQueryError)?
            .into_values()
            .for_each(|v| {
                new_interfaces_set.insert(v.clone());
            });

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
                match kind {
                    InterfaceType::Bridge => {
                        self.ifconfig.create_bridge(&name).await?;
                    }
                    InterfaceType::Wlan => {
                        if let Some(LinkOptions::Wlan { parent, options }) =
                            options
                        {
                            self.ifconfig
                                .create_wlan(
                                    &name,
                                    &parent,
                                    options.mode,
                                    options.regdomain,
                                    options.region,
                                )
                                .await?;
                        } else {
                            Err("Missing WLAN link options for creation")?
                        }
                    }
                    _ => Err("Interface type not supported for creation")?,
                };

                Ok(InterfaceResponse::Success(()))
            }
            InterfaceManagerAction::DelLink { name } => {
                let mut modifier = NetlinkModifier::new()?;
                if let Some(info) = self.get_interface(&name) {
                    modifier.del_link(info.id).await?;
                } else {
                    Err(InterfaceError::NotFound(name))?;
                }

                Ok(InterfaceResponse::Success(()))
            }
            InterfaceManagerAction::ModLink {
                name,
                ipv4,
                ipv6,
                oper_state,
            } => {
                async fn mod_ip<T>(
                    info: &InterfaceInfo,
                    modification: Modification<T>,
                ) -> Result<(), InterfaceError>
                where
                    T: Into<PrefixedIpAddr>,
                {
                    let mut modifier = NetlinkModifier::new()
                        .map_err(|e| InterfaceError::Other(e.to_string()))?;
                    match modification {
                        Modification::Remove(ip) => {
                            modifier.del_address(info.id, ip.into()).await?;
                        }
                        Modification::Append(ip) => {
                            modifier.add_address(info.id, ip.into()).await?;
                        }
                        Modification::Clear => {
                            for addr in info.ipv4_addrs.iter() {
                                modifier
                                    .del_address(info.id, addr.clone().into())
                                    .await?;
                            }
                        }
                        Modification::Replace(ip) => {
                            for addr in info.ipv4_addrs.iter() {
                                modifier
                                    .del_address(info.id, addr.clone().into())
                                    .await?;
                            }

                            modifier.add_address(info.id, ip.into()).await?;
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

                mod_ip(&info, ipv4).await?;
                mod_ip(&info, ipv6).await?;

                if let Modification::Replace(state) = oper_state {
                    let ifconfig = Ifconfig::new();
                    ifconfig.set_link_status(&name, state).await?;
                }

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
