use std::{net::Ipv4Addr, ops::ControlFlow, time::Duration};

use dhcproto::{Encodable, Encoder, v4, v6::EncodeError};
use etherparse::{PacketBuilder, err::packet::BuildWriteError};
use futures_lite::{AsyncWriteExt, StreamExt};
use kameo::{
    Actor,
    actor::{ActorId, ActorRef, Spawn, WeakActorRef},
    error::ActorStopReason,
    prelude::{Context, Message},
};
use libnetwork_daemon::{Lease, LeaseBuilder, MacAddr, ignore};
use thiserror::Error;
use tracing::{info, warn};

use crate::dhcp::{
    DHCP_CLIENT_PORT, DHCP_SERVER_PORT, DhcpManager,
    lease::{LeaseEvent, LeaseWatchdog},
    socket::DhcpSocket,
    utils::{AttemptError, attempt},
};

const BROADCAST_MAC: [u8; 6] = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];

#[derive(Debug, Error)]
pub enum DhcpClientError {
    #[error("Dhcp Packet encode error")]
    DhcpPacketEncodeError(#[from] EncodeError),
    #[error("Ethernet packet build error")]
    EthernetPacketBuildError(#[from] BuildWriteError),
    #[error("Socket error: {0}")]
    SocketError(#[from] std::io::Error),
    #[error("DHCP transaction failed")]
    TransactionFailed,
    #[error("DHCP Server rejected the request")]
    ServerRejected,
    #[error("Invalid DHCP response")]
    InvalidResponse,
}

/// Helper struct to build DHCP packets.
#[derive(Default)]
pub struct DhcpPacketBuilder(v4::Message);

impl DhcpPacketBuilder {
    pub fn set_xid(mut self, xid: u32) -> Self {
        self.0.set_xid(xid);
        self
    }

    pub fn set_htype(mut self, htype: v4::HType) -> Self {
        self.0.set_htype(htype);
        self
    }

    pub fn set_opcode(mut self, opcode: v4::Opcode) -> Self {
        self.0.set_opcode(opcode);
        self
    }

    pub fn set_broadcast(mut self, broadcast: bool) -> Self {
        if broadcast {
            self.0.set_flags(v4::Flags::new(0x8000));
        } else {
            self.0.set_flags(v4::Flags::new(0x0000));
        }
        self
    }

    pub fn set_chaddr(mut self, chaddr: &MacAddr) -> Self {
        self.0.set_chaddr(chaddr.as_bytes());
        self
    }

    pub fn set_ciaddr(mut self, ciaddr: Ipv4Addr) -> Self {
        self.0.set_ciaddr(ciaddr);
        self
    }

    pub fn insert_option(mut self, option: v4::DhcpOption) -> Self {
        self.0.opts_mut().insert(option);
        self
    }

    pub fn build(self) -> v4::Message {
        self.0
    }
}

pub struct DhcpClient {
    xid: u32,
    iface: String,
    mac: MacAddr,
    socket: DhcpSocket,
    lease: Option<Lease>,
    manager: ActorRef<DhcpManager>,
}

impl DhcpClient {
    pub fn new(
        iface: String,
        mac: MacAddr,
        manager: ActorRef<DhcpManager>,
    ) -> Result<Self, DhcpClientError> {
        let socket = DhcpSocket::new(&iface)?;
        Ok(DhcpClient {
            xid: 0,
            iface,
            mac,
            socket,
            lease: None,
            manager,
        })
    }

    fn build_packet(
        &self,
        dhcp_msg: v4::Message,
        src: Option<[u8; 4]>,
        dest: Option<[u8; 4]>,
    ) -> Result<Vec<u8>, DhcpClientError> {
        let mac = self.mac;
        let builder = PacketBuilder::ethernet2(*mac.as_bytes(), BROADCAST_MAC)
            .ipv4(
                src.unwrap_or([0, 0, 0, 0]),
                dest.unwrap_or([255, 255, 255, 255]),
                64,
            )
            .udp(DHCP_CLIENT_PORT, DHCP_SERVER_PORT);
        let mut dhcp_payload = Vec::new();
        dhcp_msg.encode(&mut Encoder::new(&mut dhcp_payload))?;

        let mut packet =
            Vec::<u8>::with_capacity(builder.size(dhcp_payload.len()));
        builder.write(&mut packet, &dhcp_payload)?;
        Ok(packet)
    }

    fn default_params() -> v4::DhcpOption {
        v4::DhcpOption::ParameterRequestList(vec![
            v4::OptionCode::SubnetMask,
            v4::OptionCode::Router,
            v4::OptionCode::DomainName,
            v4::OptionCode::DomainNameServer,
            v4::OptionCode::BroadcastAddr,
            v4::OptionCode::AddressLeaseTime,
            v4::OptionCode::Renewal,
            v4::OptionCode::Rebinding,
            v4::OptionCode::MessageType,
            v4::OptionCode::ServerIdentifier,
            v4::OptionCode::NtpServers,
            v4::OptionCode::NetBiosNodeType,
            v4::OptionCode::Hostname,
        ])
    }

    /// Start a new DHCP transaction by generating a new transaction ID.
    fn start_new_transaction(&mut self) {
        self.xid = rand::random();
    }

    fn construct_base(&self) -> DhcpPacketBuilder {
        DhcpPacketBuilder::default()
            .set_xid(self.xid)
            .set_htype(v4::HType::Eth)
            .set_opcode(v4::Opcode::BootRequest)
            .set_broadcast(true)
            .set_chaddr(&self.mac)
    }

    async fn request(&mut self) -> Result<Lease, DhcpClientError> {
        self.start_new_transaction();

        let params = Self::default_params();
        let dhcp_discover = self
            .construct_base()
            .insert_option(v4::DhcpOption::MessageType(
                v4::MessageType::Discover,
            ))
            .insert_option(params.clone())
            .build();

        let packet = self.build_packet(dhcp_discover, None, None)?;

        self.socket.write(&packet).await?;
        info!("DHCP Client [{}]: Sent DISCOVER", self.iface);

        // Wait for OFFER
        let offer = attempt(
            Some(Duration::from_secs(5)),
            3,
            self,
            async move |s| {
                s.socket.write(&packet).await.map_err(|_| ())?;
                Ok(())
            },
            async |s| {
                loop {
                    let offer = s.socket.next().await;
                    match offer {
                        Some(Ok(dhcp_msg)) => {
                            if dhcp_msg.xid() == s.xid
                                && let Some(v4::MessageType::Offer) =
                                    dhcp_msg.opts().msg_type()
                            {
                                info!(
                                    "DHCP Client [{}]: Received OFFER",
                                    s.iface
                                );
                                return ControlFlow::Break(Ok(dhcp_msg));
                            }
                            // Ignore other packets
                        }
                        e => {
                            warn!(
                                "DHCP Client [{}]: Error during OFFER: {:?}",
                                s.iface, e
                            );
                            return ControlFlow::Continue(());
                        }
                    }
                }
            },
        )
        .await
        .map_err(|e| match e {
            AttemptError::CrticalError(e) => e,
            _ => DhcpClientError::TransactionFailed,
        })?;

        // Send REQUEST
        let server_id = offer
            .opts()
            .get(v4::OptionCode::ServerIdentifier)
            .ok_or(DhcpClientError::InvalidResponse)?;
        let server_id = match server_id {
            v4::DhcpOption::ServerIdentifier(id) => id,
            _ => return Err(DhcpClientError::InvalidResponse),
        };
        let dhcp_request = self
            .construct_base()
            .insert_option(v4::DhcpOption::MessageType(
                v4::MessageType::Request,
            ))
            .insert_option(v4::DhcpOption::RequestedIpAddress(offer.yiaddr()))
            .insert_option(v4::DhcpOption::ServerIdentifier(*server_id))
            .insert_option(params)
            .build();

        let packet = self.build_packet(dhcp_request, None, None)?;

        self.socket.write(&packet).await?;

        info!("DHCP Client [{}]: Sent REQUEST", self.iface);

        // Wait for ACK
        let ack = attempt(
            Some(Duration::from_secs(5)),
            3,
            self,
            async move |s| {
                s.socket.write(&packet).await.map_err(|_| ())?;
                Ok(())
            },
            async |s| {
                loop {
                    let ack = s.socket.next().await;
                    match ack {
                        Some(Ok(dhcp_msg)) => {
                            if dhcp_msg.xid() == s.xid {
                                match dhcp_msg.opts().msg_type() {
                                    Some(v4::MessageType::Ack) => {
                                        info!(
                                            "DHCP Client [{}]: Received ACK",
                                            s.iface
                                        );
                                        return ControlFlow::Break(Ok(
                                            dhcp_msg,
                                        ));
                                    }
                                    Some(v4::MessageType::Nak) => {
                                        info!(
                                            "DHCP Client [{}]: Received NAK",
                                            s.iface
                                        );
                                        return ControlFlow::Break(Err(
                                            DhcpClientError::ServerRejected,
                                        ));
                                    }
                                    o => {
                                        info!(
                                            "DHCP Client [{}]: Received \
                                             unexpected message type: {:?}",
                                            s.iface, o
                                        );
                                        // Continue loop
                                    }
                                }
                            }
                            // Ignore other packets
                        }
                        e => {
                            warn!(
                                "DHCP Client [{}]: Error during RENEW: {:?}",
                                s.iface, e
                            );
                            return ControlFlow::Continue(());
                        }
                    }
                }
            },
        )
        .await
        .map_err(|e| match e {
            AttemptError::CrticalError(e) => e,
            _ => DhcpClientError::TransactionFailed,
        })?;

        let mut lease_builder = LeaseBuilder::new()
            .set_assigned_ip(ack.yiaddr())
            .set_server_id(*server_id);

        for (_code, opt) in ack.opts().iter() {
            match opt {
                v4::DhcpOption::SubnetMask(mask) => {
                    lease_builder = lease_builder.set_subnet_mask(*mask);
                }
                v4::DhcpOption::Router(router) => {
                    lease_builder = lease_builder.set_router(router[0]);
                }
                v4::DhcpOption::DomainNameServer(servers) => {
                    for dns in servers {
                        lease_builder = lease_builder.add_dns_server(*dns);
                    }
                }
                v4::DhcpOption::AddressLeaseTime(time) => {
                    lease_builder = lease_builder.set_lease_time(*time);
                }
                v4::DhcpOption::Renewal(time) => {
                    lease_builder = lease_builder.set_renewal_time(*time);
                }
                v4::DhcpOption::Rebinding(time) => {
                    lease_builder = lease_builder.set_rebinding_time(*time);
                }
                _ => {}
            }
        }

        let lease = lease_builder
            .build()
            .map_err(|_| DhcpClientError::InvalidResponse)?;
        info!("DHCP Client [{}]: Obtained Lease: {:?}", self.iface, lease);

        Ok(lease)
    }

    async fn renew(&mut self) -> Result<Lease, DhcpClientError> {
        self.start_new_transaction();

        let params = Self::default_params();
        let mut new_lease = self.lease.clone().unwrap();
        let request = self
            .construct_base()
            .set_ciaddr(new_lease.assigned_ip)
            .insert_option(v4::DhcpOption::MessageType(
                v4::MessageType::Request,
            ))
            .insert_option(params)
            .build();
        let packet = self.build_packet(
            request,
            Some(new_lease.assigned_ip.octets()),
            Some(new_lease.server_id.octets()),
        )?;
        self.socket.write(&packet).await?;
        info!("DHCP Client [{}]: Sent RENEW", self.iface);

        let ack = attempt(
            Some(Duration::from_secs(5)),
            3,
            self,
            async move |s| {
                s.socket.write(&packet).await.map_err(|_| ())?;
                Ok(())
            },
            async |s| {
                loop {
                    let resp = s.socket.next().await;
                    match resp {
                        Some(Ok(dhcp_msg)) => {
                            if dhcp_msg.xid() == s.xid {
                                match dhcp_msg.opts().msg_type() {
                                    Some(v4::MessageType::Ack) => {
                                        info!(
                                            "DHCP Client [{}]: Received RENEW \
                                             ACK",
                                            s.iface
                                        );
                                        return ControlFlow::Break(Ok(
                                            dhcp_msg,
                                        ));
                                    }
                                    Some(v4::MessageType::Nak) => {
                                        info!(
                                            "DHCP Client [{}]: Received RENEW \
                                             NAK",
                                            s.iface
                                        );
                                        return ControlFlow::Break(Err(
                                            DhcpClientError::ServerRejected,
                                        ));
                                    }
                                    o => {
                                        info!(
                                            "DHCP Client [{}]: Received \
                                             unexpected message type: {:?}",
                                            s.iface, o
                                        );
                                        // Continue loop
                                    }
                                }
                            }
                        }
                        e => {
                            warn!(
                                "DHCP Client [{}]: Error during RENEW: {:?}",
                                s.iface, e
                            );
                            return ControlFlow::Continue(());
                        }
                    }
                }
            },
        )
        .await
        .map_err(|e| match e {
            AttemptError::CrticalError(e) => e,
            _ => DhcpClientError::TransactionFailed,
        })?;

        // Build new lease from ACK
        for (_code, opt) in ack.opts().iter() {
            match opt {
                v4::DhcpOption::SubnetMask(mask) => {
                    new_lease.subnet_mask = *mask;
                }
                v4::DhcpOption::Router(router) => {
                    new_lease.router = Some(router[0]);
                }
                v4::DhcpOption::DomainNameServer(servers) => {
                    new_lease.dns_servers.clear();
                    for dns in servers {
                        new_lease.dns_servers.push(*dns);
                    }
                }
                v4::DhcpOption::AddressLeaseTime(time) => {
                    new_lease.lease_time = *time;
                }
                v4::DhcpOption::Renewal(time) => {
                    new_lease.renewal_time = *time;
                }
                v4::DhcpOption::Rebinding(time) => {
                    new_lease.rebinding_time = *time;
                }
                _ => {}
            }
        }

        info!(
            "DHCP Client [{}]: Renewed Lease: {:?}",
            self.iface, new_lease
        );

        Ok(new_lease)
    }

    async fn rebind(&mut self) -> Result<Lease, DhcpClientError> {
        self.start_new_transaction();

        let params = Self::default_params();
        let mut new_lease = self.lease.clone().unwrap();
        let request = self
            .construct_base()
            .set_ciaddr(new_lease.assigned_ip)
            .insert_option(v4::DhcpOption::MessageType(
                v4::MessageType::Request,
            ))
            .insert_option(params)
            .build();
        let packet = self.build_packet(
            request,
            Some(new_lease.assigned_ip.octets()),
            None,
        )?;
        self.socket.write(&packet).await?;
        info!("DHCP Client [{}]: Sent REBIND", self.iface);

        let ack = attempt(
            Some(Duration::from_secs(5)),
            3,
            self,
            async move |s| {
                s.socket.write(&packet).await.map_err(|_| ())?;
                Ok(())
            },
            async |s| {
                loop {
                    let resp = s.socket.next().await;
                    match resp {
                        Some(Ok(dhcp_msg)) => {
                            if dhcp_msg.xid() == s.xid {
                                match dhcp_msg.opts().msg_type() {
                                    Some(v4::MessageType::Ack) => {
                                        info!(
                                            "DHCP Client [{}]: Received \
                                             REBIND ACK",
                                            s.iface
                                        );
                                        return ControlFlow::Break(Ok(
                                            dhcp_msg,
                                        ));
                                    }
                                    Some(v4::MessageType::Nak) => {
                                        info!(
                                            "DHCP Client [{}]: Received \
                                             REBIND NAK",
                                            s.iface
                                        );
                                        return ControlFlow::Break(Err(
                                            DhcpClientError::ServerRejected,
                                        ));
                                    }
                                    o => {
                                        info!(
                                            "DHCP Client [{}]: Received \
                                             unexpected message type: {:?}",
                                            s.iface, o
                                        );
                                        // Continue loop
                                    }
                                }
                            }
                        }
                        e => {
                            warn!(
                                "DHCP Client [{}]: Error during REBIND: {:?}",
                                s.iface, e
                            );
                            return ControlFlow::Continue(());
                        }
                    }
                }
            },
        )
        .await
        .map_err(|e| match e {
            AttemptError::CrticalError(e) => e,
            _ => DhcpClientError::TransactionFailed,
        })?;

        // Build new lease from ACK
        for (_code, opt) in ack.opts().iter() {
            match opt {
                v4::DhcpOption::SubnetMask(mask) => {
                    new_lease.subnet_mask = *mask;
                }
                v4::DhcpOption::Router(router) => {
                    new_lease.router = Some(router[0]);
                }
                v4::DhcpOption::DomainNameServer(servers) => {
                    new_lease.dns_servers.clear();
                    for dns in servers {
                        new_lease.dns_servers.push(*dns);
                    }
                }
                v4::DhcpOption::AddressLeaseTime(time) => {
                    new_lease.lease_time = *time;
                }
                v4::DhcpOption::Renewal(time) => {
                    new_lease.renewal_time = *time;
                }
                v4::DhcpOption::Rebinding(time) => {
                    new_lease.rebinding_time = *time;
                }
                _ => {}
            }
        }

        info!(
            "DHCP Client [{}]: Rebound Lease: {:?}",
            self.iface, new_lease
        );

        Ok(new_lease)
    }

    pub async fn release(&mut self) -> Result<(), DhcpClientError> {
        if let Some(lease) = self.lease.clone() {
            self.start_new_transaction();
            let request = self
                .construct_base()
                .set_ciaddr(lease.assigned_ip)
                .insert_option(v4::DhcpOption::MessageType(
                    v4::MessageType::Release,
                ))
                .insert_option(v4::DhcpOption::ServerIdentifier(
                    lease.server_id,
                ))
                .build();

            let packet = self.build_packet(
                request,
                Some(lease.assigned_ip.octets()),
                Some(lease.server_id.octets()),
            )?;
            self.socket.write(&packet).await?;
            info!("DHCP Client [{}]: Sent RELEASE", self.iface);
        }
        Ok(())
    }

    #[allow(dead_code)] // DHCP DECLINE; used to report a bad lease
    pub async fn decline(
        &mut self,
        lease: Lease,
    ) -> Result<(), DhcpClientError> {
        self.start_new_transaction();
        let request = self
            .construct_base()
            .insert_option(v4::DhcpOption::MessageType(
                v4::MessageType::Decline,
            ))
            .insert_option(v4::DhcpOption::RequestedIpAddress(
                lease.assigned_ip,
            ))
            .insert_option(v4::DhcpOption::ServerIdentifier(lease.server_id))
            .build();

        let packet = self.build_packet(request, Some([0, 0, 0, 0]), None)?;
        self.socket.write(&packet).await?;
        info!("DHCP Client [{}]: Sent DECLINE", self.iface);
        Ok(())
    }
}

impl Actor for DhcpClient {
    type Args = Self;
    type Error = DhcpClientError;

    async fn on_start(
        mut args: Self::Args,
        actor_ref: ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        let lease = args.request().await?;
        args.lease = Some(lease.clone());
        let lease_watchdog = LeaseWatchdog::new(
            lease.renewal_time,
            lease.rebinding_time,
            lease.lease_time,
            actor_ref.clone(),
        );
        let watchdog = LeaseWatchdog::spawn(lease_watchdog);

        actor_ref.link(&watchdog).await;

        ignore!(args.manager.tell((None, args.lease.clone())).await);

        Ok(args)
    }

    async fn on_link_died(
        &mut self,
        _actor_ref: WeakActorRef<Self>,
        _id: ActorId,
        reason: ActorStopReason,
    ) -> Result<ControlFlow<ActorStopReason>, Self::Error> {
        match reason {
            ActorStopReason::Normal => {
                self.release().await?;
                Ok(ControlFlow::Break(ActorStopReason::Normal))
            }
            r => Ok(ControlFlow::Break(r)),
        }
    }
}

impl Message<LeaseEvent> for DhcpClient {
    type Reply = Result<(u32, u32, u32), DhcpClientError>;

    async fn handle(
        &mut self,
        msg: LeaseEvent,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        let ret = match msg {
            LeaseEvent::Rebinding => self.rebind().await,
            LeaseEvent::Renewal => self.renew().await,
            LeaseEvent::LeaseExpired => Err(DhcpClientError::ServerRejected),
        };

        let (expired, new_lease) = match ret {
            Ok(lease) => (false, Ok(lease)),
            Err(DhcpClientError::ServerRejected) => {
                warn!(
                    "DHCP Client [{}]: Server rejected the request or lease \
                     expired",
                    self.iface
                );
                let ret = attempt(
                    None,
                    3,
                    self,
                    async |_| Ok(()),
                    async |s| match s.request().await {
                        Ok(lease) => ControlFlow::Break(Ok(lease)),
                        Err(e) => match e {
                            DhcpClientError::ServerRejected => {
                                ControlFlow::Continue(())
                            }
                            _ => ControlFlow::Break(Err(e)),
                        },
                    },
                )
                .await
                .map_err(|e| match e {
                    AttemptError::CrticalError(e) => e,
                    _ => DhcpClientError::TransactionFailed,
                });

                match ret {
                    Ok(lease) => (false, Ok(lease)),
                    Err(e) => {
                        warn!(
                            "DHCP Client [{}]: Failed to obtain new lease: \
                             {:?}",
                            self.iface, e
                        );
                        (true, Err(DhcpClientError::ServerRejected))
                    }
                }
            }
            _ => (false, Err(DhcpClientError::TransactionFailed)),
        };

        if expired {
            let old_lease = self.lease.clone();
            self.lease = None;
            ignore!(self.manager.tell((old_lease, None)).await);
            Err(DhcpClientError::ServerRejected)
        } else {
            let new_lease = new_lease?;
            let old_lease = self.lease.clone();

            self.lease = Some(new_lease.clone());

            ignore!(
                self.manager
                    .tell((old_lease, Some(new_lease.clone())))
                    .await
            );

            Ok((
                new_lease.renewal_time,
                new_lease.rebinding_time,
                new_lease.lease_time,
            ))
        }
    }
}
