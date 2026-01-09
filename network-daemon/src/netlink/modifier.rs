use std::{
    net::IpAddr,
    sync::atomic::{AtomicU32, Ordering},
};

use async_io::Async;
use futures_lite::{AsyncReadExt, AsyncWriteExt};
use libnetwork_daemon::{PrefixedIpAddr, error::NetlinkQueryError};
use netlink_packet_core::{
    NLM_F_ACK, NLM_F_CREATE, NLM_F_EXCL, NLM_F_REQUEST, NetlinkMessage,
    NetlinkPayload,
};
use netlink_packet_route::{
    AddressFamily, RouteNetlinkMessage,
    address::{AddressAttribute, AddressMessage},
    link::LinkMessage,
    route::{
        RouteAddress, RouteAttribute, RouteMessage, RouteProtocol, RouteScope,
        RouteType,
    },
};
use route_manager::Route;
use tracing::warn;

use crate::netlink::NetlinkSocket;

pub struct NetlinkModifier {
    socket: Async<NetlinkSocket>,
    sequence_number: AtomicU32,
}

impl NetlinkModifier {
    pub fn new() -> Result<Self, NetlinkQueryError> {
        let socket = NetlinkSocket::new()
            .map_err(|e| NetlinkQueryError::IoError(e.into()))?;
        Ok(Self {
            socket: Async::new(socket)
                .map_err(|e| NetlinkQueryError::IoError(e.into()))?,
            sequence_number: AtomicU32::new(1),
        })
    }

    pub async fn del_link(
        &mut self,
        index: u32,
    ) -> Result<(), NetlinkQueryError> {
        let mut msg = LinkMessage::default();
        msg.header.index = index;

        let mut packet =
            NetlinkMessage::from(RouteNetlinkMessage::DelLink(msg));
        self.send_request(&mut packet).await
    }

    pub async fn add_address(
        &mut self,
        index: u32,
        addr: PrefixedIpAddr,
    ) -> Result<(), NetlinkQueryError> {
        let mut msg = AddressMessage::default();
        msg.header.index = index;
        let addr = match addr {
            PrefixedIpAddr::V4(v4) => {
                msg.header.family = AddressFamily::Inet;
                msg.header.prefix_len = v4.prefix_len;
                IpAddr::V4(v4.addr)
            }
            PrefixedIpAddr::V6(v6) => {
                msg.header.family = AddressFamily::Inet6;
                msg.header.prefix_len = v6.prefix_len;
                IpAddr::V6(v6.addr)
            }
        };
        msg.attributes.push(AddressAttribute::Address(addr));

        let mut packet =
            NetlinkMessage::from(RouteNetlinkMessage::NewAddress(msg));
        packet.header.flags =
            NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_EXCL;
        self.send_request(&mut packet).await
    }

    pub async fn del_route(
        &mut self,
        rt: &Route,
    ) -> Result<(), NetlinkQueryError> {
        let mut msg = RouteMessage::default();
        msg.header.address_family = match rt.destination() {
            IpAddr::V4(_) => AddressFamily::Inet,
            IpAddr::V6(_) => AddressFamily::Inet6,
        };
        msg.header.destination_prefix_length = rt.prefix();
        msg.header.protocol = RouteProtocol::Unspec;
        msg.header.scope = RouteScope::Universe;
        msg.header.kind = RouteType::Unicast;
        msg.attributes.push(RouteAttribute::Destination(
            match rt.destination() {
                IpAddr::V4(v4) => RouteAddress::Inet(v4),
                IpAddr::V6(v6) => RouteAddress::Inet6(v6),
            },
        ));

        if let Some(oif) = rt.if_index() {
            msg.attributes.push(RouteAttribute::Oif(oif));
        }

        let mut packet =
            NetlinkMessage::from(RouteNetlinkMessage::DelRoute(msg));
        if let Err(e) = self.send_request(&mut packet).await {
            warn!("Failed to delete route {:?}: {}", rt, e);
            Err(e)
        } else {
            Ok(())
        }
    }

    pub async fn add_route(
        &mut self,
        rt: &Route,
    ) -> Result<(), NetlinkQueryError> {
        let mut msg = RouteMessage::default();
        msg.header.address_family = match rt.destination() {
            IpAddr::V4(_) => AddressFamily::Inet,
            IpAddr::V6(_) => AddressFamily::Inet6,
        };
        msg.header.destination_prefix_length = rt.prefix();
        msg.header.protocol = RouteProtocol::Unspec;
        msg.header.scope = RouteScope::Universe;
        msg.header.kind = RouteType::Unicast;
        msg.attributes.push(RouteAttribute::Destination(
            match rt.destination() {
                IpAddr::V4(v4) => RouteAddress::Inet(v4),
                IpAddr::V6(v6) => RouteAddress::Inet6(v6),
            },
        ));

        if let Some(gw) = rt.gateway() {
            msg.attributes.push(RouteAttribute::Gateway(match gw {
                IpAddr::V4(v4) => RouteAddress::Inet(v4),
                IpAddr::V6(v6) => RouteAddress::Inet6(v6),
            }));
        }

        if let Some(oif) = rt.if_index() {
            msg.attributes.push(RouteAttribute::Oif(oif));
        }

        let mut packet =
            NetlinkMessage::from(RouteNetlinkMessage::NewRoute(msg));
        packet.header.flags =
            NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_EXCL;
        if let Err(e) = self.send_request(&mut packet).await {
            warn!("Failed to add route {:?}: {}", rt, e);
            Err(e)
        } else {
            Ok(())
        }
    }

    pub async fn del_address(
        &mut self,
        index: u32,
        addr: PrefixedIpAddr,
    ) -> Result<(), NetlinkQueryError> {
        let mut msg = AddressMessage::default();
        msg.header.index = index;
        let addr = match addr {
            PrefixedIpAddr::V4(v4) => {
                msg.header.family = AddressFamily::Inet;
                msg.header.prefix_len = v4.prefix_len;
                IpAddr::V4(v4.addr)
            }
            PrefixedIpAddr::V6(v6) => {
                msg.header.family = AddressFamily::Inet6;
                msg.header.prefix_len = v6.prefix_len;
                IpAddr::V6(v6.addr)
            }
        };
        msg.attributes.push(AddressAttribute::Address(addr));

        let mut packet =
            NetlinkMessage::from(RouteNetlinkMessage::DelAddress(msg));
        self.send_request(&mut packet).await
    }

    async fn send_request(
        &mut self,
        packet: &mut NetlinkMessage<RouteNetlinkMessage>,
    ) -> Result<(), NetlinkQueryError> {
        packet.header.sequence_number =
            self.sequence_number.fetch_add(1, Ordering::Relaxed);
        packet.header.flags |= NLM_F_REQUEST | NLM_F_ACK;

        packet.finalize();
        let mut buf = vec![0u8; packet.header.length as usize];
        packet.serialize(&mut buf);

        self.socket
            .write_all(&buf)
            .await
            .map_err(|e| NetlinkQueryError::IoError(e.into()))?;

        // Receive ACK
        let mut recv_buf = vec![0u8; 4096];
        self.socket
            .read(&mut recv_buf)
            .await
            .map_err(|e| NetlinkQueryError::IoError(e.into()))?;

        let ack = NetlinkMessage::<RouteNetlinkMessage>::deserialize(&recv_buf)
            .map_err(|e| NetlinkQueryError::DecodeError(e.into()))?;

        if let NetlinkPayload::Error(e) = ack.payload {
            if let Some(e) = e.code {
                return Err(NetlinkQueryError::NetlinkError(e.into()));
            }
        }
        Ok(())
    }
}
