//! `jev_bool(state, criterion) -> BOOLEAN`（同一 noul 概率路径，p >= 0.5 含边界）

use duckdb::ffi::*;

use super::{run, FunctionKind};

unsafe extern "C" fn eval(
    info: duckdb_function_info,
    input: duckdb_data_chunk,
    output: duckdb_vector,
) {
    run(FunctionKind::Bool, info, input, output);
}

/// # Safety
/// `con` 必须是有效的 DuckDB 连接。
pub unsafe fn register(con: duckdb_connection) -> bool {
    super::register_function(con, "jev_bool", FunctionKind::Bool, eval)
}
