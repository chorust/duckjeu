//! `duckjeu_last_profile() -> VARCHAR`, reading the latest completed judgment query.

use std::ffi::{c_char, CString};

use duckdb::ffi::*;

use crate::host::{QueryOutcome, QuerySnapshot, SharedConnectionRuntime};
use duckjeu_judgment_core::metrics::{render_profile, ProfileReport};

struct ProfileState {
    connection: Option<SharedConnectionRuntime>,
}

unsafe extern "C" fn drop_state(state: *mut std::ffi::c_void) {
    if !state.is_null() {
        drop(Box::from_raw(state.cast::<ProfileState>()));
    }
}

unsafe extern "C" fn init(info: duckdb_init_info) {
    let state = ProfileState {
        connection: crate::host::connection::runtime_for_scalar_init(info),
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

fn status(outcome: QueryOutcome) -> &'static str {
    match outcome {
        QueryOutcome::Succeeded => "completed",
        QueryOutcome::Failed => "failed",
        QueryOutcome::Cancelled => "cancelled",
    }
}

fn serialize(snapshot: &QuerySnapshot) -> Result<String, String> {
    render_profile(ProfileReport {
        status: status(snapshot.outcome),
        provider: snapshot.provider.as_deref(),
        requested_model: snapshot.requested_model.as_deref(),
        counts: &snapshot.counts,
        elapsed: snapshot.elapsed,
        detailed_timing: snapshot.detailed_timing,
    })
    .map_err(|_| "execution profile could not be serialized".to_string())
}

unsafe extern "C" fn eval(
    info: duckdb_function_info,
    input: duckdb_data_chunk,
    output: duckdb_vector,
) {
    let state = duckdb_scalar_function_get_state(info).cast::<ProfileState>();
    if state.is_null() {
        set_error(info, "internal error: profile state is missing");
        return;
    }
    let snapshot = (*state).connection.as_ref().and_then(|connection| {
        connection
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .last_query()
            .cloned()
    });
    let rendered = match snapshot.as_ref().map(serialize).transpose() {
        Ok(value) => value,
        Err(error) => {
            set_error(info, &error);
            return;
        }
    };
    let output_rows = duckdb_data_chunk_get_size(input) as usize;
    for row in 0..output_rows {
        let Some(profile) = rendered.as_deref() else {
            duckdb_vector_ensure_validity_writable(output);
            let validity = duckdb_vector_get_validity(output);
            if !validity.is_null() {
                duckdb_validity_set_row_invalid(validity, row as idx_t);
            }
            continue;
        };
        duckdb_vector_assign_string_element_len(
            output,
            row as idx_t,
            profile.as_ptr().cast::<c_char>(),
            profile.len() as idx_t,
        );
    }
}

/// # Safety
/// `con` must be a valid DuckDB connection.
pub unsafe fn register(con: duckdb_connection) -> bool {
    let mut function = duckdb_create_scalar_function();
    let name = match CString::new("duckjeu_last_profile") {
        Ok(value) => value,
        Err(_) => return false,
    };
    duckdb_scalar_function_set_name(function, name.as_ptr());
    let mut return_type = duckdb_create_logical_type(DUCKDB_TYPE_DUCKDB_TYPE_VARCHAR);
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
