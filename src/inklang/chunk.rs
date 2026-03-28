//! Bytecode chunk structure for the Inklang VM.
//!
//! A Chunk is a compiled function or script body containing packed bytecode
//! instructions and associated data (constants, strings, nested functions, etc).

use std::collections::HashMap;

use super::value::Value;

/// A node in the concrete syntax tree, stored in the chunk's cst_table.
/// The VM reads these at runtime to dispatch grammar declarations to PackageBridge.
#[derive(Debug, Clone)]
pub enum CstNodeEntry {
    Declaration {
        keyword: String,
        name: String,
        body: Vec<CstNodeEntry>,
    },
    RuleMatch {
        rule_name: String,
        children: Vec<CstNodeEntry>,
    },
    Keyword {
        value: String,
    },
    FunctionBlock {
        func_idx: usize,
    },
}

/// Bytecode operations - MUST match Kotlin OpCode values exactly.
///
/// # Bit Layout (32-bit word)
/// | bits 0-7  | bits 8-11  | bits 12-15 | bits 16-19 | bits 20-31 |
/// | opcode    | dst (4-bit)| src1(4-bit)| src2(4-bit)| immediate  |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpCode {
    LoadImm = 0x00,
    Pop = 0x01,
    LoadGlobal = 0x05,
    StoreGlobal = 0x06,
    Move = 0x07,
    Add = 0x08,
    Sub = 0x09,
    Mul = 0x0A,
    Div = 0x0B,
    Neg = 0x0C,
    Not = 0x0D,
    Eq = 0x0E,
    Neq = 0x0F,
    Lt = 0x10,
    Lte = 0x11,
    Gt = 0x12,
    Gte = 0x13,
    Jump = 0x14,
    JumpIfFalse = 0x15,
    LoadFunc = 0x16,
    Call = 0x17,
    Return = 0x18,
    Break = 0x19,
    Next = 0x1A,
    Mod = 0x1B,
    PushArg = 0x1C,
    GetField = 0x1D,
    SetField = 0x1E,
    NewInstance = 0x1F,
    IsType = 0x20,
    NewArray = 0x21,
    GetIndex = 0x22,
    SetIndex = 0x23,
    Range = 0x24,
    BuildClass = 0x25,
    Spill = 0x26,
    Unspill = 0x27,
    Pow = 0x28,
    Has = 0x29,
    Throw = 0x2A,
    RegisterEvent = 0x2B,
    AsyncCall = 0x2C,
    Await = 0x2D,
    Spawn = 0x2E,
    SpawnVirtual = 0x2F,
    GetUpvalue = 0x30,
    CallHandler = 0x31,
}

#[derive(Debug, Clone)]
pub struct ClassInfo {
    pub name: String,
    pub super_class: Option<String>,
    pub methods: HashMap<String, usize>,
}

#[derive(Debug, Clone)]
pub struct FunctionDefaults {
    pub default_chunks: Vec<Option<usize>>,
}

#[derive(Debug, Clone)]
pub struct Chunk {
    pub code: Vec<i32>,
    pub constants: Vec<Value>,
    pub strings: Vec<String>,
    pub functions: Vec<Box<Chunk>>,
    pub classes: Vec<ClassInfo>,
    pub function_defaults: Vec<FunctionDefaults>,
    pub function_upvalues: HashMap<usize, (usize, Vec<usize>)>,
    pub spill_slot_count: usize,
    pub cst_table: Vec<CstNodeEntry>,
}

impl Chunk {
    pub fn new() -> Self {
        Self {
            code: Vec::new(),
            constants: Vec::new(),
            strings: Vec::new(),
            functions: Vec::new(),
            classes: Vec::new(),
            function_defaults: Vec::new(),
            function_upvalues: HashMap::new(),
            spill_slot_count: 0,
            cst_table: Vec::new(),
        }
    }

    pub fn add_constant(&mut self, value: Value) -> usize {
        let idx = self.constants.len();
        self.constants.push(value);
        idx
    }

    pub fn add_string(&mut self, s: &str) -> usize {
        if let Some(idx) = self.strings.iter().position(|x| x == s) {
            return idx;
        }
        let idx = self.strings.len();
        self.strings.push(s.to_string());
        idx
    }

    pub fn write(&mut self, opcode: OpCode, dst: usize, src1: usize, src2: usize, imm: usize) {
        let word = ((opcode as i32) & 0xFF) |
            ((dst as i32) & 0x0F) << 8 |
            ((src1 as i32) & 0x0F) << 12 |
            ((src2 as i32) & 0x0F) << 16 |
            ((imm as i32) & 0xFFF) << 20;
        self.code.push(word);
    }

    pub fn disassemble(&self) {
        println!("Constants: {:?}", self.constants);
        println!("Strings: {:?}", self.strings);
        for (idx, word) in self.code.iter().enumerate() {
            let opcode = word & 0xFF;
            let dst = (word >> 8) & 0x0F;
            let src1 = (word >> 12) & 0x0F;
            let src2 = (word >> 16) & 0x0F;
            let imm = (word >> 20) & 0xFFF;
            println!("{}: word={} opcode={} dst=r{} src1=r{} src2=r{} imm={}", idx, word, opcode, dst, src1, src2, imm);
        }
    }
}

impl Default for Chunk {
    fn default() -> Self {
        Self::new()
    }
}
