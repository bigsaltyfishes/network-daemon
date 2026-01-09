use std::{
    io::{Read, Write},
    os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd},
};

use async_io::IoSafe;

use crate::ffi;

pub struct NetlinkSocket(OwnedFd);

impl NetlinkSocket {
    pub fn new() -> Result<Self, std::io::Error> {
        unsafe {
            let fd = libc::socket(
                ffi::AF_NETLINK as _,
                libc::SOCK_RAW,
                ffi::NETLINK_ROUTE as _,
            );
            if fd < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(NetlinkSocket(OwnedFd::from_raw_fd(fd)))
        }
    }

    pub fn bind(
        &mut self,
        addr: &ffi::sockaddr_nl,
    ) -> Result<i32, std::io::Error> {
        unsafe {
            let ret = libc::bind(
                self.as_raw_fd(),
                addr as *const ffi::sockaddr_nl as *const libc::sockaddr,
                std::mem::size_of::<ffi::sockaddr_nl>() as _,
            );
            if ret < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(ret)
        }
    }

    pub fn set_filter_enabled(
        &mut self,
        enabled: bool,
    ) -> Result<(), std::io::Error> {
        let optval: libc::c_int = if enabled { 1 } else { 0 };
        unsafe {
            let ret = libc::setsockopt(
                self.as_raw_fd(),
                ffi::SOL_NETLINK as _,
                ffi::NETLINK_GET_STRICT_CHK as _,
                &optval as *const libc::c_int as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as _,
            );
            if ret < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        }
    }
}

impl AsFd for NetlinkSocket {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl AsRawFd for NetlinkSocket {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

impl Read for NetlinkSocket {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, std::io::Error> {
        unsafe {
            let ret = libc::read(
                self.as_raw_fd(),
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
            );
            if ret < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(ret as usize)
        }
    }
}

impl Write for NetlinkSocket {
    fn write(&mut self, buf: &[u8]) -> Result<usize, std::io::Error> {
        unsafe {
            let ret = libc::write(
                self.as_raw_fd(),
                buf.as_ptr() as *const libc::c_void,
                buf.len(),
            );
            if ret < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(ret as usize)
        }
    }

    fn flush(&mut self) -> Result<(), std::io::Error> {
        Ok(())
    }
}

unsafe impl IoSafe for NetlinkSocket {}
