//! JSON serialization for compiled Inklang scripts.
//!
//! This module handles serialization of the compiled Chunk to JSON format
//! that matches the Kotlin ChunkSerializer exactly.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::chunk::{Chunk, ClassInfo, CstNodeEntry, FunctionDefaults};
use super::value::Value;

// ---------------------------------------------------------------------------
// SerialCstNode - matches Kotlin CstNode sealed class with discriminator "t"
// ---------------------------------------------------------------------------

/// Serializable CST node that matches the Kotlin CstNode sealed class.
/// The discriminator field is "t" with short names matching the Kotlin @SerialName.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum SerialCstNode {
    #[serde(rename = "decl")]
    Declaration {
        keyword: String,
        name: String,
        body: Vec<SerialCstNode>,
    },
    #[serde(rename = "rule")]
    RuleMatch {
        #[serde(rename = "ruleName")]
        rule_name: String,
        children: Vec<SerialCstNode>,
    },
    #[serde(rename = "kw")]
    Keyword { value: String },
    #[serde(rename = "fnblk")]
    FunctionBlock {
        #[serde(rename = "funcIdx")]
        func_idx: usize,
    },
}

impl SerialCstNode {
    pub fn from_entry(entry: &CstNodeEntry) -> Self {
        match entry {
            CstNodeEntry::Declaration { keyword, name, body } => SerialCstNode::Declaration {
                keyword: keyword.clone(),
                name: name.clone(),
                body: body.iter().map(SerialCstNode::from_entry).collect(),
            },
            CstNodeEntry::RuleMatch { rule_name, children } => SerialCstNode::RuleMatch {
                rule_name: rule_name.clone(),
                children: children.iter().map(SerialCstNode::from_entry).collect(),
            },
            CstNodeEntry::Keyword { value } => SerialCstNode::Keyword { value: value.clone() },
            CstNodeEntry::FunctionBlock { func_idx } => SerialCstNode::FunctionBlock { func_idx: *func_idx },
        }
    }
}

// ---------------------------------------------------------------------------
// SerialValue - matches Kotlin SerialValue exactly
// ---------------------------------------------------------------------------

/// Serializable value types (compile-time constants only).
/// MUST match Kotlin SerialValue exactly with serde tag="t".
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum SerialValue {
    #[serde(rename = "null")]
    Null,
    #[serde(rename = "bool")]
    Bool { v: bool },
    #[serde(rename = "int")]
    Int { v: i64 },
    #[serde(rename = "float")]
    Float { v: f32 },
    #[serde(rename = "double")]
    Double { v: f64 },
    #[serde(rename = "string")]
    String { v: String },
    #[serde(rename = "event")]
    Event { name: String, params: Vec<Vec<String>> },
}

impl SerialValue {
    pub fn from_value(value: &Value) -> Self {
        match value {
            Value::Null => SerialValue::Null,
            Value::Boolean(true) => SerialValue::Bool { v: true },
            Value::Boolean(false) => SerialValue::Bool { v: false },
            Value::Int(v) => SerialValue::Int { v: *v },
            Value::Float(v) => SerialValue::Float { v: *v },
            Value::Double(v) => SerialValue::Double { v: *v },
            Value::String(s) => SerialValue::String { v: s.clone() },
            Value::EventInfo { name, params } => {
                SerialValue::Event {
                    name: name.clone(),
                    params: params.iter().map(|(n, t)| vec![n.clone(), t.clone()]).collect(),
                }
            }
        }
    }

    pub fn to_value(&self) -> Value {
        match self {
            SerialValue::Null => Value::Null,
            SerialValue::Bool { v } => Value::Boolean(*v),
            SerialValue::Int { v } => Value::Int(*v),
            SerialValue::Float { v } => Value::Float(*v),
            SerialValue::Double { v } => Value::Double(*v),
            SerialValue::String { v } => Value::String(v.clone()),
            SerialValue::Event { name, params } => {
                Value::EventInfo {
                    name: name.clone(),
                    params: params.iter().map(|p| (p[0].clone(), p[1].clone())).collect(),
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// SerialUpvalue
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialUpvalue {
    pub count: usize,
    pub regs: Vec<usize>,
}

// ---------------------------------------------------------------------------
// SerialClassInfo - matches Kotlin SerialClassInfo
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialClassInfo {
    pub name: String,
    #[serde(rename = "superClass")]
    pub super_class: Option<String>,
    pub methods: HashMap<String, usize>,
}

impl From<&ClassInfo> for SerialClassInfo {
    fn from(info: &ClassInfo) -> Self {
        SerialClassInfo {
            name: info.name.clone(),
            super_class: info.super_class.clone(),
            methods: info.methods.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// SerialChunk - matches Kotlin SerialChunk
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialChunk {
    pub code: Vec<i32>,
    pub constants: Vec<SerialValue>,
    pub strings: Vec<String>,
    pub functions: Vec<SerialChunk>,
    #[serde(rename = "classes")]
    pub classes: Vec<SerialClassInfo>,
    #[serde(rename = "functionDefaults")]
    pub function_defaults: Vec<Vec<Option<usize>>>,
    #[serde(rename = "functionUpvalues")]
    pub function_upvalues: HashMap<String, SerialUpvalue>,
    #[serde(rename = "spillSlotCount")]
    pub spill_slot_count: usize,
    #[serde(rename = "cstTable")]
    pub cst_table: Vec<SerialCstNode>,
}

impl SerialChunk {
    pub fn from_chunk(chunk: &Chunk) -> Self {
        SerialChunk {
            code: chunk.code.clone(),
            constants: chunk.constants.iter().map(SerialValue::from_value).collect(),
            strings: chunk.strings.clone(),
            functions: chunk.functions.iter().map(|f| SerialChunk::from_chunk(f)).collect(),
            classes: chunk.classes.iter().map(SerialClassInfo::from).collect(),
            function_defaults: chunk.function_defaults.iter().map(|fd| fd.default_chunks.clone()).collect(),
            function_upvalues: chunk.function_upvalues.iter()
                .map(|(k, (count, regs))| (k.to_string(), SerialUpvalue { count: *count, regs: regs.clone() }))
                .collect(),
            spill_slot_count: chunk.spill_slot_count,
            cst_table: chunk.cst_table.iter().map(SerialCstNode::from_entry).collect(),
        }
    }

    pub fn to_chunk(&self) -> Result<Chunk, crate::inklang::error::Error> {
        let mut chunk = Chunk::new();
        chunk.code = self.code.clone();
        chunk.constants = self.constants.iter().map(SerialValue::to_value).collect();
        chunk.strings = self.strings.clone();
        chunk.functions = self.functions.iter().map(|f| Box::new(f.to_chunk().unwrap())).collect();
        chunk.classes = self.classes.iter().map(|c| ClassInfo {
            name: c.name.clone(),
            super_class: c.super_class.clone(),
            methods: c.methods.clone(),
        }).collect();
        chunk.function_defaults = self.function_defaults.iter().map(|fd| FunctionDefaults { default_chunks: fd.clone() }).collect();
        for (k, v) in &self.function_upvalues {
            let key = k.parse::<usize>().map_err(|_| crate::inklang::error::Error::Compile(format!("Invalid upvalue key: {}", k)))?;
            chunk.function_upvalues.insert(key, (v.count, v.regs.clone()));
        }
        chunk.spill_slot_count = self.spill_slot_count;
        // cst_table is not deserialized (empty in Rust)
        Ok(chunk)
    }
}

// ---------------------------------------------------------------------------
// SerialConfigField - matches Kotlin SerialConfigField
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialConfigField {
    pub name: String,
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(rename = "defaultValue")]
    pub default_value: Option<SerialValue>,
}

// ---------------------------------------------------------------------------
// SerialScript - matches Kotlin SerialScript
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialScript {
    pub name: String,
    pub chunk: SerialChunk,
    #[serde(rename = "configDefinitions")]
    pub config_definitions: HashMap<String, Vec<SerialConfigField>>,
}

impl SerialScript {
    pub fn from_chunk(name: &str, chunk: &Chunk) -> Self {
        SerialScript {
            name: name.to_string(),
            chunk: SerialChunk::from_chunk(chunk),
            config_definitions: HashMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Serialization helpers
// ---------------------------------------------------------------------------

/// Serialize a SerialScript to JSON string.
pub fn serialize(script: &SerialScript) -> String {
    serde_json::to_string(script).expect("Failed to serialize SerialScript")
}

/// Deserialize a JSON string to SerialScript.
pub fn deserialize(json: &str) -> SerialScript {
    serde_json::from_str(json).expect("Failed to deserialize SerialScript")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serial_value_null() {
        let val = SerialValue::from_value(&Value::Null);
        assert!(matches!(val, SerialValue::Null));

        let json = serde_json::to_string(&val).unwrap();
        assert_eq!(json, r#"{"t":"null"}"#);
    }

    #[test]
    fn test_serial_value_bool() {
        let val = SerialValue::from_value(&Value::Boolean(true));
        let json = serde_json::to_string(&val).unwrap();
        assert_eq!(json, r#"{"t":"bool","v":true}"#);

        let val_false = SerialValue::from_value(&Value::Boolean(false));
        let json_false = serde_json::to_string(&val_false).unwrap();
        assert_eq!(json_false, r#"{"t":"bool","v":false}"#);
    }

    #[test]
    fn test_serial_value_int() {
        let val = SerialValue::from_value(&Value::Int(42));
        let json = serde_json::to_string(&val).unwrap();
        assert_eq!(json, r#"{"t":"int","v":42}"#);
    }

    #[test]
    fn test_serial_value_float() {
        let val = SerialValue::from_value(&Value::Float(3.14));
        let json = serde_json::to_string(&val).unwrap();
        assert_eq!(json, r#"{"t":"float","v":3.14}"#);
    }

    #[test]
    fn test_serial_value_double() {
        let val = SerialValue::from_value(&Value::Double(3.14159));
        let json = serde_json::to_string(&val).unwrap();
        assert_eq!(json, r#"{"t":"double","v":3.14159}"#);
    }

    #[test]
    fn test_serial_value_string() {
        let val = SerialValue::from_value(&Value::String("hello".to_string()));
        let json = serde_json::to_string(&val).unwrap();
        assert_eq!(json, r#"{"t":"string","v":"hello"}"#);
    }

    #[test]
    fn test_serial_value_event() {
        let val = SerialValue::from_value(&Value::EventInfo {
            name: "player_join".to_string(),
            params: vec![("player".to_string(), "Player".to_string())],
        });
        let json = serde_json::to_string(&val).unwrap();
        assert_eq!(json, r#"{"t":"event","name":"player_join","params":[["player","Player"]]}"#);
    }

    #[test]
    fn test_roundtrip_value() {
        let original = Value::Int(42);
        let serial = SerialValue::from_value(&original);
        let json = serde_json::to_string(&serial).unwrap();
        let deserial: SerialValue = serde_json::from_str(&json).unwrap();
        let recovered = deserial.to_value();
        assert_eq!(original, recovered);
    }

    #[test]
    fn test_serial_chunk_basic() {
        let mut chunk = Chunk::new();
        chunk.code = vec![0x00000000, 0x01010001];
        chunk.constants = vec![Value::Int(1), Value::String("test".to_string())];
        chunk.strings = vec!["test".to_string()];
        chunk.spill_slot_count = 0;

        let serial = SerialChunk::from_chunk(&chunk);
        let json = serde_json::to_string(&serial).unwrap();

        // Just verify the JSON contains expected structure
        assert!(json.contains(r#""code":"#));
        assert!(json.contains(r#""constants":["#));
        assert!(json.contains(r#""strings":["#));
    }

    #[test]
    fn test_serial_script() {
        let mut chunk = Chunk::new();
        chunk.code = vec![];

        let script = SerialScript::from_chunk("test_script", &chunk);
        let json = serde_json::to_string(&script).unwrap();

        assert!(json.contains(r#""name":"test_script""#));
    }

    #[test]
    fn test_serialize_deserialize_roundtrip() {
        let mut chunk = Chunk::new();
        chunk.code = vec![42];
        chunk.constants = vec![Value::Int(100), Value::String("hello".to_string())];
        chunk.strings = vec!["hello".to_string()];
        chunk.spill_slot_count = 2;

        let script = SerialScript::from_chunk("roundtrip_test", &chunk);
        let json = serialize(&script);
        let recovered = deserialize(&json);

        assert_eq!(recovered.name, "roundtrip_test");
        assert_eq!(recovered.chunk.code, vec![42]);
        assert_eq!(recovered.chunk.constants.len(), 2);
        assert_eq!(recovered.chunk.spill_slot_count, 2);
    }
}
