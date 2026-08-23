//! rwir `json.to` / `json.from`：KV 子树 ↔ JSON 文本（go/json 的 Rust 内嵌，语义照搬）。
//!   json.to(root)   -> str   root 整棵子树读成 JSON：目录→嵌套对象；compact 数组与
//!                            散 key 数组 name[0..N-1] 统一序列化为 JSON 数组。
//!   json.from(json) -> root  反序列化 JSON 写回 root 子树。
//! 编码走权威 kvspace ABI：DecodeHead 读头、TlvEncode/NewCharByte 编码。

use serde_json::{Map, Value};

use crate::engine::Engine;
use crate::ffi::*;

pub fn to(eng: &Engine, pc: &str) {
    let names = params(eng, pc);
    let root = if names[1].starts_with('/') {
        names[1].clone()
    } else {
        eng.read0(pc)
    };
    let out = build_value(eng, &root).to_string();
    eng.set_kv(&eng.write0(pc), &out);
}

pub fn from(eng: &Engine, pc: &str) {
    let names = params(eng, pc);
    let src = eng.read0(pc);
    let root = if names[2].starts_with('/') {
        names[2].clone()
    } else {
        eng.write0(pc)
    };
    if let Ok(v) = serde_json::from_str::<Value>(&src) {
        if let Value::Object(_) = v {
            write_map(eng, &root, &v);
        }
    }
}

fn params(eng: &Engine, pc: &str) -> Vec<String> {
    let s = take(unsafe { kvlang_rwirextParams(eng.kv, cs(pc).as_ptr()) });
    s.lines().map(str::to_string).collect()
}

// ── KV 子树 → JSON ────────────────────────────────────────────────

fn build_value(eng: &Engine, root: &str) -> Value {
    let mut map = Map::new();
    let mut scat: std::collections::BTreeMap<String, Vec<usize>> = Default::default();
    for child in eng.list_kv(&format!("{root}/")) {
        if child.ends_with('/') {
            let name = child.trim_end_matches('/').to_string();
            map.insert(name.clone(), build_value(eng, &format!("{root}/{name}")));
        } else if let Some((base, idx)) = split_array_name(&child) {
            scat.entry(base).or_default().push(idx);
        } else {
            let (kind, raw, arr_len) = parse_tlv(&eng.get_tlv(&format!("{root}/{child}")));
            map.insert(child, tlv_to_json(&kind, &raw, arr_len));
        }
    }
    for (base, mut idxs) in scat {
        idxs.sort_unstable();
        let arr = idxs
            .iter()
            .map(|i| {
                let (kind, raw, arr_len) =
                    parse_tlv(&eng.get_tlv(&format!("{root}/{base}[{i}]")));
                tlv_to_json(&kind, &raw, arr_len)
            })
            .collect();
        map.insert(base, Value::Array(arr));
    }
    Value::Object(map)
}

fn split_array_name(name: &str) -> Option<(String, usize)> {
    let lt = name.rfind('[')?;
    if lt == 0 || !name.ends_with(']') {
        return None;
    }
    let idx = name[lt + 1..name.len() - 1].parse().ok()?;
    Some((name[..lt].to_string(), idx))
}

fn parse_tlv(data: &[u8]) -> (String, Vec<u8>, usize) {
    let mut h = KvspaceHead::default();
    if data.is_empty()
        || unsafe { kvspaceDecodeHead(data.as_ptr(), data.len() as u32, &mut h) } != 0
    {
        return (String::new(), Vec::new(), 1);
    }
    let kx = String::from_utf8_lossy(&h.kindexpr)
        .trim_end_matches('\0')
        .to_string();
    let (_, dims, kind) = parse_kindexpr(&kx);
    let (bo, bl) = (h.body_offset as usize, h.body_len.max(0) as usize);
    let raw = if bo + bl <= data.len() {
        data[bo..bo + bl].to_vec()
    } else {
        Vec::new()
    };
    let mut arr_len = 1usize;
    for d in &dims {
        arr_len *= (*d).max(1) as usize;
    }
    (kind, raw, arr_len)
}

fn tlv_to_json(kind: &str, raw: &[u8], arr_len: usize) -> Value {
    let es = elem_size(kind);
    match kind {
        "bool" => {
            if arr_len > 1 {
                Value::Array((0..arr_len).map(|i| Value::Bool(raw[i] != 0)).collect())
            } else {
                Value::Bool(raw.first().map(|&b| b != 0).unwrap_or(false))
            }
        }
        "int8" | "int16" | "int32" | "int64" | "uint8" | "uint16" | "uint32" | "uint64" => {
            if arr_len > 1 {
                Value::Array(
                    (0..arr_len)
                        .map(|i| Value::from(read_int(&raw[i * es..i * es + es])))
                        .collect(),
                )
            } else {
                Value::from(read_int(raw))
            }
        }
        "float32" | "float64" => {
            if arr_len > 1 {
                Value::Array(
                    (0..arr_len)
                        .map(|i| Value::from(float_from(&raw[i * es..i * es + es])))
                        .collect(),
                )
            } else {
                Value::from(float_from(raw))
            }
        }
        "char/utf8" | "char/ascii" => Value::String(String::from_utf8_lossy(raw).into_owned()),
        "char/utf32" => Value::String(utf32_to_string(raw)),
        _ => Value::String(String::from_utf8_lossy(raw).into_owned()),
    }
}

fn elem_size(kind: &str) -> usize {
    match kind {
        "int8" | "uint8" | "bool" => 1,
        "int16" | "uint16" => 2,
        "int32" | "uint32" | "float32" => 4,
        "int64" | "uint64" | "float64" => 8,
        _ => 0,
    }
}

fn read_int(raw: &[u8]) -> i64 {
    match raw.len() {
        1 => raw[0] as i8 as i64,
        2 => i16::from_le_bytes(raw.try_into().unwrap()) as i64,
        4 => i32::from_le_bytes(raw.try_into().unwrap()) as i64,
        8 => i64::from_le_bytes(raw.try_into().unwrap()),
        _ => 0,
    }
}

fn float_from(raw: &[u8]) -> f64 {
    match raw.len() {
        4 => f32::from_le_bytes(raw.try_into().unwrap()) as f64,
        8 => f64::from_le_bytes(raw.try_into().unwrap()),
        _ => 0.0,
    }
}

fn utf32_to_string(raw: &[u8]) -> String {
    raw.chunks_exact(4)
        .map(|c| char::from_u32(u32::from_le_bytes(c.try_into().unwrap())).unwrap_or('\u{FFFD}'))
        .collect()
}

// ── JSON → KV 子树 ────────────────────────────────────────────────

fn write_map(eng: &Engine, root: &str, v: &Value) {
    if let Value::Object(map) = v {
        for (k, val) in map {
            let child = format!("{root}/{k}");
            match val {
                Value::Object(_) => {
                    eng.mkindex(&format!("{child}/"));
                    write_map(eng, &child, val);
                }
                Value::Array(arr) => eng.set_tlv(&child, &array_to_tlv(arr)),
                _ => eng.set_tlv(&child, &value_to_tlv(val)),
            }
        }
    }
}

fn value_to_tlv(v: &Value) -> Vec<u8> {
    match v {
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                tlv_encode("int64", &i.to_le_bytes(), 1)
            } else {
                tlv_encode("float64", &n.as_f64().unwrap_or(0.0).to_le_bytes(), 1)
            }
        }
        Value::Bool(b) => tlv_encode("bool", &[*b as u8], 1),
        Value::String(s) => new_char_byte(s.as_bytes()),
        _ => Vec::new(),
    }
}

fn array_to_tlv(arr: &[Value]) -> Vec<u8> {
    match arr.first() {
        Some(Value::Number(_)) => {
            if arr.iter().all(|e| e.as_i64().is_some()) {
                let mut raw = Vec::with_capacity(arr.len() * 8);
                for e in arr {
                    raw.extend_from_slice(&e.as_i64().unwrap().to_le_bytes());
                }
                tlv_encode("int64", &raw, arr.len())
            } else {
                let mut raw = Vec::with_capacity(arr.len() * 8);
                for e in arr {
                    raw.extend_from_slice(&e.as_f64().unwrap_or(0.0).to_le_bytes());
                }
                tlv_encode("float64", &raw, arr.len())
            }
        }
        Some(Value::Bool(_)) => {
            let raw: Vec<u8> = arr.iter().map(|e| e.as_bool().unwrap_or(false) as u8).collect();
            tlv_encode("bool", &raw, arr.len())
        }
        _ => Vec::new(),
    }
}

// ── TLV 编码（权威 kvspace ABI）───────────────────────────────────

fn tlv_encode(kind: &str, raw: &[u8], arr_len: usize) -> Vec<u8> {
    unsafe {
        let (mut out, mut olen) = (std::ptr::null_mut(), 0u32);
        let dims = [arr_len as i32];
        let (dptr, ndim) = if arr_len > 1 {
            (dims.as_ptr(), 1i32)
        } else {
            (std::ptr::null(), 0i32)
        };
        kvspaceTlvEncode(
            cs(kind).as_ptr(),
            raw.as_ptr(),
            raw.len() as u32,
            dptr,
            ndim,
            &mut out,
            &mut olen,
        );
        if out.is_null() || olen == 0 {
            return Vec::new();
        }
        let v = std::slice::from_raw_parts(out, olen as usize).to_vec();
        kvspaceBytesFree(out, olen);
        v
    }
}

fn new_char_byte(bytes: &[u8]) -> Vec<u8> {
    unsafe {
        let (mut out, mut olen) = (std::ptr::null_mut(), 0u32);
        kvspaceNewCharByte(bytes.as_ptr(), bytes.len() as u32, &mut out, &mut olen);
        if out.is_null() || olen == 0 {
            return Vec::new();
        }
        let v = std::slice::from_raw_parts(out, olen as usize).to_vec();
        kvspaceBytesFree(out, olen);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::c_char;

    fn i64_tlv(vals: &[i64]) -> Vec<u8> {
        let mut raw = Vec::with_capacity(vals.len() * 8);
        for v in vals {
            raw.extend_from_slice(&v.to_le_bytes());
        }
        tlv_encode("int64", &raw, vals.len())
    }

    fn test_engine() -> Engine {
        let dsn = "fs:///tmp/byteseek-json-test";
        let kv = unsafe { kvspaceConnect(cs(dsn).as_ptr()) };
        assert!(!kv.is_null());
        let mut err = [0u8; 256];
        unsafe { kvspaceClear(kv, err.as_mut_ptr() as *mut c_char, 256) };
        Engine {
            rt: std::ptr::null_mut(),
            kv,
            dsn: dsn.to_string(),
            subs: std::cell::Cell::new(0),
        }
    }

    #[test]
    fn roundtrip() {
        let eng = test_engine();
        eng.set_tlv("/data/age", &i64_tlv(&[42]));
        eng.set_tlv("/data/name", &new_char_byte(b"alice"));
        eng.set_tlv("/data/score", &tlv_encode("float64", &3.14f64.to_le_bytes(), 1));
        eng.set_tlv("/data/active", &tlv_encode("bool", &[1], 1));
        eng.set_tlv("/data/grp/c", &i64_tlv(&[1]));
        eng.set_tlv("/data/grp/d", &i64_tlv(&[2]));
        eng.set_tlv("/data/cont", &i64_tlv(&[10, 20, 30]));
        eng.set_tlv("/data/scat[0]", &i64_tlv(&[10]));
        eng.set_tlv("/data/scat[1]", &i64_tlv(&[20]));
        eng.set_tlv("/data/scat[2]", &i64_tlv(&[30]));

        let j = build_value(&eng, "/data").to_string();
        assert_eq!(
            j,
            r#"{"active":true,"age":42,"cont":[10,20,30],"grp":{"c":1,"d":2},"name":"alice","scat":[10,20,30],"score":3.14}"#
        );

        let v: Value = serde_json::from_str(&j).unwrap();
        write_map(&eng, "/data2", &v);
        let (kind, raw, _) = parse_tlv(&eng.get_tlv("/data2/age"));
        assert_eq!(kind, "int64");
        assert_eq!(read_int(&raw), 42);
        assert_eq!(eng.get_kv("/data2/name"), "alice");
        let (_, raw, _) = parse_tlv(&eng.get_tlv("/data2/cont"));
        assert_eq!(read_int(&raw[0..8]), 10);
        assert_eq!(read_int(&raw[16..24]), 30);
    }
}
