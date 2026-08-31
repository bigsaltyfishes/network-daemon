use std::sync::atomic::{AtomicU32, Ordering};

use async_io::Async;
use futures_lite::{AsyncReadExt, AsyncWriteExt};
use libnetwork_daemon::error::NetlinkQueryError;
use netlink_packet_core::{
    NLM_F_MULTIPART, NetlinkHeader, NetlinkMessage, NetlinkPayload,
};
use netlink_packet_route::RouteNetlinkMessage;

use super::NetlinkSocket;

pub struct NetlinkQuery<T = ()> {
    socket: Async<NetlinkSocket>,
    sequence_number: AtomicU32,
    _marker: std::marker::PhantomData<T>,
}

impl<T> NetlinkQuery<T> {
    pub fn new() -> Result<Self, NetlinkQueryError> {
        let socket = NetlinkSocket::new()
            .map_err(|e| NetlinkQueryError::IoError(e.into()))?;
        Ok(Self {
            socket: Async::new(socket)
                .map_err(|e| NetlinkQueryError::IoError(e.into()))?,
            sequence_number: AtomicU32::new(1),
            _marker: std::marker::PhantomData,
        })
    }

    /// Get a mutable reference to the inner NetlinkSocket
    pub fn inner_mut(&mut self) -> &mut NetlinkSocket {
        // SAFETY: Safe because the lifetime of the socket is managed by
        // NetlinkQuery
        unsafe { self.socket.get_mut() }
    }

    pub async fn query<F>(
        &mut self,
        flags: u16,
        message: RouteNetlinkMessage,
        mut callback: F,
    ) -> Result<(), NetlinkQueryError>
    where
        F: FnMut(RouteNetlinkMessage),
    {
        let mut packet = NetlinkMessage::new(
            NetlinkHeader::default(),
            NetlinkPayload::from(message),
        );
        packet.header.flags = flags;
        packet.header.sequence_number =
            self.sequence_number.fetch_add(1, Ordering::Relaxed);
        packet.finalize();

        let mut buf = vec![0u8; packet.buffer_len()];
        packet.serialize(&mut buf);
        self.socket
            .write_all(&buf)
            .await
            .map_err(|e| NetlinkQueryError::IoError(e.into()))?;

        let mut recv_buf = vec![0u8; 4096];
        'outer: loop {
            let nbytes = self
                .socket
                .read(&mut recv_buf)
                .await
                .map_err(|e| NetlinkQueryError::IoError(e.into()))?;
            let mut offset = 0;

            loop {
                if offset >= nbytes {
                    break;
                }

                let recv_packet =
                    match NetlinkMessage::<RouteNetlinkMessage>::deserialize(
                        &recv_buf[offset..],
                    ) {
                        Ok(p) => p,
                        Err(e) => {
                            // One bad message (e.g. a FreeBSD link attribute
                            // this crate doesn't model) must not abort the dump.
                            // Discard the rest of this buffer and read the next
                            // batch of messages from the kernel.
                            tracing::warn!(
                                "netlink decode error, skipping buffer: {}",
                                e
                            );
                            break;
                        }
                    };
                match recv_packet.payload {
                    NetlinkPayload::InnerMessage(msg) => callback(msg),
                    NetlinkPayload::Done(_) => break 'outer,
                    _ => {}
                }

                offset += recv_packet.header.length as usize;
                if recv_packet.header.length == 0 {
                    break;
                }
                if recv_packet.header.flags & NLM_F_MULTIPART == 0 {
                    break 'outer;
                }
            }
        }
        Ok(())
    }
}
