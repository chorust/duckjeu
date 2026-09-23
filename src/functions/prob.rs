//! `jev_prob(state, criterion) -> DOUBLE`

use duckdb::ffi::*;

use super::{run, FunctionKind};

unsafe extern "C" fn eval(
    info: duckdb_function_info,
    input: duckdb_data_chunk,
    output: duckdb_vector,
) {
    run(FunctionKind::Prob, info, input, output);
}

/// # Safety
/// `con` 必须是有效的 DuckDB 连接。
pub unsafe fn register(con: duckdb_connection) -> bool {
    super::register_function(con, "jev_prob", FunctionKind::Prob, eval)
}
