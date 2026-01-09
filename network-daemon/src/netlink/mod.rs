mod modifier;
mod query;
mod socket;

use std::task::Poll;

use async_io::Async;
use futures_lite::{AsyncReadExt, Stream};
use futures_util::FutureExt;
pub use modifier::NetlinkModifier;
use netlink_packet_core::{DecodeError, NetlinkMessage};
use netlink_packet_route::RouteNetlinkMessage;
pub use query::NetlinkQuery;
pub use socket::NetlinkSocket;

use crate::ffi;

/// Netlink listener service
///
/// Handles receiving netlink messages and broadcasting them to subscribers
pub struct NetlinkListener {
    socket: Async<NetlinkSocket>,
}

impl NetlinkListener {
    pub fn new(groups: u32) -> Result<Self, std::io::Error> {
        let mut socket = NetlinkSocket::new()?;

        let mut sockaddr: ffi::sockaddr_nl = unsafe { std::mem::zeroed() };
        sockaddr.nl_family = ffi::AF_NETLINK as _;
        sockaddr.nl_groups = groups;

        socket.bind(&sockaddr)?;

        Ok(Self {
            socket: Async::new(socket)?,
        })
    }
}

impl Stream for NetlinkListener {
    type Item = Result<NetlinkMessage<RouteNetlinkMessage>, DecodeError>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let socket = &mut self.get_mut().socket;
        let mut buffer = vec![0u8; 4096];

        match futures_lite::ready!(socket.read(&mut buffer).poll_unpin(cx)) {
            Ok(n) => {
                if n == 0 {
                    return Poll::Ready(None);
                }
                let message: Result<NetlinkMessage<RouteNetlinkMessage>, _> =
                    NetlinkMessage::deserialize(&buffer[..n]);

                Poll::Ready(Some(message))
            }
            Err(e) => {
                eprintln!("Failed to read from netlink socket: {}", e);
                Poll::Ready(None)
            }
        }
    }
}
