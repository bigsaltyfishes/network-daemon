mod backup;
mod table;

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use async_channel::Receiver;
use futures_lite::stream;
use kameo::{
    Actor,
    actor::ActorRef,
    message::StreamMessage,
    prelude::{Context, Message},
};
use libnetwork_daemon::{
    ConnectionState, InterfaceInfo, InterfaceManagerEvent, IntoPrefixed,
    PrefixedIpv4Addr, PrefixedIpv6Addr, ignore,
};
use route_manager::{
    AsyncRouteListener, AsyncRouteManager as RawRouteManager, Route,
    RouteChange,
};
use thiserror::Error;
use tracing::{error, info, warn};

use crate::route::{backup::BackupRouteTable, table::RouteTable};

#[derive(Debug, Error)]
pub enum RouteManagerError {
    #[error("Io error: {0}")]
    IoError(#[from] std::io::Error),
}

pub struct RouteManager {
    v4_backup: BackupRouteTable<PrefixedIpv4Addr>,
    v6_backup: BackupRouteTable<PrefixedIpv6Addr>,
    route_mgr: RawRouteManager,

    route_table: RouteTable,
}

impl RouteManager {
    /// Add default routes from backup table if needed
    pub async fn add_default_rt_if_needed(&mut self) {
        if self.route_table.find_default(false).is_none() {
            // No IPv4 default route, try to add one
            if let Some((gw, oif)) = self.v4_backup.available_gateways().next()
            {
                let default_route =
                    Route::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 0)
                        .with_gateway(gw.addr.into())
                        .with_if_index(oif);

                ignore!(self.route_mgr.add(&default_route).await);

                info!(
                    "Added IPv4 default route via {} on interface {}",
                    gw.addr, oif
                );
            }
        }

        if self.route_table.find_default(true).is_none() {
            // No IPv6 default route, try to add one
            if let Some((gw, oif)) = self.v6_backup.available_gateways().next()
            {
                let default_route = Route::new(
                    IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 0)),
                    0,
                )
                .with_gateway(gw.addr.into())
                .with_if_index(oif);

                ignore!(self.route_mgr.add(&default_route).await);

                info!(
                    "Added IPv6 default route via {} on interface {}",
                    gw.addr, oif
                );
            }
        }
    }

    async fn dead_rt_cleanup(&mut self, info: &InterfaceInfo) {
        // Remove dead routes
        if let Some(rts) = self.route_table.find_by_oif(info.id) {
            for rt in rts {
                // Some route may have been already removed by
                // kernel,
                // so ignore errors here.
                info!("Removing dead route: {:?}", rt);
                if let Err(e) = self.route_mgr.delete(rt).await {
                    warn!("Failed to remove dead route {:?}: {}", rt, e);
                }

                // Maybe there are another oif can reach the
                // same network,
                // we need to add them back
                match rt.destination() {
                    IpAddr::V4(ip) => {
                        let ip = ip.into_prefixed(rt.prefix());
                        if let Some(mut outputs) =
                            self.v4_backup.net_outputs(&ip)
                            && let Some(&output) =
                                outputs.find(|&&oif| oif != info.id)
                        {
                            // Kernel may have automatically
                            // added the route back,
                            // so ignore errors here.
                            // TODO: Check kernel behavior
                            info!(
                                "Adding back route via {} on interface {}",
                                ip, output
                            );
                            ignore!(
                                self.route_mgr
                                    .add(
                                        &Route::new(
                                            rt.destination(),
                                            rt.prefix()
                                        )
                                        .with_if_index(output)
                                    )
                                    .await
                            );
                        }
                    }
                    IpAddr::V6(ip) => {
                        let ip = ip.into_prefixed(rt.prefix());
                        if let Some(mut outputs) =
                            self.v6_backup.net_outputs(&ip)
                            && let Some(&output) =
                                outputs.find(|&&oif| oif != info.id)
                        {
                            // Kernel may have automatically
                            // added the route back,
                            // so ignore errors here.
                            // TODO: Check kernel behavior
                            info!(
                                "Adding back route via {} on interface {}",
                                ip, output
                            );
                            ignore!(
                                self.route_mgr
                                    .add(
                                        &Route::new(
                                            rt.destination(),
                                            rt.prefix()
                                        )
                                        .with_if_index(output)
                                    )
                                    .await
                            );
                        }
                    }
                }
            }

            // Default route addition will be handled in
            // `RouteChange` handler
        }
    }
}
impl Actor for RouteManager {
    type Args = Receiver<InterfaceManagerEvent>;
    type Error = RouteManagerError;

    async fn on_start(
        args: Self::Args,
        actor_ref: ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        let listener = Box::pin(stream::unfold(
            AsyncRouteListener::new()?,
            async |mut rt_listener| {
                loop {
                    match rt_listener.listen().await {
                        Ok(route) => return Some((route, rt_listener)),
                        Err(e) => {
                            error!("Route listen error: {}", e);
                            continue;
                        }
                    }
                }
            },
        ));
        let stream = Box::pin(args);
        let mut route_mgr = RawRouteManager::new()?;
        let mut route_table = RouteTable::new();

        // Query existing routes and populate route table
        route_mgr.list().await?.into_iter().for_each(|route| {
            route_table.insert(route);
        });

        // Attach streams
        actor_ref.attach_stream(stream, (), ());
        actor_ref.attach_stream(listener, (), ());

        info!("RouteManager started");

        Ok(Self {
            v4_backup: BackupRouteTable::new(),
            v6_backup: BackupRouteTable::new(),
            route_mgr,
            route_table,
        })
    }
}

impl Message<StreamMessage<InterfaceManagerEvent, (), ()>> for RouteManager {
    type Reply = ();

    async fn handle(
        &mut self,
        msg: StreamMessage<InterfaceManagerEvent, (), ()>,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        if let StreamMessage::Next(evt) = msg {
            match evt {
                InterfaceManagerEvent::InterfaceAdded(info) => {
                    if let (Some(gw), nets) =
                        (&info.gateway_ipv4, &info.ipv4_addrs)
                    {
                        self.v4_backup.insert(
                            info.id,
                            gw.clone(),
                            nets.iter().map(|ip| ip.net_id()),
                        );
                    }
                    if let Some(gw) = &info.gateway_ipv6 {
                        self.v6_backup.insert(
                            info.id,
                            gw.clone(),
                            info.ipv6_addrs.iter().map(|ip| ip.net_id()),
                        );
                    }

                    // After interface added, check and add default routes
                    // if needed, usually required
                    // when daemon just started.
                    self.add_default_rt_if_needed().await;
                }
                InterfaceManagerEvent::InterfaceRemoved(info) => {
                    self.v4_backup.remove(info.id);
                    self.v6_backup.remove(info.id);

                    // Clean up dead routes
                    self.dead_rt_cleanup(&info).await;
                }
                InterfaceManagerEvent::InterfaceChanged(info) => {
                    // Update IPv4 gateway
                    if let Some(gw) = &info.gateway_ipv4 {
                        self.v4_backup.insert(
                            info.id,
                            gw.clone(),
                            info.ipv4_addrs.iter().map(|ip| ip.net_id()),
                        );
                    } else {
                        self.v4_backup.remove(info.id);
                    }

                    // Update IPv6 gateway
                    if let Some(gw) = &info.gateway_ipv6 {
                        self.v6_backup.insert(
                            info.id,
                            gw.clone(),
                            info.ipv6_addrs.iter().map(|ip| ip.net_id()),
                        );
                    } else {
                        self.v6_backup.remove(info.id);
                    }

                    // Clean up dead routes
                    if info.state != ConnectionState::Connected {
                        self.v4_backup.remove(info.id);
                        self.v6_backup.remove(info.id);

                        self.dead_rt_cleanup(&info).await;
                    }

                    // Default route addition will be handled in
                    // `RouteChange` handler
                    self.add_default_rt_if_needed().await;
                }
            }
        }
    }
}

impl Message<StreamMessage<RouteChange, (), ()>> for RouteManager {
    type Reply = ();

    async fn handle(
        &mut self,
        msg: StreamMessage<RouteChange, (), ()>,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        match msg {
            StreamMessage::Next(change) => {
                info!("Route change detected: {:?}", change);
                match change {
                    RouteChange::Add(route) => {
                        self.route_table.insert(route);
                    }
                    RouteChange::Delete(route) => {
                        self.route_table.remove(
                            &route.destination().into_prefixed(route.prefix()),
                        );
                    }
                    RouteChange::Change(route) => {
                        self.route_table.remove(
                            &route.destination().into_prefixed(route.prefix()),
                        );
                        self.route_table.insert(route);
                    }
                }

                // After route change, check and add default routes if needed
                self.add_default_rt_if_needed().await;
            }
            StreamMessage::Finished(_) => panic!("Route listener stream ended"),
            _ => {}
        }
    }
}
