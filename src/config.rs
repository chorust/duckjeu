//! 连接级配置与凭据读取（宿主层）。
//!
//! 配置项在 `duckdb_register_config_option` 注册，scalar init 回调按**每次执行**读取
//! 并冻结为 `ProviderContext`；连接之间互不共享。凭据只从环境变量读取，没有 SQL 途径。

use std::ffi::{c_void, CStr, CString};
use std::ptr;

use duckdb::ffi::*;

use crate::provider::{
    ProviderContext, ProviderKind, DEFAULT_API_URL, DEFAULT_MAX_RESPONSE_BYTES, DEFAULT_MODEL,
    DEFAULT_TIMEOUT_MS,
};

pub const OPTION_PROVIDER: &str = "duckjeu_provider";
pub const OPTION_API_URL: &str = "duckjeu_api_url";
pub const OPTION_MODEL: &str = "duckjeu_model";
pub const OPTION_TIMEOUT_MS: &str = "duckjeu_timeout_ms";
pub const OPTION_MAX_RESPONSE_BYTES: &str = "duckjeu_max_response_bytes";

/// 凭据环境变量；`DUCKJEU_API_KEY` 优先于 `TYPESAFE_API_KEY`。
pub const CREDENTIAL_ENV: &str = "TYPESAFE_API_KEY";
pub const CREDENTIAL_ENV_OVERRIDE: &str = "DUCKJEU_API_KEY";

unsafe fn register_varchar_option(con: duckdb_connection, name: &str, default: &str) -> bool {
    let mut option = duckdb_create_config_option();
    let cname = CString::new(name).expect("option name has no NUL");
    duckdb_config_option_set_name(option, cname.as_ptr());
    let mut ty = duckdb_create_logical_type(DUCKDB_TYPE_DUCKDB_TYPE_VARCHAR);
    duckdb_config_option_set_type(option, ty);
    let mut value = duckdb_create_varchar_length(default.as_ptr().cast(), default.len() as idx_t);
    duckdb_config_option_set_default_value(option, value);
    duckdb_config_option_set_default_scope(
        option,
        duckdb_config_option_scope_DUCKDB_CONFIG_OPTION_SCOPE_SESSION,
    );
    let rc = duckdb_register_config_option(con, option);
    duckdb_destroy_config_option(&mut option);
    duckdb_destroy_value(&mut value);
    duckdb_destroy_logical_type(&mut ty);
    rc == DuckDBSuccess
}

unsafe fn register_bigint_option(con: duckdb_connection, name: &str, default: i64) -> bool {
    let mut option = duckdb_create_config_option();
    let cname = CString::new(name).expect("option name has no NUL");
    duckdb_config_option_set_name(option, cname.as_ptr());
    let mut ty = duckdb_create_logical_type(DUCKDB_TYPE_DUCKDB_TYPE_BIGINT);
    duckdb_config_option_set_type(option, ty);
    let mut value = duckdb_create_int64(default);
    duckdb_config_option_set_default_value(option, value);
    duckdb_config_option_set_default_scope(
        option,
        duckdb_config_option_scope_DUCKDB_CONFIG_OPTION_SCOPE_SESSION,
    );
    let rc = duckdb_register_config_option(con, option);
    duckdb_destroy_config_option(&mut option);
    duckdb_destroy_value(&mut value);
    duckdb_destroy_logical_type(&mut ty);
    rc == DuckDBSuccess
}

/// 注册全部 `duckjeu_*` session 配置项。
///
/// # Safety
/// `con` 必须是当前扩展初始化阶段的有效 DuckDB 连接。
pub unsafe fn register_config_options(con: duckdb_connection) -> bool {
    let mut ok = true;
    ok &= register_varchar_option(con, OPTION_PROVIDER, ProviderKind::Mock.as_str());
    ok &= register_varchar_option(con, OPTION_API_URL, DEFAULT_API_URL);
    ok &= register_varchar_option(con, OPTION_MODEL, DEFAULT_MODEL);
    ok &= register_bigint_option(con, OPTION_TIMEOUT_MS, DEFAULT_TIMEOUT_MS as i64);
    ok &= register_bigint_option(
        con,
        OPTION_MAX_RESPONSE_BYTES,
        DEFAULT_MAX_RESPONSE_BYTES as i64,
    );
    ok
}

unsafe fn read_option_string(ctx: duckdb_client_context, name: &str) -> Option<String> {
    let cname = CString::new(name).expect("option name has no NUL");
    let mut scope = 0;
    let value = duckdb_client_context_get_config_option(ctx, cname.as_ptr(), &mut scope);
    if value.is_null() {
        return None;
    }
    let ptr = duckdb_get_varchar(value);
    let out = if ptr.is_null() {
        None
    } else {
        Some(CStr::from_ptr(ptr).to_string_lossy().into_owned())
    };
    if !ptr.is_null() {
        duckdb_free(ptr.cast::<c_void>());
    }
    let mut value = value;
    duckdb_destroy_value(&mut value);
    out
}

unsafe fn read_option_int64(ctx: duckdb_client_context, name: &str) -> Option<i64> {
    let cname = CString::new(name).expect("option name has no NUL");
    let mut scope = 0;
    let value = duckdb_client_context_get_config_option(ctx, cname.as_ptr(), &mut scope);
    if value.is_null() {
        return None;
    }
    let out = duckdb_get_int64(value);
    let mut value = value;
    duckdb_destroy_value(&mut value);
    Some(out)
}

/// 从环境变量读取凭据；不存在或为空白时返回 None。
pub fn credential_from_env() -> Option<String> {
    for key in [CREDENTIAL_ENV_OVERRIDE, CREDENTIAL_ENV] {
        if let Ok(value) = std::env::var(key) {
            if !value.trim().is_empty() {
                return Some(value);
            }
        }
    }
    None
}

/// 在 scalar init 回调中读取当前执行的配置快照。
///
/// # Safety
/// `init_info` 必须来自正在执行的标量函数 init 回调。
pub unsafe fn read_provider_context(
    init_info: duckdb_init_info,
) -> Result<ProviderContext, String> {
    let mut ctx: duckdb_client_context = ptr::null_mut();
    duckdb_scalar_function_init_get_client_context(init_info, &mut ctx);
    if ctx.is_null() {
        return Err("internal error: client context is unavailable".to_string());
    }

    let provider_raw = read_option_string(ctx, OPTION_PROVIDER);
    let api_url = read_option_string(ctx, OPTION_API_URL);
    let model = read_option_string(ctx, OPTION_MODEL);
    let timeout_ms = read_option_int64(ctx, OPTION_TIMEOUT_MS);
    let max_response_bytes = read_option_int64(ctx, OPTION_MAX_RESPONSE_BYTES);
    duckdb_destroy_client_context(&mut ctx);

    let provider_raw = provider_raw.unwrap_or_else(|| ProviderKind::Mock.as_str().to_string());
    let provider = ProviderKind::parse(&provider_raw).map_err(|e| e.message().to_string())?;
    let timeout_ms = match timeout_ms {
        Some(v) if v > 0 => v as u64,
        Some(_) => return Err("duckjeu_timeout_ms must be positive".to_string()),
        None => DEFAULT_TIMEOUT_MS,
    };
    let max_response_bytes = match max_response_bytes {
        Some(v) if v > 0 => v as u64,
        Some(_) => return Err("duckjeu_max_response_bytes must be positive".to_string()),
        None => DEFAULT_MAX_RESPONSE_BYTES,
    };

    Ok(ProviderContext {
        provider,
        api_url: api_url.unwrap_or_else(|| DEFAULT_API_URL.to_string()),
        model: model.unwrap_or_else(|| DEFAULT_MODEL.to_string()),
        timeout_ms,
        max_response_bytes,
        credential: credential_from_env(),
    })
}
