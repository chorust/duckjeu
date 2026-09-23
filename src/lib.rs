//! DuckJeu v0.1：DuckDB loadable extension，提供 JEV 判断标量函数。
//!
//! 分层（见根规格 §9.1）：`functions/` 与 `config.rs` 是宿主层；`serialize.rs`、
//! `judgment.rs`、`provider/` 不依赖 DuckDB，可在无数据库、无网络条件下独立验证。

#![allow(unsafe_op_in_unsafe_fn)]

pub mod config;
pub mod functions;
pub mod judgment;
pub mod provider;
pub mod serialize;

use std::ffi::CString;
use std::ptr;

use duckdb::ffi::*;

use functions::FunctionKind;

/// 与锁定的目标版本一致；不匹配时 DuckDB 会拒绝加载。
const MINIMUM_DUCKDB_VERSION: &str = "v1.5.5";

unsafe fn report_error(
    access: *const duckdb_extension_access,
    info: duckdb_extension_info,
    message: &str,
) {
    if let Some(set_error) = (*access).set_error {
        if let Ok(cstr) = CString::new(message.replace('\0', " ")) {
            set_error(info, cstr.as_ptr());
        }
    }
}

/// 扩展入口：注册 session 配置项与三个标量函数。
///
/// # Safety
/// 由 DuckDB 在 `LOAD` 时调用；`info`/`access` 由宿主保证有效。
#[no_mangle]
pub unsafe extern "C" fn duckjeu_init_c_api(
    info: duckdb_extension_info,
    access: *const duckdb_extension_access,
) -> bool {
    match duckdb_rs_extension_api_init(info, access, MINIMUM_DUCKDB_VERSION) {
        Ok(true) => {}
        Ok(false) => {
            report_error(access, info, "duckjeu requires DuckDB v1.5.5");
            return false;
        }
        Err(message) => {
            report_error(access, info, message);
            return false;
        }
    }

    let get_database = match (*access).get_database {
        Some(get_database) => get_database,
        None => {
            report_error(access, info, "duckjeu: get_database is unavailable");
            return false;
        }
    };
    let database = *get_database(info);
    if database.is_null() {
        return false;
    }

    let mut connection: duckdb_connection = ptr::null_mut();
    if duckdb_connect(database, &mut connection) != DuckDBSuccess {
        report_error(access, info, "duckjeu: could not open a connection");
        return false;
    }

    let mut ok = config::register_config_options(connection);
    if !ok {
        report_error(
            access,
            info,
            "duckjeu: could not register session config options",
        );
    }
    ok &= functions::prob::register(connection);
    ok &= functions::bool_fn::register(connection);
    ok &= functions::choice::register(connection);
    if !ok {
        report_error(
            access,
            info,
            "duckjeu: could not register one or more scalar functions",
        );
    }

    duckdb_disconnect(&mut connection);
    ok
}

/// 供集成测试与文档引用：三个函数的名称与签名。
pub const FUNCTIONS: [(&str, FunctionKind); 3] = [
    ("jev_prob", FunctionKind::Prob),
    ("jev_bool", FunctionKind::Bool),
    ("jev_choice", FunctionKind::Choice),
];
