use std::{
    ffi::{CString, NulError},
    str::FromStr,
};

use libnetwork_daemon::{
    CountryCode, RegDomain, WlanMode, error::IfconfigError,
};
use tokio::task;

use crate::ffi;

#[derive(Clone)]
pub struct Ifconfig;

impl Ifconfig {
    pub fn new() -> Self {
        Self
    }

    pub async fn create_bridge(&self, name: &str) -> Result<(), IfconfigError> {
        let name = name.to_string();
        task::spawn_blocking(move || Self::create_interface(&name, None))
            .await??;
        Ok(())
    }

    pub async fn create_wlan(
        &self,
        name: &str,
        parent: &str,
        mode: WlanMode,
        regdomain: RegDomain,
        country: CountryCode,
    ) -> Result<(), IfconfigError> {
        let name = name.to_string();
        let parent = parent.to_string();

        task::spawn_blocking(move || {
            let mut params: ffi::ieee80211_clone_params =
                unsafe { std::mem::zeroed() };

            // Helper to copy strings to fixed size arrays
            let set_parent = |dst: &mut [std::ffi::c_char],
                              val: &str|
             -> Result<(), NulError> {
                let c_str = CString::from_str(val)?;
                let bytes = c_str.as_bytes_with_nul();
                let len = bytes.len().min(dst.len());
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        bytes.as_ptr() as *const std::ffi::c_char,
                        dst.as_mut_ptr(),
                        len,
                    );
                }
                Ok(())
            };

            set_parent(&mut params.icp_parent, &parent)?;

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
        let s = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
        if s < 0 {
            return Err(IfconfigError::IoError(
                std::io::Error::last_os_error().into(),
            ));
        }

        let mut ifr: libc::ifreq = unsafe { std::mem::zeroed() };

        let c_name = CString::from_str(name)?;
        let name_bytes = c_name.as_bytes_with_nul();
        let name_len =
            name_bytes.len().min(std::mem::size_of_val(&ifr.ifr_name));

        unsafe {
            std::ptr::copy_nonoverlapping(
                name_bytes.as_ptr() as *const std::ffi::c_char,
                ifr.ifr_name.as_mut_ptr(),
                name_len,
            );

            if let Some(d) = data {
                ifr.ifr_ifru.ifru_data = d as *mut std::ffi::c_char;
            }

            // SIOCIFCREATE2 value comes from ffi, cast to proper type
            let ret = libc::ioctl(s, ffi::SIOCIFCREATE2 as _, &mut ifr);
            libc::close(s);

            if ret < 0 {
                // Check errno
                return Err(IfconfigError::IoError(
                    std::io::Error::last_os_error().into(),
                ));
            }
        }

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
            let c_name = CString::from_str(&*name)?;
            let name_bytes = c_name.as_bytes_with_nul();

            unsafe {
                let len = name_bytes.len().min(ifr.ifr_name.len());
                std::ptr::copy_nonoverlapping(
                    name_bytes.as_ptr() as *const std::ffi::c_char,
                    ifr.ifr_name.as_mut_ptr(),
                    len,
                );

                if libc::ioctl(s, ffi::SIOCGIFFLAGS as _, &mut ifr) < 0 {
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

                if libc::ioctl(s, ffi::SIOCSIFFLAGS as _, &mut ifr) < 0 {
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
            let c_name = CString::from_str(&*name)?;
            let name_bytes = c_name.as_bytes_with_nul();

            unsafe {
                let len = name_bytes.len().min(ifreq.ifname.len());
                std::ptr::copy_nonoverlapping(
                    name_bytes.as_ptr() as *const std::ffi::c_char,
                    ifreq.ifname.as_mut_ptr(),
                    len,
                );

                if libc::ioctl(s, ffi::SIOCGIFINFO_IN6 as _, &mut ifreq) < 0 {
                    libc::close(s);
                    return Err(IfconfigError::IoError(
                        std::io::Error::last_os_error().into(),
                    ));
                }

                let enabled =
                    (ifreq.ndi.flags & ffi::ND6_IFF_ACCEPT_RTADV as u32) != 0;
                libc::close(s);
                Ok(enabled)
            }
        })
        .await??;

        Ok(ret)
    }

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
            let c_name = CString::from_str(&*name)?;
            let name_bytes = c_name.as_bytes_with_nul();

            unsafe {
                let len = name_bytes.len().min(ifreq.ifname.len());
                std::ptr::copy_nonoverlapping(
                    name_bytes.as_ptr() as *const std::ffi::c_char,
                    ifreq.ifname.as_mut_ptr(),
                    len,
                );

                if libc::ioctl(s, ffi::SIOCGIFINFO_IN6 as _, &mut ifreq) < 0 {
                    libc::close(s);
                    return Err(IfconfigError::IoError(
                        std::io::Error::last_os_error().into(),
                    ));
                }

                if enabled {
                    ifreq.ndi.flags |= ffi::ND6_IFF_ACCEPT_RTADV as u32;
                } else {
                    ifreq.ndi.flags &= !(ffi::ND6_IFF_ACCEPT_RTADV as u32);
                }

                if libc::ioctl(s, ffi::SIOCSIFINFO_IN6 as _, &mut ifreq) < 0 {
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
