//! Compatibility exports for the host-independent core executor.
pub use duckjeu_judgment_core::executor::*;

#[cfg(test)]
include!("executor_tests.rs");
