//! Canonical state encoding（判断语义层，不依赖 DuckDB 或网络）。
//!
//! 见 `specs/001-judgment-foundation/contracts/sql-api.md` 的 state 编码约定与
//! `specs/001-judgment-foundation/research.md` D6。canonical bytes 是确定性的完整比较
//! 内容，供 v0.2 缓存/去重使用；因此这里不能只保留 hash。

use std::collections::BTreeMap;
use std::fmt;

/// 编码版本，随 canonical 格式变更递增（v0.2 缓存 key 的一部分）。
pub const CANONICAL_VERSION: u32 = 1;

/// 状态的来源类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateKind {
    /// VARCHAR state：原始文本，不做 JSON 包装。
    Text,
    /// STRUCT state：类型化、字段按名称排序的编码。
    Struct,
}

/// 类型化取值树。v0.1 支持 BOOLEAN、各宽度整数、有限浮点、VARCHAR 与 NULL。
#[derive(Debug, Clone, PartialEq)]
pub enum TypedValue {
    Null,
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    /// DECIMAL：保留精确十进制文本（不经过浮点转换）。
    Decimal(String),
    /// HUGEINT（128 位有符号）：精确十进制文本。
    BigInt(String),
    /// UHUGEINT（128 位无符号）：精确十进制文本。
    UBigInt(String),
    Text(String),
    Struct(BTreeMap<String, TypedValue>),
}

/// 规范化后的状态。
#[derive(Debug, Clone, PartialEq)]
pub struct CanonicalState {
    kind: StateKind,
    canonical: Vec<u8>,
    value: TypedValue,
}

impl CanonicalState {
    pub fn kind(&self) -> StateKind {
        self.kind
    }

    /// 确定性 canonical bytes（文本状态即原始 UTF-8 字节）。
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }

    /// 类型化取值，用于构造 provider 请求体。
    pub fn value(&self) -> &TypedValue {
        &self.value
    }

    /// 文本状态的原文；struct 状态返回 None。
    pub fn as_text(&self) -> Option<&str> {
        match &self.value {
            TypedValue::Text(s) => Some(s.as_str()),
            _ => None,
        }
    }
}

/// 把 DECIMAL 的底层整数与 scale 渲染为精确十进制文本。
pub fn format_decimal(raw: i128, scale: u8) -> String {
    if scale == 0 {
        return raw.to_string();
    }
    let negative = raw < 0;
    let digits = raw.unsigned_abs().to_string();
    let scale = scale as usize;
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    if digits.len() <= scale {
        out.push_str("0.");
        for _ in 0..(scale - digits.len()) {
            out.push('0');
        }
        out.push_str(&digits);
    } else {
        let split = digits.len() - scale;
        out.push_str(&digits[..split]);
        out.push('.');
        out.push_str(&digits[split..]);
    }
    out
}

/// 编码失败（不支持的取值或不合法输入）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerializeError(String);

impl SerializeError {
    pub fn new(msg: impl Into<String>) -> Self {
        SerializeError(msg.into())
    }

    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SerializeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for SerializeError {}

/// VARCHAR state：原文保留，canonical bytes 即原始字节。
pub fn encode_text(text: &str) -> CanonicalState {
    CanonicalState {
        kind: StateKind::Text,
        canonical: text.as_bytes().to_vec(),
        value: TypedValue::Text(text.to_string()),
    }
}

/// STRUCT state：字段按名称排序编码；拒绝非有限浮点。
pub fn encode_struct(
    fields: BTreeMap<String, TypedValue>,
) -> Result<CanonicalState, SerializeError> {
    validate_value(&TypedValue::Struct(fields.clone()))?;
    let mut buf = String::new();
    buf.push_str("{\"canonical_version\":");
    buf.push_str(&CANONICAL_VERSION.to_string());
    buf.push_str(",\"state\":");
    write_value(&mut buf, &TypedValue::Struct(fields.clone()));
    buf.push('}');
    Ok(CanonicalState {
        kind: StateKind::Struct,
        canonical: buf.into_bytes(),
        value: TypedValue::Struct(fields),
    })
}

fn validate_value(value: &TypedValue) -> Result<(), SerializeError> {
    match value {
        TypedValue::Float(v) => {
            if !v.is_finite() {
                return Err(SerializeError::new(
                    "state contains a non-finite floating point value",
                ));
            }
            Ok(())
        }
        TypedValue::Struct(fields) => {
            for v in fields.values() {
                validate_value(v)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn write_value(buf: &mut String, value: &TypedValue) {
    match value {
        TypedValue::Null => buf.push_str("{\"t\":\"null\"}"),
        TypedValue::Bool(b) => {
            buf.push_str("{\"t\":\"bool\",\"v\":");
            buf.push_str(if *b { "true" } else { "false" });
            buf.push('}');
        }
        TypedValue::Int(i) => {
            buf.push_str("{\"t\":\"int\",\"v\":\"");
            buf.push_str(&i.to_string());
            buf.push_str("\"}");
        }
        TypedValue::UInt(u) => {
            buf.push_str("{\"t\":\"uint\",\"v\":\"");
            buf.push_str(&u.to_string());
            buf.push_str("\"}");
        }
        TypedValue::Float(f) => {
            buf.push_str("{\"t\":\"float\",\"v\":");
            // Rust 的 Debug 对 f64 给出最短往返表示。
            buf.push_str(&format!("{f:?}"));
            buf.push('}');
        }
        TypedValue::Decimal(d) => {
            buf.push_str("{\"t\":\"decimal\",\"v\":");
            buf.push_str(&serde_json::to_string(d).expect("string is always serializable"));
            buf.push('}');
        }
        TypedValue::BigInt(d) => {
            buf.push_str("{\"t\":\"int128\",\"v\":\"");
            buf.push_str(d);
            buf.push_str("\"}");
        }
        TypedValue::UBigInt(d) => {
            buf.push_str("{\"t\":\"uint128\",\"v\":\"");
            buf.push_str(d);
            buf.push_str("\"}");
        }
        TypedValue::Text(s) => {
            buf.push_str("{\"t\":\"text\",\"v\":");
            buf.push_str(&serde_json::to_string(s).expect("string is always serializable"));
            buf.push('}');
        }
        TypedValue::Struct(fields) => {
            buf.push_str("{\"t\":\"struct\",\"f\":{");
            // BTreeMap 迭代即按字段名排序。
            for (idx, (name, v)) in fields.iter().enumerate() {
                if idx > 0 {
                    buf.push(',');
                }
                buf.push_str(&serde_json::to_string(name).expect("string is always serializable"));
                buf.push(':');
                write_value(buf, v);
            }
            buf.push_str("}}");
        }
    }
}

/// 请求体中使用的 JSON 取值：文本状态为原始字符串，struct 状态为节点对象。
pub fn to_json_value(value: &TypedValue) -> serde_json::Value {
    match value {
        TypedValue::Null => serde_json::json!({ "t": "null" }),
        TypedValue::Bool(b) => serde_json::json!({ "t": "bool", "v": b }),
        TypedValue::Int(i) => serde_json::json!({ "t": "int", "v": i.to_string() }),
        TypedValue::UInt(u) => serde_json::json!({ "t": "uint", "v": u.to_string() }),
        TypedValue::Float(f) => serde_json::json!({ "t": "float", "v": f }),
        TypedValue::Decimal(d) => serde_json::json!({ "t": "decimal", "v": d }),
        TypedValue::BigInt(d) => serde_json::json!({ "t": "int128", "v": d }),
        TypedValue::UBigInt(d) => serde_json::json!({ "t": "uint128", "v": d }),
        TypedValue::Text(s) => serde_json::json!({ "t": "text", "v": s }),
        TypedValue::Struct(fields) => {
            let mut map = serde_json::Map::new();
            for (name, v) in fields {
                map.insert(name.clone(), to_json_value(v));
            }
            serde_json::json!({ "t": "struct", "f": serde_json::Value::Object(map) })
        }
    }
}
