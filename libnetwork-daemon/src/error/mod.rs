//! Error types for wutil-rs
//!
//! This module defines module-level error types using thiserror.
//! Top-level code uses anyhow for error aggregation.

mod daemon;
mod interface;
mod io;
mod netlink;
mod wifi;
mod wpa;

pub use daemon::NetworkDaemonError;
pub use interface::{IfconfigError, InterfaceError};
pub use io::*;
pub use netlink::*;
pub use wifi::WifiError;
pub use wpa::{WpaCtrlError, WpaSocketError};
