//! `duckjeu_cache_clear() -> BIGINT`, scoped to the current DuckDB connection.

use std::ffi::{c_void, CString};
use std::sync::Mutex;
use std::time::Instant;

use duckdb::ffi::*;

use crate::cache::CachePolicy;
use crate::host::SharedConnectionRuntime;

struct ClearState {
    connection: Option<SharedConnectionRuntime>,
    cache_policy: Option<CachePolicy>,
    result: Mutex<Option<i64>>,
}

unsafe extern "C" fn drop_state(state: *mut c_void) {
    if !state.is_null() {
        drop(Box::from_raw(state.cast::<ClearState>()));
    }
}

unsafe extern "C" fn init(info: duckdb_init_info) {
    let cache_policy = crate::config::read_runtime_config(info)
        .ok()
        .filter(|config| config.cache_enabled)
        .map(|config| CachePolicy {
            max_entries: config.cache_max_entries,
            max_bytes: config.cache_max_bytes,
            ttl: std::time::Duration::from_millis(config.cache_ttl_ms as u64),
        });
    let state = ClearState {
        connection: crate::host::connection::runtime_for_scalar_init(info),
        cache_policy,
        result: Mutex::new(None),
    };
    duckdb_scalar_function_init_set_state(
        info,
        Box::into_raw(Box::new(state)).cast(),
        Some(drop_state),
    );
}

unsafe fn set_error(info: duckdb_function_info, message: &str) {
    if let Ok(message) = CString::new(message.replace('\0', " ")) {
        duckdb_scalar_function_set_error(info, message.as_ptr());
    }
}

unsafe extern "C" fn eval(
    info: duckdb_function_info,
    input: duckdb_data_chunk,
    output: duckdb_vector,
) {
    let state = duckdb_scalar_function_get_state(info).cast::<ClearState>();
    if state.is_null() {
        set_error(info, "internal error: cache clear state is missing");
        return;
    }
    let Some(connection) = &(*state).connection else {
        set_error(
            info,
            "duckjeu_cache_clear requires the DuckDB v1.5.5 connection bridge",
        );
        return;
    };

    let mut cached_result = (*state)
        .result
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if cached_result.is_none() {
        let mut runtime = connection
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let cache = runtime.cache_mut();
        if let Some(policy) = (*state).cache_policy.as_ref() {
            cache.reconcile_policy(Instant::now(), policy);
        }
        let removed = cache.clear();
        *cached_result = Some(i64::try_from(removed).unwrap_or(i64::MAX));
    }
    let removed = cached_result.unwrap_or_default();
    let data = duckdb_vector_get_data(output).cast::<i64>();
    if data.is_null() {
        set_error(
            info,
            "internal error: cache clear output vector is unavailable",
        );
        return;
    }
    for row in 0..duckdb_data_chunk_get_size(input) as usize {
        *data.add(row) = removed;
    }
}

/// # Safety
/// `con` must be a valid DuckDB connection.
pub unsafe fn register(con: duckdb_connection) -> bool {
    let mut function = duckdb_create_scalar_function();
    let name = match CString::new("duckjeu_cache_clear") {
        Ok(value) => value,
        Err(_) => return false,
    };
    duckdb_scalar_function_set_name(function, name.as_ptr());
    let mut return_type = duckdb_create_logical_type(DUCKDB_TYPE_DUCKDB_TYPE_BIGINT);
    duckdb_scalar_function_set_return_type(function, return_type);
    duckdb_scalar_function_set_volatile(function);
    duckdb_scalar_function_set_special_handling(function);
    duckdb_scalar_function_set_init(function, Some(init));
    duckdb_scalar_function_set_function(function, Some(eval));

    let status = duckdb_register_scalar_function(con, function);
    duckdb_destroy_scalar_function(&mut function);
    duckdb_destroy_logical_type(&mut return_type);
    status == DuckDBSuccess
}
