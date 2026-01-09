use netlink_packet_core::DecodeError;
use schemars::JsonSchema;
use serde::Serialize;
use thiserror::Error;

use crate::error::IoError;

#[derive(Debug, Error, Serialize, JsonSchema)]
pub enum NetlinkQueryError {
    #[error("IO Error: {0}")]
    IoError(#[from] IoError),
    #[error("Decode Error: {0}")]
    DecodeError(#[from] NetlinkDecodeError),
    #[error("Operation failed: code {0}")]
    NetlinkError(i32),
}

#[derive(Debug, Error)]
#[error("I/O error: {0}")]
pub struct NetlinkDecodeError(#[from] DecodeError);

impl Serialize for NetlinkDecodeError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl JsonSchema for NetlinkDecodeError {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("NetlinkDecodeError")
    }

    fn json_schema(
        generator: &mut schemars::SchemaGenerator,
    ) -> schemars::Schema {
        generator.subschema_for::<String>()
    }
}
