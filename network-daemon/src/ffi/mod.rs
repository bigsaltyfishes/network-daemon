//! Bindings for FreeBSD's kernel network-control ABI.
//!
//! The daemon talks to the kernel through sockets, sysctls, netlink and
//! ioctl requests. It does not invoke FreeBSD's interface-management
//! command-line utilities or link their user-space libraries.

#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]
#![allow(clippy::all)]

// Include the generated bindings
include!(concat!(env!("OUT_DIR"), "/freebsd.rs"));

// bindgen cannot consistently materialize FreeBSD's type-sized `_IO*`
// macros. The wrapper exports the request values after the target headers
// have evaluated them, so ioctl numbers and struct layouts stay in sync.
