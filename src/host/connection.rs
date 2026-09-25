use duckdb::ffi::*;

use super::{acquire_connection_runtime, SharedConnectionRuntime};

struct OwnedClientContext(duckdb_client_context);

impl Drop for OwnedClientContext {
    fn drop(&mut self) {
        unsafe { duckdb_destroy_client_context(&mut self.0) };
    }
}

/// Gets the real ClientContext associated with a scalar function initialization callback.
///
/// DuckDB's extension initialization connection is deliberately not used here: this runs only
/// when DuckDB initializes a user query's scalar function.
pub(crate) unsafe fn runtime_for_scalar_init(
    info: duckdb_init_info,
) -> Option<SharedConnectionRuntime> {
    let mut context: duckdb_client_context = std::ptr::null_mut();
    duckdb_scalar_function_init_get_client_context(info, &mut context);
    if context.is_null() {
        return None;
    }
    let _owned_context = OwnedClientContext(context);
    acquire_connection_runtime(context)
}
