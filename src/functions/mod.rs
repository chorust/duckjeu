//! 宿主层桥接：函数注册、ANY 参数读取、NULL 处理、chunk 行映射与输出写回。
//!
//! DuckDB 向量、连接与内存生命周期只存在于本层；provider 与判断语义层不依赖它们
//! （见根规格 §9.1）。

pub mod bool_fn;
pub mod choice;
pub mod prob;

use std::collections::BTreeMap;
use std::ffi::{c_char, c_void, CStr, CString};

use duckdb::ffi::*;

use crate::config;
use crate::judgment::{bool_from_probability, JudgmentError, JudgmentRequest, JudgmentResult};
use crate::provider::{provider_for, Provider, ProviderContext};
use crate::serialize::{encode_struct, encode_text, CanonicalState, TypedValue};

/// 三个 SQL 函数共享同一执行路径，仅输出映射不同。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionKind {
    Prob,
    Bool,
    Choice,
}

/// 每次执行冻结的上下文与 provider 实例。
pub struct ExecutionState {
    #[allow(dead_code)]
    pub context: ProviderContext,
    pub provider: Result<Box<dyn Provider>, String>,
}

unsafe extern "C" fn drop_execution_state(ptr: *mut c_void) {
    if !ptr.is_null() {
        drop(Box::from_raw(ptr.cast::<ExecutionState>()));
    }
}

/// 三个函数共用的 init 回调：读取 session 配置并构造 provider。
///
/// # Safety
/// 由 DuckDB 在函数执行初始化时调用，`info` 由宿主保证有效。
pub unsafe extern "C" fn init(info: duckdb_init_info) {
    let state = match config::read_provider_context(info) {
        Ok(context) => {
            let provider = provider_for(&context).map_err(|e| e.message().to_string());
            ExecutionState { context, provider }
        }
        Err(message) => ExecutionState {
            context: ProviderContext::default(),
            provider: Err(message),
        },
    };
    duckdb_scalar_function_init_set_state(
        info,
        Box::into_raw(Box::new(state)).cast(),
        Some(drop_execution_state),
    );
}

unsafe fn set_error(info: duckdb_function_info, message: &str) {
    // 错误信息已在判断/provider 层脱敏；此处只做 C 字符串转换。
    let sanitized: String = message.replace('\0', " ");
    if let Ok(cstr) = CString::new(sanitized) {
        duckdb_scalar_function_set_error(info, cstr.as_ptr());
    }
}

unsafe fn is_null(vector: duckdb_vector, row: usize) -> bool {
    let validity = duckdb_vector_get_validity(vector);
    !validity.is_null() && !duckdb_validity_row_is_valid(validity, row as idx_t)
}

unsafe fn set_output_null(output: duckdb_vector, row: usize) {
    duckdb_vector_ensure_validity_writable(output);
    let validity = duckdb_vector_get_validity(output);
    if !validity.is_null() {
        duckdb_validity_set_row_invalid(validity, row as idx_t);
    }
}

unsafe fn read_varchar(vector: duckdb_vector, row: usize) -> Option<String> {
    let data = duckdb_vector_get_data(vector) as *const duckdb_string_t;
    if data.is_null() {
        return None;
    }
    let mut value = *data.add(row);
    let length = duckdb_string_t_length(value) as usize;
    let ptr = duckdb_string_t_data(&mut value);
    if ptr.is_null() {
        return Some(String::new());
    }
    let bytes = std::slice::from_raw_parts(ptr as *const u8, length);
    Some(String::from_utf8_lossy(bytes).into_owned())
}

fn hugeint_to_i128(value: duckdb_hugeint) -> i128 {
    ((value.upper as i128) << 64) | (value.lower as i128)
}

unsafe fn read_value(
    vector: duckdb_vector,
    logical_type: duckdb_logical_type,
    row: usize,
) -> Result<TypedValue, JudgmentError> {
    let type_id = duckdb_get_type_id(logical_type);
    let data = duckdb_vector_get_data(vector);
    if data.is_null() {
        // DuckDB 在执行前会 Flatten，正常情况下不会为空；防御性检查避免解引用空指针。
        return Err(JudgmentError::invalid_input(
            "internal error: state vector data is unavailable",
        ));
    }
    let unsupported = |what: &str| {
        JudgmentError::invalid_input(format!(
            "unsupported state field type: {what}; v0.1 supports BOOLEAN, integers, finite floats, VARCHAR and NULL"
        ))
    };
    match type_id {
        DUCKDB_TYPE_DUCKDB_TYPE_BOOLEAN => {
            let value = *(data as *const u8).add(row);
            Ok(TypedValue::Bool(value != 0))
        }
        DUCKDB_TYPE_DUCKDB_TYPE_TINYINT => {
            Ok(TypedValue::Int(*(data as *const i8).add(row) as i64))
        }
        DUCKDB_TYPE_DUCKDB_TYPE_SMALLINT => {
            Ok(TypedValue::Int(*(data as *const i16).add(row) as i64))
        }
        DUCKDB_TYPE_DUCKDB_TYPE_INTEGER => {
            Ok(TypedValue::Int(*(data as *const i32).add(row) as i64))
        }
        DUCKDB_TYPE_DUCKDB_TYPE_BIGINT => Ok(TypedValue::Int(*(data as *const i64).add(row))),
        DUCKDB_TYPE_DUCKDB_TYPE_UTINYINT => {
            Ok(TypedValue::UInt(*(data as *const u8).add(row) as u64))
        }
        DUCKDB_TYPE_DUCKDB_TYPE_USMALLINT => {
            Ok(TypedValue::UInt(*(data as *const u16).add(row) as u64))
        }
        DUCKDB_TYPE_DUCKDB_TYPE_UINTEGER => {
            Ok(TypedValue::UInt(*(data as *const u32).add(row) as u64))
        }
        DUCKDB_TYPE_DUCKDB_TYPE_UBIGINT => Ok(TypedValue::UInt(*(data as *const u64).add(row))),
        DUCKDB_TYPE_DUCKDB_TYPE_FLOAT => {
            Ok(TypedValue::Float(*(data as *const f32).add(row) as f64))
        }
        DUCKDB_TYPE_DUCKDB_TYPE_DOUBLE => Ok(TypedValue::Float(*(data as *const f64).add(row))),
        DUCKDB_TYPE_DUCKDB_TYPE_VARCHAR => Ok(TypedValue::Text(
            read_varchar(vector, row).unwrap_or_default(),
        )),
        DUCKDB_TYPE_DUCKDB_TYPE_DECIMAL => {
            let width = duckdb_decimal_width(logical_type);
            let scale = duckdb_decimal_scale(logical_type);
            let internal = duckdb_decimal_internal_type(logical_type);
            let raw: i128 = match internal {
                DUCKDB_TYPE_DUCKDB_TYPE_SMALLINT => *(data as *const i16).add(row) as i128,
                DUCKDB_TYPE_DUCKDB_TYPE_INTEGER => *(data as *const i32).add(row) as i128,
                DUCKDB_TYPE_DUCKDB_TYPE_BIGINT => *(data as *const i64).add(row) as i128,
                DUCKDB_TYPE_DUCKDB_TYPE_HUGEINT => {
                    hugeint_to_i128(*(data as *const duckdb_hugeint).add(row))
                }
                other => {
                    return Err(unsupported(&format!(
                        "DECIMAL({width}) with internal type id {other}"
                    )))
                }
            };
            Ok(TypedValue::Decimal(crate::serialize::format_decimal(
                raw, scale,
            )))
        }
        DUCKDB_TYPE_DUCKDB_TYPE_HUGEINT => Ok(TypedValue::BigInt(
            hugeint_to_i128(*(data as *const duckdb_hugeint).add(row)).to_string(),
        )),
        DUCKDB_TYPE_DUCKDB_TYPE_UHUGEINT => {
            let value = *(data as *const duckdb_uhugeint).add(row);
            Ok(TypedValue::UBigInt(
                (((value.upper as u128) << 64) | value.lower as u128).to_string(),
            ))
        }
        DUCKDB_TYPE_DUCKDB_TYPE_SQLNULL => Ok(TypedValue::Null),
        other => Err(unsupported(&format!("duckdb type id {other}"))),
    }
}

unsafe fn read_struct(
    vector: duckdb_vector,
    logical_type: duckdb_logical_type,
    row: usize,
) -> Result<BTreeMap<String, TypedValue>, JudgmentError> {
    let count = duckdb_struct_type_child_count(logical_type);
    let mut fields = BTreeMap::new();
    let unsupported = |what: &str| {
        JudgmentError::invalid_input(format!(
            "unsupported state field type: {what}; v0.1 supports BOOLEAN, integers, finite floats, VARCHAR and NULL"
        ))
    };
    for index in 0..count {
        let name_ptr: *mut c_char = duckdb_struct_type_child_name(logical_type, index);
        let name = if name_ptr.is_null() {
            String::new()
        } else {
            let owned = CStr::from_ptr(name_ptr).to_string_lossy().into_owned();
            duckdb_free(name_ptr.cast::<c_void>());
            owned
        };
        let child = duckdb_struct_vector_get_child(vector, index);
        let mut child_type = duckdb_struct_type_child_type(logical_type, index);
        let child_type_id = duckdb_get_type_id(child_type);
        if !matches!(
            child_type_id,
            DUCKDB_TYPE_DUCKDB_TYPE_BOOLEAN
                | DUCKDB_TYPE_DUCKDB_TYPE_TINYINT
                | DUCKDB_TYPE_DUCKDB_TYPE_SMALLINT
                | DUCKDB_TYPE_DUCKDB_TYPE_INTEGER
                | DUCKDB_TYPE_DUCKDB_TYPE_BIGINT
                | DUCKDB_TYPE_DUCKDB_TYPE_UTINYINT
                | DUCKDB_TYPE_DUCKDB_TYPE_USMALLINT
                | DUCKDB_TYPE_DUCKDB_TYPE_UINTEGER
                | DUCKDB_TYPE_DUCKDB_TYPE_UBIGINT
                | DUCKDB_TYPE_DUCKDB_TYPE_FLOAT
                | DUCKDB_TYPE_DUCKDB_TYPE_DOUBLE
                | DUCKDB_TYPE_DUCKDB_TYPE_VARCHAR
                | DUCKDB_TYPE_DUCKDB_TYPE_DECIMAL
                | DUCKDB_TYPE_DUCKDB_TYPE_HUGEINT
                | DUCKDB_TYPE_DUCKDB_TYPE_UHUGEINT
                | DUCKDB_TYPE_DUCKDB_TYPE_SQLNULL
        ) {
            let error = unsupported(&format!("duckdb type id {child_type_id}"));
            duckdb_destroy_logical_type(&mut child_type);
            return Err(error);
        }
        if is_null(child, row) {
            // 内部 NULL 字段按 NULL 保留，不读取底层数据。
            duckdb_destroy_logical_type(&mut child_type);
            fields.insert(name, TypedValue::Null);
            continue;
        }
        let value = read_value(child, child_type, row);
        duckdb_destroy_logical_type(&mut child_type);
        fields.insert(name, value?);
    }
    Ok(fields)
}

unsafe fn read_state(vector: duckdb_vector, row: usize) -> Result<CanonicalState, JudgmentError> {
    let mut logical_type = duckdb_vector_get_column_type(vector);
    let type_id = duckdb_get_type_id(logical_type);
    // 注意：不能用 `?` 直接返回，否则会泄漏 `logical_type`。
    let result = match type_id {
        DUCKDB_TYPE_DUCKDB_TYPE_VARCHAR => {
            Ok(encode_text(&read_varchar(vector, row).unwrap_or_default()))
        }
        DUCKDB_TYPE_DUCKDB_TYPE_STRUCT => match read_struct(vector, logical_type, row) {
            Ok(fields) => encode_struct(fields)
                .map_err(|e| JudgmentError::invalid_input(e.message().to_string())),
            Err(error) => Err(error),
        },
        _ => Err(JudgmentError::invalid_input(
            "unsupported state type: expected STRUCT or VARCHAR",
        )),
    };
    duckdb_destroy_logical_type(&mut logical_type);
    result
}

unsafe fn read_string_list(
    vector: duckdb_vector,
    row: usize,
) -> Result<Vec<String>, JudgmentError> {
    let data = duckdb_vector_get_data(vector) as *const duckdb_list_entry;
    if data.is_null() {
        return Err(JudgmentError::invalid_input("choices must be a list"));
    }
    let entry = *data.add(row);
    let child = duckdb_list_vector_get_child(vector);
    let mut labels = Vec::with_capacity(entry.length as usize);
    for index in 0..entry.length {
        let child_row = (entry.offset + index) as usize;
        if is_null(child, child_row) {
            return Err(JudgmentError::invalid_input(
                "choices must not contain NULL labels",
            ));
        }
        labels.push(read_varchar(child, child_row).unwrap_or_default());
    }
    Ok(labels)
}

unsafe fn evaluate_row(
    kind: FunctionKind,
    provider: &Result<Box<dyn Provider>, String>,
    input: duckdb_data_chunk,
    row: usize,
) -> Result<Option<JudgmentResult>, JudgmentError> {
    let state_vector = duckdb_data_chunk_get_vector(input, 0);
    let text_vector = duckdb_data_chunk_get_vector(input, 1);
    // 顶层 NULL 短路：结果为 NULL 且不调用 provider。
    if is_null(state_vector, row) || is_null(text_vector, row) {
        return Ok(None);
    }

    // choice 的 choices 也是顶层可空参数；在读取/校验 state 之前先短路。
    let choices_vector = match kind {
        FunctionKind::Choice => {
            let vector = duckdb_data_chunk_get_vector(input, 2);
            if is_null(vector, row) {
                return Ok(None);
            }
            Some(vector)
        }
        FunctionKind::Prob | FunctionKind::Bool => None,
    };

    let state = read_state(state_vector, row)?;
    let text = read_varchar(text_vector, row).unwrap_or_default();
    let request = match kind {
        FunctionKind::Prob | FunctionKind::Bool => {
            JudgmentRequest::noul(state, &text)?
        }
        FunctionKind::Choice => {
            let choices_vector = choices_vector.ok_or_else(|| {
                JudgmentError::invalid_input("internal error: choices vector is missing")
            })?;
            let choices = read_string_list(choices_vector, row)?;
            JudgmentRequest::choice(state, &text, &choices)?
        }
    };
    // Provider 初始化错误只对真正需要判断的行可见；NULL 行保持 NULL。
    let provider = provider
        .as_ref()
        .map_err(|message| JudgmentError::configuration(message.clone()))?;
    provider.judge(&request).map(Some)
}

unsafe fn write_result(
    kind: FunctionKind,
    output: duckdb_vector,
    row: usize,
    result: &JudgmentResult,
) {
    match (kind, result) {
        (FunctionKind::Prob, JudgmentResult::Noul(probability)) => {
            let data = duckdb_vector_get_data(output) as *mut f64;
            *data.add(row) = *probability;
        }
        (FunctionKind::Bool, JudgmentResult::Noul(probability)) => {
            let data = duckdb_vector_get_data(output) as *mut u8;
            *data.add(row) = u8::from(bool_from_probability(*probability));
        }
        (FunctionKind::Choice, JudgmentResult::Choice(label)) => {
            duckdb_vector_assign_string_element_len(
                output,
                row as idx_t,
                label.as_ptr().cast::<c_char>(),
                label.len() as idx_t,
            );
        }
        _ => {}
    }
}

/// 三个函数共用的 eval 主体：逐行映射，失败即让查询报错。
///
/// # Safety
/// `info`/`input`/`output` 必须来自同一次标量函数调用，且 `output` 已按函数返回类型分配。
pub unsafe fn run(
    kind: FunctionKind,
    info: duckdb_function_info,
    input: duckdb_data_chunk,
    output: duckdb_vector,
) {
    let state_ptr = duckdb_scalar_function_get_state(info);
    if state_ptr.is_null() {
        set_error(info, "internal error: execution state is missing");
        return;
    }
    let state = &*(state_ptr as *const ExecutionState);
    let row_count = duckdb_data_chunk_get_size(input) as usize;
    for row in 0..row_count {
        match evaluate_row(kind, &state.provider, input, row) {
            Ok(None) => set_output_null(output, row),
            Ok(Some(result)) => write_result(kind, output, row, &result),
            Err(error) => {
                set_error(info, error.message());
                return;
            }
        }
    }
}

/// 注册一个 duckjeu 标量函数：ANY state + VARCHAR 问题（+ choice 的 LIST<VARCHAR>）。
///
/// # Safety
/// `con` 必须是有效的 DuckDB 连接；`eval` 必须匹配注册的签名。
pub unsafe fn register_function(
    con: duckdb_connection,
    name: &str,
    kind: FunctionKind,
    eval: unsafe extern "C" fn(duckdb_function_info, duckdb_data_chunk, duckdb_vector),
) -> bool {
    let mut function = duckdb_create_scalar_function();
    let cname = match CString::new(name) {
        Ok(value) => value,
        Err(_) => return false,
    };
    duckdb_scalar_function_set_name(function, cname.as_ptr());

    let mut any = duckdb_create_logical_type(DUCKDB_TYPE_DUCKDB_TYPE_ANY);
    duckdb_scalar_function_add_parameter(function, any);
    let mut varchar = duckdb_create_logical_type(DUCKDB_TYPE_DUCKDB_TYPE_VARCHAR);
    duckdb_scalar_function_add_parameter(function, varchar);
    if kind == FunctionKind::Choice {
        let mut list = duckdb_create_list_type(varchar);
        duckdb_scalar_function_add_parameter(function, list);
        duckdb_destroy_logical_type(&mut list);
    }

    let return_type_id = match kind {
        FunctionKind::Prob => DUCKDB_TYPE_DUCKDB_TYPE_DOUBLE,
        FunctionKind::Bool => DUCKDB_TYPE_DUCKDB_TYPE_BOOLEAN,
        FunctionKind::Choice => DUCKDB_TYPE_DUCKDB_TYPE_VARCHAR,
    };
    let mut return_type = duckdb_create_logical_type(return_type_id);
    duckdb_scalar_function_set_return_type(function, return_type);

    // 外部模型调用有成本且可能非确定：不能标为确定性函数。
    duckdb_scalar_function_set_volatile(function);
    // NULL 由扩展自行处理（顶层 NULL 短路，内部 NULL 保留）。
    duckdb_scalar_function_set_special_handling(function);
    duckdb_scalar_function_set_init(function, Some(init));
    duckdb_scalar_function_set_function(function, Some(eval));

    let rc = duckdb_register_scalar_function(con, function);
    duckdb_destroy_scalar_function(&mut function);
    duckdb_destroy_logical_type(&mut return_type);
    duckdb_destroy_logical_type(&mut varchar);
    duckdb_destroy_logical_type(&mut any);
    rc == DuckDBSuccess
}
