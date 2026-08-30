use std::{collections::HashMap, ffi::CString, ptr};

use libnetwork_daemon::{
    ConnectionState, InterfaceInfo, InterfaceType, IntoPrefixed, MacAddr,
    error::NetlinkQueryError,
};
use netlink_packet_core::{NLM_F_DUMP, NLM_F_DUMP_FILTERED, NLM_F_REQUEST};
use netlink_packet_route::{
    AddressFamily, RouteNetlinkMessage,
    address::{AddressAttribute, AddressMessage},
    link::{
        InfoKind, LinkAttribute, LinkFlags, LinkInfo, LinkLayerType,
        LinkMessage, State,
    },
};

use crate::netlink::NetlinkQuery;

impl NetlinkQuery<InterfaceInfo> {
    /// Untilize funtion to extract address messages to InterfaceInfo
    fn extract_addresses(msg: AddressMessage, interface: &mut InterfaceInfo) {
        let prefix = msg.header.prefix_len;
        msg.attributes
            .into_iter()
            .filter_map(|v| match v {
                AddressAttribute::Address(ip) => Some(ip),
                _ => None,
            })
            .for_each(|v| interface.add_addr(v.into_prefixed(prefix)));

        // For non-wireless interfaces, if we have an IP address assigned,
        // consider it connected. For wireless interfaces, we cannot determine
        // connection state here as its state is managed by wpa_supplicant.
        if (!interface.ipv4_addrs.is_empty()
            || !interface.ipv6_addrs.is_empty())
            && interface.state != ConnectionState::NoCarrier
            && interface.interface_type != InterfaceType::Wlan
        {
            interface.state = ConnectionState::Connected;
        }
    }

    pub async fn query_links(
        &mut self,
    ) -> Result<HashMap<u32, InterfaceInfo>, NetlinkQueryError> {
        let mut interfaces = HashMap::new();

        // Disable strict checking
        self.inner_mut()
            .set_filter_enabled(false)
            .map_err(|e| NetlinkQueryError::IoError(e.into()))?;

        // Step 1: Get Links
        let mut link_msg = LinkMessage::default();
        link_msg.header.interface_family = AddressFamily::Unspec;

        self.query(
            NLM_F_REQUEST | NLM_F_DUMP,
            RouteNetlinkMessage::GetLink(link_msg),
            |msg| {
                if let RouteNetlinkMessage::NewLink(link) = msg {
                    let info = Self::build_info(link);
                    interfaces.insert(info.id, info);
                }
            },
        )
        .await?;

        // Step 2: Get Addresses
        let addr_msg = AddressMessage::default();

        self.query(
            NLM_F_REQUEST | NLM_F_DUMP,
            RouteNetlinkMessage::GetAddress(addr_msg),
            |msg| {
                if let RouteNetlinkMessage::NewAddress(addr) = msg
                    && let Some(interface) =
                        interfaces.get_mut(&addr.header.index)
                {
                    Self::extract_addresses(addr, interface);
                }
            },
        )
        .await?;

        Ok(interfaces)
    }

    pub async fn query_link(
        &mut self,
        if_index: u32,
    ) -> Result<Option<InterfaceInfo>, NetlinkQueryError> {
        let mut result: Option<InterfaceInfo> = None;

        // Enable strict checking
        self.inner_mut()
            .set_filter_enabled(true)
            .map_err(|e| NetlinkQueryError::IoError(e.into()))?;

        // Step 1: Get Link
        let mut link_msg = LinkMessage::default();
        link_msg.header.interface_family = AddressFamily::Unspec;
        link_msg.header.index = if_index;

        self.query(
            NLM_F_REQUEST | NLM_F_DUMP_FILTERED,
            RouteNetlinkMessage::GetLink(link_msg),
            |msg| {
                if let RouteNetlinkMessage::NewLink(link) = msg {
                    let info = Self::build_info(link);
                    result = Some(info);
                }
            },
        )
        .await?;

        let mut interface = if let Some(interface) = result {
            interface
        } else {
            return Ok(None);
        };

        // Step 2: Get Addresses
        let mut addr_msg = AddressMessage::default();
        addr_msg.header.index = if_index;

        self.query(
            NLM_F_REQUEST | NLM_F_DUMP_FILTERED,
            RouteNetlinkMessage::GetAddress(addr_msg),
            |msg| {
                if let RouteNetlinkMessage::NewAddress(addr) = msg {
                    Self::extract_addresses(addr, &mut interface);
                }
            },
        )
        .await?;

        Ok(Some(interface))
    }

    fn build_info(msg: LinkMessage) -> InterfaceInfo {
        let mut info = InterfaceInfo {
            id: msg.header.index,
            state: if msg.header.flags.contains(LinkFlags::Up) {
                ConnectionState::Up
            } else {
                ConnectionState::Disabled
            },
            interface_type: match msg.header.link_layer_type {
                LinkLayerType::Ether => InterfaceType::Ethernet,
                LinkLayerType::Ieee80211 => InterfaceType::Wlan,
                _ => InterfaceType::Other,
            },
            ..Default::default()
        };

        for attr in msg.attributes {
            match attr {
                LinkAttribute::IfName(name) => info.name = name,
                LinkAttribute::OperState(state) => {
                    info.state = match state {
                        State::Up => ConnectionState::Up,
                        State::Down => {
                            if info.state == ConnectionState::Disabled {
                                // Interface is admin down
                                ConnectionState::Disabled
                            } else {
                                // Ethernet is not plugged
                                ConnectionState::NoCarrier
                            }
                        }
                        State::LowerLayerDown => ConnectionState::NoCarrier,
                        _ => ConnectionState::NotApplicable,
                    }
                }
                LinkAttribute::Address(mac) => {
                    if mac.len() == 6 {
                        let mut arr = [0u8; 6];
                        arr.copy_from_slice(&mac);
                        info.mac_addr = Some(MacAddr::new(arr));
                    }
                }
                LinkAttribute::LinkInfo(infos) => {
                    for sub in infos {
                        if let LinkInfo::Kind(kind) = sub {
                            info.interface_type = match kind {
                                InfoKind::Bridge => InterfaceType::Bridge,
                                InfoKind::Tun => InterfaceType::Tun,
                                InfoKind::Vlan => InterfaceType::Vlan,
                                InfoKind::Vxlan => InterfaceType::Vxlan,
                                InfoKind::GreTun => InterfaceType::GreTun,
                                InfoKind::Wireguard => InterfaceType::Wireguard,
                                InfoKind::Loopback => InterfaceType::Loopback,
                                InfoKind::Wlan => InterfaceType::Wlan,
                                InfoKind::Lagg => InterfaceType::Lagg,
                                InfoKind::Usbus => InterfaceType::Usbus,
                                InfoKind::Tap => InterfaceType::Tap,
                                InfoKind::Vmnet => InterfaceType::Vmnet,
                                InfoKind::Openvpn => InterfaceType::Openvpn,
                                InfoKind::Stf => InterfaceType::Stf,
                                InfoKind::Epair => InterfaceType::Epair,
                                InfoKind::Enc => InterfaceType::Enc,
                                InfoKind::Pflog => InterfaceType::Pflog,
                                InfoKind::Pfsync => InterfaceType::Pfsync,
                                InfoKind::Ipfw => InterfaceType::Ipfw,
                                InfoKind::Ipfwlog => InterfaceType::Ipfwlog,
                                InfoKind::Disc => InterfaceType::Disc,
                                InfoKind::Me => InterfaceType::Me,
                                InfoKind::Edsc => InterfaceType::Edsc,
                                InfoKind::Ipsec => InterfaceType::Ipsec,
                                InfoKind::Gif => InterfaceType::Gif,
                                _ => InterfaceType::Other,
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        if info.interface_type == InterfaceType::Wlan {
            info.parent = Self::query_wlan_parent(&info.name);
        }

        info
    }

    fn query_wlan_parent(name: &str) -> Option<String> {
        let num = name.trim_start_matches("wlan");
        let sysctl_name = format!("net.wlan.{}.%parent", num);
        let c_name = CString::new(sysctl_name).ok()?;

        let mut buf = [0u8; 32];
        let mut oldlen: libc::size_t = buf.len();

        let ret = unsafe {
            libc::sysctlbyname(
                c_name.as_ptr(),
                buf.as_mut_ptr() as *mut libc::c_void,
                &mut oldlen,
                ptr::null(),
                0,
            )
        };

        if ret == 0 && oldlen > 0 {
            let parent =
                String::from_utf8_lossy(&buf[..oldlen.saturating_sub(1)])
                    .into_owned();
            Some(parent)
        } else {
            None
        }
    }
}
