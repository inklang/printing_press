pub mod error;
pub mod token;
pub mod lexer;
pub mod ast;
pub mod value;
pub mod parser;
pub mod constant_fold;
pub mod lowerer;
pub mod ir;
pub mod ssa;
pub mod liveness;
pub mod register_alloc;
pub mod spill_insert;
pub mod codegen;
pub mod chunk;
pub mod serialize;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialScript {
    pub name: String,
    pub constants: Vec<serde_json::Value>,
    pub functions: Vec<serde_json::Value>,
}

pub fn compile(source: &str, name: &str) -> SerialScript {
    todo!()
}
