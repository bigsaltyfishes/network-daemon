use std::ffi::CString;

use libnetwork_daemon::{
    CountryCode, LaggProtocol, LinkOptions, RegDomain, WlanMode,
    error::IfconfigError,
};
use tokio::task;

use crate::ffi;

/// Owns the socket used for FreeBSD's kernel interface ioctls.
struct IoctlSocket {
    fd: libc::c_int,
}

impl IoctlSocket {
    fn open(domain: libc::c_int) -> Result<Self, IfconfigError> {
        let fd = unsafe { libc::socket(domain, libc::SOCK_DGRAM, 0) };
        if fd < 0 {
            return Err(IfconfigError::IoError(
                std::io::Error::last_os_error().into(),
            ));
        }
        Ok(Self { fd })
    }

    fn request<T>(
        &self,
        request: libc::c_ulong,
        data: &mut T,
    ) -> Result<(), IfconfigError> {
        // SAFETY: every caller passes a pointer to the ABI struct required by
        // the request, and both the struct and any nested data remain alive
        // until the synchronous ioctl returns.
        if unsafe { libc::ioctl(self.fd, request, data as *mut T) } < 0 {
            return Err(IfconfigError::IoError(
                std::io::Error::last_os_error().into(),
            ));
        }
        Ok(())
    }
}

impl Drop for IoctlSocket {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.fd);
        }
    }
}

#[derive(Clone)]
pub struct Ifconfig;

impl Ifconfig {
    pub fn new() -> Self {
        Self
    }

    /// Create a FreeBSD logical interface and apply its link parameters.
    ///
    /// FreeBSD owns the clone operation for bridge, lagg, and vlan devices;
    /// the daemon supplies their post-creation parameters through kernel
    /// ioctls.
    pub async fn create_link(
        &self,
        name: &str,
        options: &LinkOptions,
    ) -> Result<(), IfconfigError> {
        match options {
            LinkOptions::Wlan { parent, options } => {
                self.create_wlan(
                    name,
                    parent,
                    options.mode,
                    options.regdomain,
                    options.region,
                )
                .await
            }
            LinkOptions::Bridge { .. }
            | LinkOptions::Lagg { .. }
            | LinkOptions::Vlan { .. } => {
                Self::validate_link(name, options)?;

                let interface_name = name.to_string();
                let options = options.clone();
                task::spawn_blocking(move || {
                    Self::create_interface(&interface_name, None)?;
                    if let Err(error) =
                        Self::configure_link(&interface_name, &options)
                    {
                        // Do not leave a half-configured clone behind when a
                        // required parent/member parameter cannot be applied.
                        let _ = Self::destroy_interface(&interface_name);
                        return Err(error);
                    }
                    Ok(())
                })
                .await??;
                Ok(())
            }
        }
    }

    fn configure_link(
        name: &str,
        options: &LinkOptions,
    ) -> Result<(), IfconfigError> {
        let socket = IoctlSocket::open(libc::AF_INET)?;
        match options {
            LinkOptions::Wlan { .. } => {}
            LinkOptions::Bridge { members } => {
                for member in members {
                    let mut request: ffi::ifbreq =
                        unsafe { std::mem::zeroed() };
                    Self::write_name(&mut request.ifbr_ifsname, member)?;

                    let mut driver: ffi::ifdrv = unsafe { std::mem::zeroed() };
                    Self::write_name(&mut driver.ifd_name, name)?;
                    driver.ifd_cmd = ffi::ND_BRDGADD as _;
                    driver.ifd_len = std::mem::size_of::<ffi::ifbreq>();
                    driver.ifd_data = (&mut request as *mut ffi::ifbreq).cast();
                    socket.request(ffi::ND_SIOCSDRVSPEC as _, &mut driver)?;
                }
            }
            LinkOptions::Lagg { protocol, members } => {
                let mut protocol_request: ffi::lagg_reqall =
                    unsafe { std::mem::zeroed() };
                Self::write_name(&mut protocol_request.ra_ifname, name)?;
                protocol_request.ra_proto = match protocol {
                    LaggProtocol::Failover => ffi::ND_LAGG_PROTO_FAILOVER,
                    LaggProtocol::Lacp => ffi::ND_LAGG_PROTO_LACP,
                    LaggProtocol::Loadbalance => ffi::ND_LAGG_PROTO_LOADBALANCE,
                    LaggProtocol::Roundrobin => ffi::ND_LAGG_PROTO_ROUNDROBIN,
                    LaggProtocol::Broadcast => ffi::ND_LAGG_PROTO_BROADCAST,
                    LaggProtocol::None => ffi::ND_LAGG_PROTO_NONE,
                } as _;
                socket
                    .request(ffi::ND_SIOCSLAGG as _, &mut protocol_request)?;

                for member in members {
                    let mut request: ffi::lagg_reqport =
                        unsafe { std::mem::zeroed() };
                    Self::write_name(&mut request.rp_ifname, name)?;
                    Self::write_name(&mut request.rp_portname, member)?;
                    socket.request(ffi::ND_SIOCSLAGGPORT as _, &mut request)?;
                }
            }
            LinkOptions::Vlan { parent, tag } => {
                let mut vlan: ffi::vlanreq = unsafe { std::mem::zeroed() };
                Self::write_name(&mut vlan.vlr_parent, parent)?;
                vlan.vlr_tag = *tag as _;

                let mut request: libc::ifreq = unsafe { std::mem::zeroed() };
                Self::write_name(&mut request.ifr_name, name)?;
                // `vlan` stays alive and unmodified until the synchronous
                // SIOCSETVLAN request returns.
                request.ifr_ifru.ifru_data =
                    (&mut vlan as *mut ffi::vlanreq).cast();
                socket.request(ffi::ND_SIOCSETVLAN as _, &mut request)?;
            }
        }
        Ok(())
    }

    fn validate_link(
        name: &str,
        options: &LinkOptions,
    ) -> Result<(), IfconfigError> {
        Self::validate_name(name, "interface")?;
        match options {
            LinkOptions::Wlan { parent, .. } => {
                Self::validate_name(parent, "wireless parent")?;
            }
            LinkOptions::Bridge { members } => {
                for member in members {
                    Self::validate_name(member, "bridge member")?;
                }
            }
            LinkOptions::Lagg { members, .. } => {
                for member in members {
                    Self::validate_name(member, "lagg member")?;
                }
            }
            LinkOptions::Vlan { parent, tag } => {
                Self::validate_name(parent, "VLAN parent")?;
                if !(1..=4094).contains(tag) {
                    return Err(IfconfigError::RuntimeError(
                        "VLAN tag must be between 1 and 4094".to_string(),
                    ));
                }
            }
        }
        Ok(())
    }

    pub async fn destroy_link(&self, name: &str) -> Result<(), IfconfigError> {
        Self::validate_name(name, "interface")?;
        let name = name.to_string();
        task::spawn_blocking(move || Self::destroy_interface(&name)).await?
    }

    fn destroy_interface(name: &str) -> Result<(), IfconfigError> {
        let socket = IoctlSocket::open(libc::AF_INET)?;
        let mut request: libc::ifreq = unsafe { std::mem::zeroed() };
        Self::write_name(&mut request.ifr_name, name)?;
        socket.request(ffi::ND_SIOCIFDESTROY as _, &mut request)
    }

    fn validate_name(name: &str, kind: &str) -> Result<(), IfconfigError> {
        if name.is_empty()
            || name.chars().any(|character| {
                character.is_whitespace() || character.is_control()
            })
        {
            return Err(IfconfigError::RuntimeError(format!(
                "invalid {kind} name"
            )));
        }
        Ok(())
    }

    fn write_name(
        destination: &mut [libc::c_char],
        name: &str,
    ) -> Result<(), IfconfigError> {
        let c_name = CString::new(name)?;
        let bytes = c_name.as_bytes_with_nul();
        if bytes.len() > destination.len() {
            return Err(IfconfigError::RuntimeError(format!(
                "interface name is too long: {name}"
            )));
        }
        // SAFETY: `destination` is a writable ABI field with enough space for
        // the complete NUL-terminated name, established above.
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr().cast::<libc::c_char>(),
                destination.as_mut_ptr(),
                bytes.len(),
            );
        }
        Ok(())
    }

    /// Return FreeBSD's detected 802.11 parent devices.
    pub async fn wireless_devices(&self) -> Result<Vec<String>, IfconfigError> {
        task::spawn_blocking(|| {
            let c_name = CString::new("net.wlan.devices")?;
            let mut length = 0usize;
            // SAFETY: `c_name` is a live, NUL-terminated sysctl name. A null
            // output pointer with a valid length pointer is the documented
            // size-query form of `sysctlbyname`.
            let result = unsafe {
                libc::sysctlbyname(
                    c_name.as_ptr(),
                    std::ptr::null_mut(),
                    &mut length,
                    std::ptr::null(),
                    0,
                )
            };
            if result != 0 {
                let error = std::io::Error::last_os_error();
                if matches!(
                    error.raw_os_error(),
                    Some(libc::ENOENT) | Some(libc::EINVAL)
                ) {
                    return Ok(Vec::new());
                }
                return Err(IfconfigError::IoError(error.into()));
            }
            if length == 0 {
                return Ok(Vec::new());
            }

            let mut buffer = vec![0u8; length];
            // SAFETY: `buffer` owns `length` writable bytes and remains alive
            // for the call; `c_name` remains a valid NUL-terminated name.
            let result = unsafe {
                libc::sysctlbyname(
                    c_name.as_ptr(),
                    buffer.as_mut_ptr().cast(),
                    &mut length,
                    std::ptr::null(),
                    0,
                )
            };
            if result != 0 {
                return Err(IfconfigError::IoError(
                    std::io::Error::last_os_error().into(),
                ));
            }

            buffer.truncate(length.min(buffer.len()));
            let mut devices = buffer
                .split(|byte| {
                    *byte == 0 || byte.is_ascii_whitespace() || *byte == b','
                })
                .filter(|device| !device.is_empty())
                .map(|device| String::from_utf8_lossy(device).into_owned())
                .collect::<Vec<_>>();
            devices.sort_unstable();
            devices.dedup();
            Ok(devices)
        })
        .await?
    }

    pub async fn create_wlan(
        &self,
        name: &str,
        parent: &str,
        mode: WlanMode,
        regdomain: RegDomain,
        country: CountryCode,
    ) -> Result<(), IfconfigError> {
        Self::validate_name(name, "interface")?;
        Self::validate_name(parent, "wireless parent")?;
        let name = name.to_string();
        let parent = parent.to_string();

        task::spawn_blocking(move || {
            let mut params: ffi::ieee80211_clone_params =
                unsafe { std::mem::zeroed() };

            Self::write_name(&mut params.icp_parent, &parent)?;

            params.icp_opmode = match mode {
                WlanMode::Sta => ffi::IEEE80211_M_STA as _,
                WlanMode::HostAp => ffi::IEEE80211_M_HOSTAP as _,
                WlanMode::Adhoc => ffi::IEEE80211_M_IBSS as _,
            };

            // Regdomain logic not fully supported via clone params in this FFI
            // version TODO: Implement regdomain setting via
            // post-creation ioctl
            let _ = regdomain;
            let _ = country;

            Self::create_interface(
                &name,
                Some(&mut params as *mut _ as *mut libc::c_void),
            )
        })
        .await??;

        Ok(())
    }

    fn create_interface(
        name: &str,
        data: Option<*mut libc::c_void>,
    ) -> Result<(), IfconfigError> {
        let socket = IoctlSocket::open(libc::AF_INET)?;
        let mut ifr: libc::ifreq = unsafe { std::mem::zeroed() };
        Self::write_name(&mut ifr.ifr_name, name)?;

        if let Some(d) = data {
            ifr.ifr_ifru.ifru_data = d as *mut std::ffi::c_char;
        }
        socket.request(ffi::ND_SIOCIFCREATE2 as _, &mut ifr)?;

        Ok(())
    }

    pub async fn set_link_status(
        &self,
        name: &str,
        up: bool,
    ) -> Result<(), IfconfigError> {
        let name = name.to_string();
        task::spawn_blocking(move || {
            let s = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
            if s < 0 {
                return Err(IfconfigError::IoError(
                    std::io::Error::last_os_error().into(),
                ));
            }

            let mut ifr: libc::ifreq = unsafe { std::mem::zeroed() };
            let c_name = CString::new(name.as_str())?;
            let name_bytes = c_name.as_bytes_with_nul();

            unsafe {
                let len = name_bytes.len().min(ifr.ifr_name.len());
                std::ptr::copy_nonoverlapping(
                    name_bytes.as_ptr() as *const std::ffi::c_char,
                    ifr.ifr_name.as_mut_ptr(),
                    len,
                );

                if libc::ioctl(s, ffi::ND_SIOCGIFFLAGS as _, &mut ifr) < 0 {
                    libc::close(s);
                    return Err(IfconfigError::IoError(
                        std::io::Error::last_os_error().into(),
                    ));
                }

                let fs = &mut ifr.ifr_ifru.ifru_flags;
                if up {
                    fs[0] |= ffi::IFF_UP as i16;
                } else {
                    fs[0] &= !(ffi::IFF_UP as i16);
                }

                if libc::ioctl(s, ffi::ND_SIOCSIFFLAGS as _, &mut ifr) < 0 {
                    libc::close(s);
                    return Err(IfconfigError::IoError(
                        std::io::Error::last_os_error().into(),
                    ));
                }

                libc::close(s);
            }
            Ok(())
        })
        .await?
    }

    /// Query whether SLAAC (kernel RA) is enabled on an interface.
    ///
    /// Used by the SLAAC-wiring phase of the daemon (Phase E); wired up later.
    #[allow(dead_code)]
    pub async fn get_slaac_state(
        &self,
        name: &str,
    ) -> Result<bool, IfconfigError> {
        let name = name.to_string();
        let ret = task::spawn_blocking(move || {
            let s =
                unsafe { libc::socket(libc::AF_INET6, libc::SOCK_DGRAM, 0) };
            if s < 0 {
                return Err(IfconfigError::IoError(
                    std::io::Error::last_os_error().into(),
                ));
            }

            let mut ifreq: ffi::in6_ndireq = unsafe { std::mem::zeroed() };
            let c_name = CString::new(name.as_str())?;
            let name_bytes = c_name.as_bytes_with_nul();

            unsafe {
                let len = name_bytes.len().min(ifreq.ifname.len());
                std::ptr::copy_nonoverlapping(
                    name_bytes.as_ptr() as *const std::ffi::c_char,
                    ifreq.ifname.as_mut_ptr(),
                    len,
                );

                if libc::ioctl(s, ffi::ND_SIOCGIFINFO_IN6 as _, &mut ifreq) < 0
                {
                    libc::close(s);
                    return Err(IfconfigError::IoError(
                        std::io::Error::last_os_error().into(),
                    ));
                }

                let enabled =
                    (ifreq.ndi.flags & ffi::ND6_IFF_ACCEPT_RTADV) != 0;
                libc::close(s);
                Ok(enabled)
            }
        })
        .await??;

        Ok(ret)
    }

    /// Enable/disable SLAAC (kernel RA) on an interface.
    ///
    /// Used by the SLAAC-wiring phase of the daemon (Phase E); wired up later.
    #[allow(dead_code)]
    pub async fn set_slaac_state(
        &self,
        name: &str,
        enabled: bool,
    ) -> Result<(), IfconfigError> {
        let name = name.to_string();
        task::spawn_blocking(move || {
            let s =
                unsafe { libc::socket(libc::AF_INET6, libc::SOCK_DGRAM, 0) };
            if s < 0 {
                return Err(IfconfigError::IoError(
                    std::io::Error::last_os_error().into(),
                ));
            }

            let mut ifreq: ffi::in6_ndireq = unsafe { std::mem::zeroed() };
            let c_name = CString::new(name.as_str())?;
            let name_bytes = c_name.as_bytes_with_nul();

            unsafe {
                let len = name_bytes.len().min(ifreq.ifname.len());
                std::ptr::copy_nonoverlapping(
                    name_bytes.as_ptr() as *const std::ffi::c_char,
                    ifreq.ifname.as_mut_ptr(),
                    len,
                );

                if libc::ioctl(s, ffi::ND_SIOCGIFINFO_IN6 as _, &mut ifreq) < 0
                {
                    libc::close(s);
                    return Err(IfconfigError::IoError(
                        std::io::Error::last_os_error().into(),
                    ));
                }

                if enabled {
                    ifreq.ndi.flags |= ffi::ND6_IFF_ACCEPT_RTADV;
                } else {
                    ifreq.ndi.flags &= !(ffi::ND6_IFF_ACCEPT_RTADV);
                }

                if libc::ioctl(s, ffi::ND_SIOCSIFINFO_IN6 as _, &mut ifreq) < 0
                {
                    libc::close(s);
                    return Err(IfconfigError::IoError(
                        std::io::Error::last_os_error().into(),
                    ));
                }
                libc::close(s);
            }
            Ok(())
        })
        .await?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_bridge_members() {
        Ifconfig::validate_link(
            "bridge0",
            &LinkOptions::Bridge {
                members: vec!["em0".to_string(), "em1".to_string()],
            },
        )
        .unwrap();
    }

    #[test]
    fn validates_lagg_parameters() {
        Ifconfig::validate_link(
            "lagg0",
            &LinkOptions::Lagg {
                protocol: LaggProtocol::Lacp,
                members: vec!["em0".to_string(), "em1".to_string()],
            },
        )
        .unwrap();
    }

    #[test]
    fn validates_vlan_parameters() {
        Ifconfig::validate_link(
            "vlan10",
            &LinkOptions::Vlan {
                parent: "em0".to_string(),
                tag: 10,
            },
        )
        .unwrap();
    }

    #[test]
    fn rejects_invalid_link_parameters_before_ioctl() {
        let invalid_vlan = Ifconfig::validate_link(
            "vlan0",
            &LinkOptions::Vlan {
                parent: "em0".to_string(),
                tag: 4095,
            },
        );
        assert!(invalid_vlan.is_err());

        let invalid_member = Ifconfig::validate_link(
            "bridge0",
            &LinkOptions::Bridge {
                members: vec!["em 0".to_string()],
            },
        );
        assert!(invalid_member.is_err());
    }
}
