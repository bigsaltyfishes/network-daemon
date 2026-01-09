use schemars::JsonSchema;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("I/O error: {0}")]
pub struct IoError(#[from] std::io::Error);

impl Serialize for IoError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl JsonSchema for IoError {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("IoError")
    }

    fn json_schema(
        generator: &mut schemars::SchemaGenerator,
    ) -> schemars::Schema {
        generator.subschema_for::<String>()
    }
}
