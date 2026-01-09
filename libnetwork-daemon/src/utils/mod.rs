//! Utility functions
//!
//! This module provides various utility functions for string processing,
//! SSID encoding/decoding, and display width calculations.

mod broadcast;
mod ssid;

pub use broadcast::*;
pub use ssid::*;

/// Ensure a value is present or successful, otherwise panic with an invariant
/// violation message.
///
/// This macro is intended for use with trusted data sources where an error or
/// none value indicates a bug in the program logic rather than a runtime
/// failure.
#[macro_export]
macro_rules! ensure {
    ($e:expr) => {
        $e.expect("Invariant violation: impossible exception")
    };
    ($e:expr, $msg:literal) => {
        $e.expect(concat!("Invariant violation: ", $msg))
    };
    ($e:expr, $fmt:literal, $($arg:tt)*) => {
        $e.expect(&format!(concat!("Invariant violation: ", $fmt), $($arg)*))
    };
}

#[macro_export]
macro_rules! ignore {
    ($e:expr) => {
        let _ = $e;
    };
}
