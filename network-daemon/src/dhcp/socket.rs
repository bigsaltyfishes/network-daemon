use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use async_io::Async;
use dhcproto::{Decodable, v4};
use etherparse::PacketHeaders;
use futures_lite::{AsyncWrite, Stream};
use pcap::{Active, Capture};

const BPF_FILTER: &str = "udp src port 67 and udp dst port 68";

pub struct DhcpSocket {
    inner: Async<Capture<Active>>,
}

impl DhcpSocket {
    pub fn new(iface: &str) -> io::Result<Self> {
        let mut cap = Capture::from_device(iface)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?
            .immediate_mode(true)
            .promisc(true)
            .buffer_size(4096)
            .open()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        cap.filter(BPF_FILTER, true)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        cap = cap
            .setnonblock()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        Ok(DhcpSocket {
            inner: Async::new(cap)?,
        })
    }

    fn parse_packet(data: &[u8]) -> Option<v4::Message> {
        let parsed_packet = PacketHeaders::from_ethernet_slice(data).ok()?;
        let transport = parsed_packet.transport?;
        let udp = transport.udp()?;

        if udp.source_port != 67 {
            return None;
        }

        let payload = parsed_packet.payload;
        let mut decoder = v4::Decoder::new(payload.slice());

        v4::Message::decode(&mut decoder).ok()
    }
}

impl AsyncWrite for DhcpSocket {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let socket = &mut self.get_mut().inner;

        match socket.poll_writable(cx) {
            Poll::Ready(Ok(())) => {
                // SAFETY: We own self and do not move internal data
                let cap = unsafe { socket.get_mut() };

                match cap.sendpacket(buf) {
                    Ok(_) => Poll::Ready(Ok(buf.len())),
                    Err(e) => Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::Other,
                        e,
                    ))),
                }
            }
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
        }
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

impl Stream for DhcpSocket {
    type Item = io::Result<v4::Message>;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let socket = &mut self.get_mut().inner;

        loop {
            match socket.poll_readable(cx) {
                Poll::Ready(Ok(())) => {
                    // SAFETY: We own self and do not move internal data
                    let cap = unsafe { socket.get_mut() };

                    match cap.next_packet() {
                        Ok(packet) => {
                            if let Some(msg) =
                                DhcpSocket::parse_packet(packet.data)
                            {
                                return Poll::Ready(Some(Ok(msg)));
                            }
                        }
                        Err(pcap::Error::TimeoutExpired) => {
                            return Poll::Pending;
                        }
                        Err(pcap::Error::NoMorePackets) => {
                            return Poll::Ready(None);
                        }
                        Err(e) => {
                            return Poll::Ready(Some(Err(io::Error::new(
                                io::ErrorKind::Other,
                                e,
                            ))));
                        }
                    }
                }
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => match e.kind() {
                    io::ErrorKind::WouldBlock => continue,
                    _ => return Poll::Ready(Some(Err(e))),
                },
            }
        }
    }
}
