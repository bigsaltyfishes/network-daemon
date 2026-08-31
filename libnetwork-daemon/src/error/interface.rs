//! Network interface error types

use std::ffi::NulError;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::error::{IoError, netlink::NetlinkQueryError};

#[derive(Error, Debug, Serialize, Deserialize, JsonSchema)]
pub enum IfconfigError {
    #[error("IO Error: {0}")]
    IoError(#[from] IoError),
    #[error("Nul Error: {0}")]
    NulError(String),
    #[error("Runtime Error: {0}")]
    RuntimeError(String),
}

impl From<NulError> for IfconfigError {
    fn from(err: NulError) -> Self {
        IfconfigError::NulError(err.to_string())
    }
}

impl From<tokio::task::JoinError> for IfconfigError {
    fn from(err: tokio::task::JoinError) -> Self {
        IfconfigError::RuntimeError(err.to_string())
    }
}

/// Errors that can occur when managing network interfaces
#[derive(Error, Debug, Serialize, Deserialize, JsonSchema)]
pub enum InterfaceError {
    /// Failed to query interface info
    #[error("failed to query interface info: {0}")]
    NetlinkQueryError(#[from] NetlinkQueryError),

    /// Failed to get media info for interface
    #[error("failed to get media info for interface: {name}")]
    GetMediaFailed { name: String },

    /// Failed to get interface groups
    #[error("failed to get interface groups")]
    GetGroupsFailed,

    /// Interface not found
    #[error("interface not found: {name}")]
    InterfaceNotFound { name: String },

    /// Failed to get parent interface
    #[error("failed to get parent interface")]
    GetParentFailed,

    /// Invalid interface name
    // #[error("invalid interface name: {0}")]
    // InvalidName(String),

    /// Interface not found
    #[error("interface not found: {0}")]
    NotFound(String),

    /// sysctl operation failed
    #[error("sysctl operation failed")]
    SysctlFailed(#[source] IoError),

    /// Netlink listener error
    #[error("netlink listener error: {0}")]
    NetlinkListenerError(#[source] IoError),

    /// Interface Configuration error
    #[error("interface configuration error: {0}")]
    IfconfigError(#[from] IfconfigError),

    #[error("{0}")]
    Other(String),
}

impl<T> From<T> for InterfaceError
where
    T: AsRef<str>,
{
    fn from(err: T) -> Self {
        InterfaceError::Other(err.as_ref().to_string())
    }
}
