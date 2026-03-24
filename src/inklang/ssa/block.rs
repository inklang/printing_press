//! SSA basic blocks.
//!
//! A basic block in SSA form contains phi functions at the start,
//! followed by regular SSA instructions.

use super::value::SsaValue;
use super::function::PhiFunction;
use crate::inklang::ir::IrLabel;
use crate::inklang::token::TokenType;
use crate::inklang::value::Value;
use crate::inklang::ir::{DefaultValueInfo, MethodInfo};
use std::collections::HashMap;
use std::fmt;

/// A basic block in SSA form.
/// Contains phi functions at the start, followed by regular SSA instructions.
#[derive(Debug, Clone)]
pub struct SsaBlock {
    /// Unique block ID.
    pub id: usize,
    /// Label for this block (if any).
    pub label: Option<IrLabel>,
    /// Phi functions in this block.
    pub phi_functions: Vec<PhiFunction>,
    /// Instructions in this block.
    pub instrs: Vec<SsaInstr>,
    /// Predecessor block IDs.
    pub predecessors: Vec<usize>,
    /// Successor block IDs.
    pub successors: Vec<usize>,
}

impl SsaBlock {
    /// Create a new SSA block with the given ID and label.
    pub fn new(id: usize, label: Option<IrLabel>) -> Self {
        SsaBlock {
            id,
            label,
            phi_functions: Vec::new(),
            instrs: Vec::new(),
            predecessors: Vec::new(),
            successors: Vec::new(),
        }
    }

    /// Check if this block ends with a terminal instruction.
    pub fn is_terminal(&self) -> bool {
        if let Some(last) = self.instrs.last() {
            matches!(
                last,
                SsaInstr::Return { .. } | SsaInstr::Jump { .. } | SsaInstr::JumpIfFalse { .. } |
                SsaInstr::Break | SsaInstr::Next
            )
        } else {
            self.phi_functions.is_empty()
        }
    }
}

/// SSA-form instruction.
/// Similar to IrInstr but uses SsaValue instead of Int for registers.
/// Each instruction defines exactly one SsaValue (or none for terminators).
#[derive(Debug, Clone)]
pub enum SsaInstr {
    /// Load an immediate constant into a register.
    LoadImm {
        defined_value: SsaValue,
        const_index: usize,
    },

    /// Load a global variable into a register.
    LoadGlobal {
        defined_value: SsaValue,
        name: String,
    },

    /// Store a register value into a global variable.
    StoreGlobal {
        name: String,
        src: SsaValue,
    },

    /// Binary operation.
    BinaryOp {
        defined_value: SsaValue,
        op: TokenType,
        src1: SsaValue,
        src2: SsaValue,
    },

    /// Unary operation.
    UnaryOp {
        defined_value: SsaValue,
        op: TokenType,
        src: SsaValue,
    },

    /// Unconditional jump to a label.
    Jump {
        target: IrLabel,
    },

    /// Conditional jump - jumps if the source register is falsy.
    JumpIfFalse {
        src: SsaValue,
        target: IrLabel,
    },

    /// Define a label (jump target).
    Label {
        label: IrLabel,
    },

    /// Load a function (closure) into a register.
    LoadFunc {
        defined_value: SsaValue,
        name: String,
        arity: usize,
        instrs: Vec<crate::inklang::ir::IrInstr>,
        constants: Vec<Value>,
        default_values: Vec<Option<DefaultValueInfo>>,
        captured_vars: Vec<String>,
        upvalue_regs: Vec<usize>,
    },

    /// Call a function.
    Call {
        defined_value: SsaValue,
        func: SsaValue,
        args: Vec<SsaValue>,
    },

    /// Return from a function.
    Return {
        src: SsaValue,
    },

    /// Move a value from one register to another.
    Move {
        defined_value: SsaValue,
        src: SsaValue,
    },

    /// Index access (array/object).
    GetIndex {
        defined_value: SsaValue,
        obj: SsaValue,
        index: SsaValue,
    },

    /// Index assignment.
    SetIndex {
        obj: SsaValue,
        index: SsaValue,
        src: SsaValue,
    },

    /// Create a new array.
    NewArray {
        defined_value: SsaValue,
        elements: Vec<SsaValue>,
    },

    /// Get a field from an object.
    GetField {
        defined_value: SsaValue,
        obj: SsaValue,
        name: String,
    },

    /// Set a field on an object.
    SetField {
        obj: SsaValue,
        name: String,
        src: SsaValue,
    },

    /// Create a new instance.
    NewInstance {
        defined_value: SsaValue,
        class_reg: SsaValue,
        args: Vec<SsaValue>,
    },

    /// Type check.
    IsType {
        defined_value: SsaValue,
        src: SsaValue,
        type_name: String,
    },

    /// Load a class definition.
    LoadClass {
        defined_value: SsaValue,
        name: String,
        super_class: Option<String>,
        methods: HashMap<String, MethodInfo>,
    },

    /// Break from a loop.
    Break,

    /// Continue to the next iteration of a loop.
    Next,

    /// Call a plugin handler.
    CallHandler {
        handler_name: String,
        block_bodies: Vec<(Vec<crate::inklang::ir::IrInstr>, Vec<Value>)>,
    },
}

impl SsaInstr {
    /// Get the SSA value defined by this instruction, if any.
    pub fn defined_value(&self) -> Option<SsaValue> {
        match self {
            SsaInstr::LoadImm { defined_value, .. } => Some(*defined_value),
            SsaInstr::LoadGlobal { defined_value, .. } => Some(*defined_value),
            SsaInstr::StoreGlobal { .. } => None,
            SsaInstr::BinaryOp { defined_value, .. } => Some(*defined_value),
            SsaInstr::UnaryOp { defined_value, .. } => Some(*defined_value),
            SsaInstr::Jump { .. } => None,
            SsaInstr::JumpIfFalse { .. } => None,
            SsaInstr::Label { .. } => None,
            SsaInstr::LoadFunc { defined_value, .. } => Some(*defined_value),
            SsaInstr::Call { defined_value, .. } => Some(*defined_value),
            SsaInstr::Return { .. } => None,
            SsaInstr::Move { defined_value, .. } => Some(*defined_value),
            SsaInstr::GetIndex { defined_value, .. } => Some(*defined_value),
            SsaInstr::SetIndex { .. } => None,
            SsaInstr::NewArray { defined_value, .. } => Some(*defined_value),
            SsaInstr::GetField { defined_value, .. } => Some(*defined_value),
            SsaInstr::SetField { .. } => None,
            SsaInstr::NewInstance { defined_value, .. } => Some(*defined_value),
            SsaInstr::IsType { defined_value, .. } => Some(*defined_value),
            SsaInstr::LoadClass { defined_value, .. } => Some(*defined_value),
            SsaInstr::Break => None,
            SsaInstr::Next => None,
            SsaInstr::CallHandler { .. } => None,
        }
    }

    /// Get all SSA values used (read) by this instruction.
    pub fn used_values(&self) -> Vec<SsaValue> {
        match self {
            SsaInstr::LoadImm { .. } => vec![],
            SsaInstr::LoadGlobal { .. } => vec![],
            SsaInstr::StoreGlobal { src, .. } => vec![*src],
            SsaInstr::BinaryOp { src1, src2, .. } => vec![*src1, *src2],
            SsaInstr::UnaryOp { src, .. } => vec![*src],
            SsaInstr::Jump { .. } => vec![],
            SsaInstr::JumpIfFalse { src, .. } => vec![*src],
            SsaInstr::Label { .. } => vec![],
            SsaInstr::LoadFunc { .. } => vec![],
            SsaInstr::Call { func, args, .. } => {
                let mut vals = vec![*func];
                vals.extend(args.iter().copied());
                vals
            }
            SsaInstr::Return { src } => vec![*src],
            SsaInstr::Move { src, .. } => vec![*src],
            SsaInstr::GetIndex { obj, index, .. } => vec![*obj, *index],
            SsaInstr::SetIndex { obj, index, src } => vec![*obj, *index, *src],
            SsaInstr::NewArray { elements, .. } => elements.clone(),
            SsaInstr::GetField { obj, .. } => vec![*obj],
            SsaInstr::SetField { obj, src, .. } => vec![*obj, *src],
            SsaInstr::NewInstance { class_reg, args, .. } => {
                let mut vals = vec![*class_reg];
                vals.extend(args.iter().copied());
                vals
            }
            SsaInstr::IsType { src, .. } => vec![*src],
            SsaInstr::LoadClass { .. } => vec![],
            SsaInstr::Break => vec![],
            SsaInstr::Next => vec![],
            SsaInstr::CallHandler { .. } => vec![],
        }
    }
}

impl fmt::Display for SsaInstr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SsaInstr::LoadImm { defined_value, const_index } => {
                write!(f, "{} = LoadImm #{}", defined_value, const_index)
            }
            SsaInstr::LoadGlobal { defined_value, name } => {
                write!(f, "{} = LoadGlobal {}", defined_value, name)
            }
            SsaInstr::StoreGlobal { name, src } => {
                write!(f, "StoreGlobal {}, {}", name, src)
            }
            SsaInstr::BinaryOp { defined_value, op, src1, src2 } => {
                write!(f, "{} = {} {:?} {}", defined_value, src1, op, src2)
            }
            SsaInstr::UnaryOp { defined_value, op, src } => {
                write!(f, "{} = {:?} {}", defined_value, op, src)
            }
            SsaInstr::Jump { target } => {
                write!(f, "Jump L{}", target.0)
            }
            SsaInstr::JumpIfFalse { src, target } => {
                write!(f, "JumpIfFalse {}, L{}", src, target.0)
            }
            SsaInstr::Label { label } => {
                write!(f, "Label L{}", label.0)
            }
            SsaInstr::LoadFunc { defined_value, name, arity, .. } => {
                write!(f, "{} = LoadFunc {}/{}", defined_value, name, arity)
            }
            SsaInstr::Call { defined_value, func, args } => {
                write!(f, "{} = Call {}({})", defined_value, func, args.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", "))
            }
            SsaInstr::Return { src } => {
                write!(f, "Return {}", src)
            }
            SsaInstr::Move { defined_value, src } => {
                write!(f, "{} = Move {}", defined_value, src)
            }
            SsaInstr::GetIndex { defined_value, obj, index } => {
                write!(f, "{} = {}[{}]", defined_value, obj, index)
            }
            SsaInstr::SetIndex { obj, index, src } => {
                write!(f, "{}[{}] = {}", obj, index, src)
            }
            SsaInstr::NewArray { defined_value, elements } => {
                write!(f, "{} = NewArray [{}]", defined_value, elements.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", "))
            }
            SsaInstr::GetField { defined_value, obj, name } => {
                write!(f, "{} = {}.{}", defined_value, obj, name)
            }
            SsaInstr::SetField { obj, name, src } => {
                write!(f, "{}.{} = {}", obj, name, src)
            }
            SsaInstr::NewInstance { defined_value, class_reg, args } => {
                write!(f, "{} = NewInstance {}({})", defined_value, class_reg, args.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", "))
            }
            SsaInstr::IsType { defined_value, src, type_name } => {
                write!(f, "{} = {} is {}", defined_value, src, type_name)
            }
            SsaInstr::LoadClass { defined_value, name, .. } => {
                write!(f, "{} = LoadClass {}", defined_value, name)
            }
            SsaInstr::Break => write!(f, "Break"),
            SsaInstr::Next => write!(f, "Next"),
            SsaInstr::CallHandler { handler_name, .. } => {
                write!(f, "CallHandler {}", handler_name)
            }
        }
    }
}
