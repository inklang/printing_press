//! Intermediate Representation (IR) for Inklang bytecode compilation.
//!
//! The IR is a linear, explicit-control-flow representation that is
//! lowered from the AST. It uses labels and jumps for control flow,
//! and explicit registers for values.

use std::collections::HashMap;

use super::token::TokenType;
use super::value::Value;

/// A label in the IR, used for jump targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IrLabel(pub usize);

/// Default value information for a function parameter.
/// Contains the IR instructions and constants needed to evaluate
/// the default value expression at call time.
#[derive(Debug, Clone)]
pub struct DefaultValueInfo {
    pub instrs: Vec<IrInstr>,
    pub constants: Vec<Value>,
}

/// Method information for a class method.
#[derive(Debug, Clone)]
pub struct MethodInfo {
    /// Arity of the method (includes implicit `self` parameter).
    pub arity: usize,
    /// Instructions in the method body.
    pub instrs: Vec<IrInstr>,
    /// Constants used by the method.
    pub constants: Vec<Value>,
    /// Default value information for each parameter.
    pub default_values: Vec<Option<DefaultValueInfo>>,
}

/// IR instructions - the low-level operations emitted by the AST lowerer.
#[derive(Debug, Clone)]
pub enum IrInstr {
    /// Load an immediate constant into a register.
    /// `dst = constants[index]`
    LoadImm { dst: usize, index: usize },

    /// Load a global variable into a register.
    /// `dst = globals[name]`
    LoadGlobal { dst: usize, name: String },

    /// Store a register value into a global variable.
    /// `globals[name] = src`
    StoreGlobal { name: String, src: usize },

    /// Binary operation.
    /// `dst = src1 op src2`
    BinaryOp {
        dst: usize,
        op: TokenType,
        src1: usize,
        src2: usize,
    },

    /// Unary operation.
    /// `dst = op src`
    UnaryOp { dst: usize, op: TokenType, src: usize },

    /// Unconditional jump to a label.
    Jump { target: IrLabel },

    /// Conditional jump - jumps if the source register is falsy.
    JumpIfFalse { src: usize, target: IrLabel },

    /// Define a label (jump target).
    Label { label: IrLabel },

    /// Load a function (closure) into a register.
    LoadFunc {
        dst: usize,
        name: String,
        arity: usize,
        instrs: Vec<IrInstr>,
        constants: Vec<Value>,
        /// Default value IR for each parameter.
        default_values: Vec<Option<DefaultValueInfo>>,
        /// Names of captured variables (closures).
        captured_vars: Vec<String>,
        /// Register indices of captured variables in enclosing scope.
        upvalue_regs: Vec<usize>,
    },

    /// Call a function.
    /// `dst = func(args...)`
    Call {
        dst: usize,
        func: usize,
        args: Vec<usize>,
    },

    /// Return from a function.
    Return { src: usize },

    /// Move a value from one register to another.
    /// `dst = src`
    Move { dst: usize, src: usize },

    /// Index access (array/object).
    /// `dst = obj[index]`
    GetIndex {
        dst: usize,
        obj: usize,
        index: usize,
    },

    /// Index assignment.
    /// `obj[index] = src`
    SetIndex {
        obj: usize,
        index: usize,
        src: usize,
    },

    /// Create a new array.
    /// `dst = [elements...]`
    NewArray { dst: usize, elements: Vec<usize> },

    /// Get a field from an object.
    /// `dst = obj.name`
    GetField {
        dst: usize,
        obj: usize,
        name: String,
    },

    /// Set a field on an object.
    /// `obj.name = src`
    SetField {
        obj: usize,
        name: String,
        src: usize,
    },

    /// Create a new instance.
    /// `dst = Class(args...)`
    NewInstance {
        dst: usize,
        class_reg: usize,
        args: Vec<usize>,
    },

    /// Type check.
    /// `dst = src is typeName`
    IsType {
        dst: usize,
        src: usize,
        type_name: String,
    },

    /// Field existence check (has operator).
    /// `dst = obj has fieldName`
    HasCheck {
        dst: usize,
        obj: usize,
        field_name: String,
    },

    /// Throw an exception.
    Throw { src: usize },

    /// Load a class definition.
    LoadClass {
        dst: usize,
        name: String,
        /// Name of superclass, resolved at runtime from globals.
        super_class: Option<String>,
        /// methodName -> method info.
        methods: HashMap<String, MethodInfo>,
    },

    /// Break from a loop.
    Break,

    /// Continue to the next iteration of a loop.
    Next,

    /// Spill a register value to the stack.
    /// `spills[slot] = regs[src]`
    Spill { slot: usize, src: usize },

    /// Unspill a value from the stack to a register.
    /// `regs[dst] = spills[slot]`
    Unspill { dst: usize, slot: usize },

    /// Get an upvalue (captured variable from enclosing scope).
    /// `dst = upvalues[upvalue_index]`
    GetUpvalue {
        dst: usize,
        upvalue_index: usize,
    },

    /// Register an event handler with the runtime event bus.
    RegisterEventHandler {
        event_name: String,
        handler_func_index: usize,
        event_param_name: String,
        data_param_names: Vec<String>,
    },

    /// Invoke a registered event handler (used at runtime when events fire).
    InvokeEventHandler {
        event_name: String,
        handler_index: usize,
        event_object_reg: usize,
        data_arg_regs: Vec<usize>,
    },

    /// Await a task - suspend until complete, store result in dst.
    AwaitInstr { dst: usize, task: usize },

    /// Spawn a function on a thread pool - store Task in dst.
    SpawnInstr {
        dst: usize,
        func: usize,
        args: Vec<usize>,
        virtual_: bool,
    },

    /// Async call - launch async function, store Task in dst.
    AsyncCallInstr {
        dst: usize,
        func: usize,
        args: Vec<usize>,
    },

    /// Call a plugin handler (grammar declaration dispatch).
    CallHandler {
        /// Declaration keyword, e.g. "player".
        keyword: String,
        /// Declaration instance name, e.g. "Greeter".
        decl_name: String,
        /// One entry per matched scope rule.
        rule_bodies: Vec<RuleBodyIr>,
    },
}

/// Compiled body for one scope rule inside a grammar declaration.
#[derive(Debug, Clone)]
pub struct RuleBodyIr {
    /// Fully-qualified rule name, e.g. "ink.paper/on_join_clause".
    pub rule_name: String,
    /// Leading keyword from the rule definition, e.g. "on_join".
    pub leading_keyword: Option<String>,
    /// IR instructions for the block body.
    pub instrs: Vec<IrInstr>,
    /// Constants used by the block body.
    pub constants: Vec<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inklang::token::TokenType;

    #[test]
    fn test_ir_label() {
        let label = IrLabel(0);
        assert_eq!(label.0, 0);
    }

    #[test]
    fn test_ir_label_eq() {
        let a = IrLabel(1);
        let b = IrLabel(1);
        let c = IrLabel(2);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_load_imm_instr() {
        let instr = IrInstr::LoadImm { dst: 0, index: 5 };
        assert!(matches!(instr, IrInstr::LoadImm { dst: 0, index: 5 }));
    }

    #[test]
    fn test_binary_op_instr() {
        let instr = IrInstr::BinaryOp {
            dst: 0,
            op: TokenType::Plus,
            src1: 1,
            src2: 2,
        };
        assert!(matches!(instr, IrInstr::BinaryOp { dst: 0, op: TokenType::Plus, src1: 1, src2: 2 }));
    }

    #[test]
    fn test_jump_instr() {
        let instr = IrInstr::Jump { target: IrLabel(42) };
        assert!(matches!(instr, IrInstr::Jump { target: IrLabel(42) }));
    }

    #[test]
    fn test_method_info() {
        let info = MethodInfo {
            arity: 2,
            instrs: vec![IrInstr::LoadImm { dst: 0, index: 0 }],
            constants: vec![Value::Int(42)],
            default_values: vec![],
        };
        assert_eq!(info.arity, 2);
        assert_eq!(info.instrs.len(), 1);
    }

    #[test]
    fn test_default_value_info() {
        let info = DefaultValueInfo {
            instrs: vec![IrInstr::LoadImm { dst: 0, index: 0 }],
            constants: vec![Value::Int(10)],
        };
        assert_eq!(info.instrs.len(), 1);
        assert_eq!(info.constants.len(), 1);
    }
}
