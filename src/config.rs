//! 连接级配置与凭据读取（宿主层）。
//!
//! 配置项在 `duckdb_register_config_option` 注册，scalar init 回调按**每次执行**读取
//! 并冻结为 `ProviderContext`；连接之间互不共享。凭据只从环境变量读取，没有 SQL 途径。

use std::ffi::{c_void, CStr, CString};
use std::ptr;

use duckdb::ffi::*;

use crate::provider::{
    ProviderContext, ProviderKind, DEFAULT_API_URL, DEFAULT_LOCAL_JEV_API_URL,
    DEFAULT_LOCAL_JEV_MODEL, DEFAULT_MAX_RESPONSE_BYTES, DEFAULT_MODEL, DEFAULT_TIMEOUT_MS,
};

pub const OPTION_PROVIDER: &str = "duckjeu_provider";
pub const OPTION_API_URL: &str = "duckjeu_api_url";
pub const OPTION_LOCAL_JEV_API_URL: &str = "duckjeu_local_jev_api_url";
pub const OPTION_MODEL: &str = "duckjeu_model";
pub const OPTION_TIMEOUT_MS: &str = "duckjeu_timeout_ms";
pub const OPTION_MAX_RESPONSE_BYTES: &str = "duckjeu_max_response_bytes";
pub const OPTION_EXECUTION_MODE: &str = "duckjeu_execution_mode";
pub const OPTION_BATCH_ENABLED: &str = "duckjeu_batch_enabled";
pub const OPTION_BATCH_MAX_JUDGMENTS: &str = "duckjeu_batch_max_judgments";
pub const OPTION_MAX_REQUEST_BYTES: &str = "duckjeu_max_request_bytes";
pub const OPTION_MAX_INFLIGHT: &str = "duckjeu_max_inflight";
pub const OPTION_CACHE_ENABLED: &str = "duckjeu_cache_enabled";
pub const OPTION_CACHE_MAX_ENTRIES: &str = "duckjeu_cache_max_entries";
pub const OPTION_CACHE_MAX_BYTES: &str = "duckjeu_cache_max_bytes";
pub const OPTION_CACHE_TTL_MS: &str = "duckjeu_cache_ttl_ms";
pub const OPTION_PROFILE_ENABLED: &str = "duckjeu_profile_enabled";

/// 凭据环境变量；`DUCKJEU_API_KEY` 优先于 `TYPESAFE_API_KEY`。
pub const CREDENTIAL_ENV: &str = "TYPESAFE_API_KEY";
pub const CREDENTIAL_ENV_OVERRIDE: &str = "DUCKJEU_API_KEY";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    Row,
    Optimized,
}

impl ExecutionMode {
    fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "row" => Ok(Self::Row),
            "optimized" => Ok(Self::Optimized),
            _ => Err("duckjeu_execution_mode must be 'row' or 'optimized'".to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfig {
    pub mode: ExecutionMode,
    pub batch_enabled: bool,
    pub batch_max_judgments: usize,
    pub max_request_bytes: usize,
    pub max_inflight: usize,
    pub cache_enabled: bool,
    pub cache_max_entries: usize,
    pub cache_max_bytes: usize,
    pub cache_ttl_ms: usize,
    pub profile_enabled: bool,
}

impl RuntimeConfig {
    #[allow(clippy::too_many_arguments)] // Mirrors the independent DuckDB setting values.
    fn from_values(
        mode: &str,
        batch_enabled: bool,
        batch_max_judgments: i64,
        max_request_bytes: i64,
        max_inflight: i64,
        cache_enabled: bool,
        cache_max_entries: i64,
        cache_max_bytes: i64,
        cache_ttl_ms: i64,
    ) -> Result<Self, String> {
        let mode = ExecutionMode::parse(mode)?;
        if mode == ExecutionMode::Row && batch_enabled {
            return Err(
                "duckjeu_batch_enabled requires duckjeu_execution_mode='optimized'".to_string(),
            );
        }
        if mode == ExecutionMode::Row && cache_enabled {
            return Err(
                "duckjeu_cache_enabled requires duckjeu_execution_mode='optimized'".to_string(),
            );
        }
        Ok(Self {
            mode,
            batch_enabled,
            batch_max_judgments: positive_limit(OPTION_BATCH_MAX_JUDGMENTS, batch_max_judgments)?,
            max_request_bytes: positive_limit(OPTION_MAX_REQUEST_BYTES, max_request_bytes)?,
            max_inflight: positive_limit(OPTION_MAX_INFLIGHT, max_inflight)?,
            cache_enabled,
            cache_max_entries: positive_limit(OPTION_CACHE_MAX_ENTRIES, cache_max_entries)?,
            cache_max_bytes: positive_limit(OPTION_CACHE_MAX_BYTES, cache_max_bytes)?,
            cache_ttl_ms: positive_limit(OPTION_CACHE_TTL_MS, cache_ttl_ms)?,
            profile_enabled: false,
        })
    }
}

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

unsafe fn register_boolean_option(con: duckdb_connection, name: &str, default: bool) -> bool {
    let mut option = duckdb_create_config_option();
    let cname = CString::new(name).expect("option name has no NUL");
    duckdb_config_option_set_name(option, cname.as_ptr());
    let mut ty = duckdb_create_logical_type(DUCKDB_TYPE_DUCKDB_TYPE_BOOLEAN);
    duckdb_config_option_set_type(option, ty);
    let mut value = duckdb_create_bool(default);
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
    ok &= register_varchar_option(con, OPTION_LOCAL_JEV_API_URL, DEFAULT_LOCAL_JEV_API_URL);
    // An empty value distinguishes the provider-specific default from an explicit model choice.
    ok &= register_varchar_option(con, OPTION_MODEL, "");
    ok &= register_bigint_option(con, OPTION_TIMEOUT_MS, DEFAULT_TIMEOUT_MS as i64);
    ok &= register_bigint_option(
        con,
        OPTION_MAX_RESPONSE_BYTES,
        DEFAULT_MAX_RESPONSE_BYTES as i64,
    );
    ok &= register_varchar_option(con, OPTION_EXECUTION_MODE, "row");
    ok &= register_boolean_option(con, OPTION_BATCH_ENABLED, false);
    ok &= register_bigint_option(con, OPTION_BATCH_MAX_JUDGMENTS, 16);
    ok &= register_bigint_option(con, OPTION_MAX_REQUEST_BYTES, 262_144);
    ok &= register_bigint_option(con, OPTION_MAX_INFLIGHT, 4);
    ok &= register_boolean_option(con, OPTION_CACHE_ENABLED, false);
    ok &= register_bigint_option(con, OPTION_CACHE_MAX_ENTRIES, 10_000);
    ok &= register_bigint_option(con, OPTION_CACHE_MAX_BYTES, 67_108_864);
    ok &= register_bigint_option(con, OPTION_CACHE_TTL_MS, 60_000);
    ok &= register_boolean_option(con, OPTION_PROFILE_ENABLED, false);
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

unsafe fn read_option_bool(ctx: duckdb_client_context, name: &str) -> Option<bool> {
    let cname = CString::new(name).expect("option name has no NUL");
    let mut scope = 0;
    let value = duckdb_client_context_get_config_option(ctx, cname.as_ptr(), &mut scope);
    if value.is_null() {
        return None;
    }
    let out = duckdb_get_bool(value);
    let mut value = value;
    duckdb_destroy_value(&mut value);
    Some(out)
}

fn positive_limit(name: &str, value: i64) -> Result<usize, String> {
    usize::try_from(value)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("{name} must be a positive integer"))
}

/// Reads and validates the optimization settings for this scalar execution.
///
/// Each init callback gets a fresh snapshot, so a prepared statement observes settings changed
/// after it was prepared.
///
/// # Safety
/// `init_info` must be a valid DuckDB scalar-function initialization handle.
pub unsafe fn read_runtime_config(init_info: duckdb_init_info) -> Result<RuntimeConfig, String> {
    let mut ctx: duckdb_client_context = ptr::null_mut();
    duckdb_scalar_function_init_get_client_context(init_info, &mut ctx);
    if ctx.is_null() {
        return Err("internal error: client context is unavailable".to_string());
    }

    let mode = read_option_string(ctx, OPTION_EXECUTION_MODE);
    let batch_enabled = read_option_bool(ctx, OPTION_BATCH_ENABLED);
    let batch_max_judgments = read_option_int64(ctx, OPTION_BATCH_MAX_JUDGMENTS);
    let max_request_bytes = read_option_int64(ctx, OPTION_MAX_REQUEST_BYTES);
    let max_inflight = read_option_int64(ctx, OPTION_MAX_INFLIGHT);
    let cache_enabled = read_option_bool(ctx, OPTION_CACHE_ENABLED);
    let cache_max_entries = read_option_int64(ctx, OPTION_CACHE_MAX_ENTRIES);
    let cache_max_bytes = read_option_int64(ctx, OPTION_CACHE_MAX_BYTES);
    let cache_ttl_ms = read_option_int64(ctx, OPTION_CACHE_TTL_MS);
    let profile_enabled = read_option_bool(ctx, OPTION_PROFILE_ENABLED);
    duckdb_destroy_client_context(&mut ctx);

    let mut config = RuntimeConfig::from_values(
        mode.as_deref().unwrap_or("row"),
        batch_enabled.unwrap_or(false),
        batch_max_judgments.unwrap_or(16),
        max_request_bytes.unwrap_or(262_144),
        max_inflight.unwrap_or(4),
        cache_enabled.unwrap_or(false),
        cache_max_entries.unwrap_or(10_000),
        cache_max_bytes.unwrap_or(67_108_864),
        cache_ttl_ms.unwrap_or(60_000),
    )?;
    config.profile_enabled = profile_enabled.unwrap_or(false);
    Ok(config)
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

pub fn local_jev_credential_from_env() -> Option<String> {
    std::env::var("DUCKJEU_LOCAL_JEV_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn effective_model(provider: ProviderKind, configured_model: Option<String>) -> String {
    match configured_model.filter(|model| !model.is_empty()) {
        Some(model) => model,
        None if provider == ProviderKind::SelfHosted => DEFAULT_LOCAL_JEV_MODEL.to_string(),
        None => DEFAULT_MODEL.to_string(),
    }
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
    let local_jev_api_url = read_option_string(ctx, OPTION_LOCAL_JEV_API_URL);
    let model = read_option_string(ctx, OPTION_MODEL);
    let timeout_ms = read_option_int64(ctx, OPTION_TIMEOUT_MS);
    let max_response_bytes = read_option_int64(ctx, OPTION_MAX_RESPONSE_BYTES);
    duckdb_destroy_client_context(&mut ctx);

    let provider_raw = provider_raw.unwrap_or_else(|| ProviderKind::Mock.as_str().to_string());
    let provider = ProviderKind::parse(&provider_raw).map_err(|e| e.message().to_string())?;
    let api_url = match provider {
        ProviderKind::SelfHosted => match api_url {
            Some(url) if url != DEFAULT_API_URL => url,
            _ => local_jev_api_url.unwrap_or_else(|| DEFAULT_LOCAL_JEV_API_URL.to_string()),
        },
        _ => api_url.unwrap_or_else(|| DEFAULT_API_URL.to_string()),
    };
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
        api_url,
        model: effective_model(provider, model),
        timeout_ms,
        max_response_bytes,
        credential: match provider {
            ProviderKind::SelfHosted => local_jev_credential_from_env(),
            _ => credential_from_env(),
        },
    })
}

#[cfg(test)]
mod runtime_config_tests {
    use super::{effective_model, ExecutionMode, RuntimeConfig};
    use crate::provider::{ProviderKind, DEFAULT_LOCAL_JEV_MODEL, DEFAULT_MODEL};

    #[test]
    fn provider_default_model_resolves_to_service_specific_identity() {
        assert_eq!(effective_model(ProviderKind::Typesafe, None), DEFAULT_MODEL);
        assert_eq!(
            effective_model(ProviderKind::SelfHosted, None),
            DEFAULT_LOCAL_JEV_MODEL
        );
        assert_eq!(
            effective_model(ProviderKind::SelfHosted, Some(DEFAULT_MODEL.to_string())),
            DEFAULT_MODEL
        );
        assert_eq!(
            effective_model(
                ProviderKind::SelfHosted,
                Some("custom-local-model".to_string())
            ),
            "custom-local-model"
        );
    }

    #[test]
    fn row_defaults_and_optimized_limits_are_frozen() {
        let defaults = RuntimeConfig::from_values(
            "row", false, 16, 262_144, 4, false, 10_000, 67_108_864, 60_000,
        )
        .unwrap();
        assert_eq!(defaults.mode, ExecutionMode::Row);
        assert_eq!(defaults.max_inflight, 4);
        assert!(!defaults.cache_enabled);

        let optimized =
            RuntimeConfig::from_values("optimized", true, 8, 4096, 2, true, 100, 10_000, 5_000)
                .unwrap();
        assert_eq!(optimized.mode, ExecutionMode::Optimized);
        assert!(optimized.batch_enabled);
        assert!(optimized.cache_enabled);
        assert_eq!(optimized.batch_max_judgments, 8);
        assert_eq!(optimized.max_request_bytes, 4096);
        assert_eq!(optimized.max_inflight, 2);
        assert_eq!(optimized.cache_max_entries, 100);
        assert_eq!(optimized.cache_max_bytes, 10_000);
        assert_eq!(optimized.cache_ttl_ms, 5_000);
    }

    #[test]
    fn invalid_execution_settings_fail_before_dispatch() {
        let defaults = (false, 10_000, 67_108_864, 60_000);
        assert!(RuntimeConfig::from_values(
            "unknown", false, 16, 1, 1, defaults.0, defaults.1, defaults.2, defaults.3
        )
        .is_err());
        assert!(RuntimeConfig::from_values(
            "row", true, 16, 1, 1, defaults.0, defaults.1, defaults.2, defaults.3
        )
        .is_err());
        assert!(RuntimeConfig::from_values(
            "row", false, 16, 1, 1, true, defaults.1, defaults.2, defaults.3
        )
        .is_err());
        assert!(RuntimeConfig::from_values(
            "optimized",
            false,
            0,
            1,
            1,
            defaults.0,
            defaults.1,
            defaults.2,
            defaults.3
        )
        .is_err());
        assert!(RuntimeConfig::from_values(
            "optimized",
            false,
            1,
            0,
            1,
            defaults.0,
            defaults.1,
            defaults.2,
            defaults.3
        )
        .is_err());
        assert!(RuntimeConfig::from_values(
            "optimized",
            false,
            1,
            1,
            -1,
            defaults.0,
            defaults.1,
            defaults.2,
            defaults.3
        )
        .is_err());
        assert!(RuntimeConfig::from_values(
            "optimized",
            false,
            1,
            1,
            1,
            defaults.0,
            0,
            defaults.2,
            defaults.3
        )
        .is_err());
        assert!(RuntimeConfig::from_values(
            "optimized",
            false,
            1,
            1,
            1,
            defaults.0,
            defaults.1,
            0,
            defaults.3
        )
        .is_err());
        assert!(RuntimeConfig::from_values(
            "optimized",
            false,
            1,
            1,
            1,
            defaults.0,
            defaults.1,
            defaults.2,
            0
        )
        .is_err());
    }
}
