//! Sparse Conditional Constant Propagation (SCCP) pass.
//!
//! In SSA form, each variable is defined exactly once, making constant propagation
//! much simpler and more effective. This pass:
//! 1. Tracks which SSA values hold constant values
//! 2. Propagates constants through phi functions where possible
//! 3. Evaluates constant expressions at compile time
//! 4. Eliminates dead branches based on constant conditions

use super::super::block::{SsaBlock, SsaInstr};
use super::super::function::{PhiFunction, SsaFunction};
use super::super::value::SsaValue;
use super::super::SsaOptPass;
use super::SsaOptResult;
use crate::inklang::token::TokenType;
use crate::inklang::value::Value;
use std::collections::HashMap;

/// Sparse Conditional Constant Propagation pass.
pub struct SsaConstantPropagationPass {
    /// Current SSA function being processed.
    current_func: Option<SsaFunction>,
    /// Map from SSA value to its constant value (if known).
    constants: HashMap<SsaValue, Value>,
    /// Set of blocks that are reachable.
    reachable_blocks: HashMap<usize, bool>,
}

impl SsaConstantPropagationPass {
    pub fn new() -> Self {
        SsaConstantPropagationPass {
            current_func: None,
            constants: HashMap::new(),
            reachable_blocks: HashMap::new(),
        }
    }

    /// Evaluate a phi function to see if all operands have the same constant value.
    fn evaluate_phi(&self, phi: &PhiFunction) -> Option<Value> {
        let mut const_value: Option<&Value> = None;
        for (_, operand) in &phi.operands {
            if operand.is_undefined() {
                // Undefined value
                return None;
            }
            let op_const = self.constants.get(operand)?;
            const_value = Some(match const_value {
                None => op_const,
                Some(cv) if *cv == *op_const => op_const,
                _ => return None, // Different constants
            });
        }
        const_value.cloned()
    }

    /// Evaluate a binary operation on constant values.
    fn evaluate_binary(&self, op: TokenType, left: &Value, right: &Value) -> Option<Value> {
        match op {
            TokenType::Plus => Self::evaluate_arith(left, right, |a, b| a + b),
            TokenType::Minus => Self::evaluate_arith(left, right, |a, b| a - b),
            TokenType::Star => Self::evaluate_arith(left, right, |a, b| a * b),
            TokenType::Slash => {
                let r = Self::to_double(right)?;
                if r == 0.0 {
                    return None;
                }
                Self::evaluate_arith(left, right, |a, b| a / b)
            }
            TokenType::Percent => {
                let r = Self::to_double(right)?;
                if r == 0.0 {
                    return None;
                }
                Self::evaluate_arith(left, right, |a, b| a % b)
            }
            TokenType::Lt => Self::evaluate_compare(left, right, |a, b| a < b),
            TokenType::Lte => Self::evaluate_compare(left, right, |a, b| a <= b),
            TokenType::Gt => Self::evaluate_compare(left, right, |a, b| a > b),
            TokenType::Gte => Self::evaluate_compare(left, right, |a, b| a >= b),
            TokenType::EqEq => Self::evaluate_equal(left, right, true),
            TokenType::BangEq => Self::evaluate_equal(left, right, false),
            TokenType::KwAnd => Self::evaluate_logical(left, right, true),
            TokenType::KwOr => Self::evaluate_logical(left, right, false),
            _ => None,
        }
    }

    /// Evaluate a unary operation on a constant value.
    fn evaluate_unary(&self, op: TokenType, value: &Value) -> Option<Value> {
        match op {
            TokenType::Minus => match value {
                Value::Int(v) => Some(Value::Int(-*v)),
                Value::Float(v) => Some(Value::Float(-*v)),
                Value::Double(v) => Some(Value::Double(-*v)),
                _ => None,
            },
            TokenType::Bang | TokenType::KwNot => match value {
                Value::Boolean(v) => Some(Value::Boolean(!*v)),
                _ => None,
            },
            _ => None,
        }
    }

    fn evaluate_arith<F>(left: &Value, right: &Value, op: F) -> Option<Value>
    where
        F: Fn(f64, f64) -> f64,
    {
        let l = Self::to_double(left)?;
        let r = Self::to_double(right)?;
        let result = op(l, r);
        Some(match (left, right) {
            (Value::Int(_), Value::Int(_)) => Value::Int(result as i64),
            (Value::Float(_), _) | (_, Value::Float(_)) => Value::Float(result as f32),
            _ => Value::Double(result),
        })
    }

    fn evaluate_compare<F>(left: &Value, right: &Value, op: F) -> Option<Value>
    where
        F: Fn(f64, f64) -> bool,
    {
        let l = Self::to_double(left)?;
        let r = Self::to_double(right)?;
        Some(Value::Boolean(op(l, r)))
    }

    fn evaluate_equal(left: &Value, right: &Value, equal: bool) -> Option<Value> {
        let result = match (left, right) {
            (Value::Null, Value::Null) => true,
            (Value::Null, _) | (_, Value::Null) => false,
            (Value::Boolean(a), Value::Boolean(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Double(a), Value::Double(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
            _ => return None,
        };
        Some(Value::Boolean(if equal { result } else { !result }))
    }

    fn evaluate_logical(left: &Value, right: &Value, is_and: bool) -> Option<Value> {
        match (left, right) {
            (Value::Boolean(a), Value::Boolean(b)) => {
                Some(Value::Boolean(if is_and { *a && *b } else { *a || *b }))
            }
            _ => None,
        }
    }

    fn to_double(v: &Value) -> Option<f64> {
        match v {
            Value::Int(v) => Some(*v as f64),
            Value::Float(v) => Some(*v as f64),
            Value::Double(v) => Some(*v),
            _ => None,
        }
    }
}

impl Default for SsaConstantPropagationPass {
    fn default() -> Self {
        Self::new()
    }
}

impl SsaOptPass for SsaConstantPropagationPass {
    fn name(&self) -> &str {
        "SsaConstantPropagation"
    }

    fn run(&mut self, ssa_func: SsaFunction) -> SsaOptResult {
        self.current_func = Some(ssa_func.clone());
        self.constants.clear();
        self.reachable_blocks.clear();

        let mut changed = false;

        // Get the entry block
        let entry_block = ssa_func.entry_block;

        // Iterate until fixed point
        let mut iteration_changed = true;
        while iteration_changed {
            iteration_changed = false;

            // Propagate constants through blocks
            for block in &ssa_func.blocks {
                // Skip unreachable blocks (after first iteration)
                if self.reachable_blocks.len() > 0 && !self.reachable_blocks.contains_key(&block.id) {
                    continue;
                }

                // Entry block is always reachable
                if block.id == entry_block {
                    self.reachable_blocks.insert(block.id, true);
                }

                // Process phi functions
                for phi in &block.phi_functions {
                    if let Some(const_value) = self.evaluate_phi(phi) {
                        let old = self.constants.insert(phi.result, const_value.clone());
                        if old != Some(const_value) {
                            iteration_changed = true;
                            changed = true;
                        }
                    }
                }

                // Process instructions
                for instr in &block.instrs {
                    let (new_constants, instr_changed) = self.process_instr(instr);
                    if instr_changed {
                        iteration_changed = true;
                        changed = true;
                    }
                    for (value, const_val) in &new_constants {
                        if !self.constants.contains_key(value) || self.constants.get(value) != Some(const_val) {
                            self.constants.insert(*value, const_val.clone());
                        }
                    }
                }

                // Update reachability based on branch conditions
                let last_instr = block.instrs.last();
                if let Some(SsaInstr::JumpIfFalse { src, target: _ }) = last_instr {
                    let successors = &block.successors;
                    if let Some(Value::Boolean(val)) = self.constants.get(src) {
                        // Find successor blocks
                        if !successors.is_empty() {
                            if *val {
                                // Condition is always true - fall-through is reachable
                                let fall_through = successors.iter().find(|&&s| s == block.id + 1).copied();
                                if let Some(ft) = fall_through {
                                    if self.reachable_blocks.insert(ft, true).is_none() {
                                        iteration_changed = true;
                                    }
                                }
                            } else {
                                // Condition is always false - jump target is reachable
                                let jump_target = successors.iter().find(|&&s| s != block.id + 1).copied();
                                if let Some(jt) = jump_target {
                                    if self.reachable_blocks.insert(jt, true).is_none() {
                                        iteration_changed = true;
                                    }
                                }
                            }
                        }
                    } else {
                        // Unknown condition - all successors are reachable
                        for &succ in successors {
                            if self.reachable_blocks.insert(succ, true).is_none() {
                                iteration_changed = true;
                            }
                        }
                    }
                } else {
                    // Non-conditional or unknown - all successors are reachable
                    for &succ in &block.successors {
                        if self.reachable_blocks.insert(succ, true).is_none() {
                            iteration_changed = true;
                        }
                    }
                }
            }
        }

        // If no constants were discovered, return unchanged
        if !changed {
            return SsaOptResult {
                func: ssa_func,
                changed: false,
            };
        }

        // Apply optimizations: replace constant expressions with LoadImm
        let new_blocks: Vec<SsaBlock> = ssa_func
            .blocks
            .iter()
            .map(|block| {
                if !self.reachable_blocks.contains_key(&block.id) {
                    // Keep unreachable blocks as-is (they'll be removed by DCE)
                    block.clone()
                } else {
                    self.optimize_block(block)
                }
            })
            .collect();

        let new_func = SsaFunction::new(
            new_blocks,
            ssa_func.constants,
            ssa_func.entry_block,
            ssa_func.exit_blocks,
            ssa_func.arity,
        );

        SsaOptResult {
            func: new_func,
            changed: true,
        }
    }
}

impl SsaConstantPropagationPass {
    /// Process an instruction and return any new constant bindings.
    fn process_instr(&self, instr: &SsaInstr) -> (HashMap<SsaValue, Value>, bool) {
        let mut changed = false;
        let mut new_constants = HashMap::new();

        match instr {
            SsaInstr::LoadImm { defined_value, const_index } => {
                if let Some(func) = &self.current_func {
                    if let Some(constant) = func.constants.get(*const_index) {
                        if self.constants.get(defined_value) != Some(constant) {
                            changed = true;
                        }
                        new_constants.insert(*defined_value, constant.clone());
                    }
                }
            }
            SsaInstr::Move { defined_value, src } => {
                if let Some(src_const) = self.constants.get(src) {
                    if self.constants.get(defined_value) != Some(src_const) {
                        changed = true;
                    }
                    new_constants.insert(*defined_value, src_const.clone());
                }
            }
            SsaInstr::BinaryOp { defined_value, op, src1, src2 } => {
                if let (Some(left_const), Some(right_const)) =
                    (self.constants.get(src1), self.constants.get(src2))
                {
                    if let Some(result) = self.evaluate_binary(*op, left_const, right_const) {
                        if self.constants.get(defined_value) != Some(&result) {
                            changed = true;
                        }
                        new_constants.insert(*defined_value, result);
                    }
                }
            }
            SsaInstr::UnaryOp { defined_value, op, src } => {
                if let Some(src_const) = self.constants.get(src) {
                    if let Some(result) = self.evaluate_unary(*op, src_const) {
                        if self.constants.get(defined_value) != Some(&result) {
                            changed = true;
                        }
                        new_constants.insert(*defined_value, result);
                    }
                }
            }
            _ => {}
        }

        (new_constants, changed)
    }

    /// Optimize a block by replacing constant expressions with LoadImm.
    fn optimize_block(&self, block: &SsaBlock) -> SsaBlock {
        let mut new_block = SsaBlock::new(block.id, block.label);

        // Copy phi functions
        new_block.phi_functions.clone_from(&block.phi_functions);
        new_block.predecessors.clone_from(&block.predecessors);
        new_block.successors.clone_from(&block.successors);

        for instr in &block.instrs {
            let optimized = self.optimize_instr(instr);
            new_block.instrs.extend(optimized);
        }

        new_block
    }

    /// Optimize an instruction, potentially replacing it with LoadImm.
    fn optimize_instr(&self, instr: &SsaInstr) -> Vec<SsaInstr> {
        match instr {
            SsaInstr::BinaryOp { defined_value, op, src1, src2 } => {
                if let (Some(left_const), Some(right_const)) =
                    (self.constants.get(src1), self.constants.get(src2))
                {
                    if let Some(result) = self.evaluate_binary(*op, left_const, right_const) {
                        if let Some(func) = &self.current_func {
                            let const_index = func
                                .constants
                                .iter()
                                .position(|c| c == &result)
                                .unwrap_or(func.constants.len());
                            return vec![SsaInstr::LoadImm {
                                defined_value: *defined_value,
                                const_index,
                            }];
                        }
                    }
                }
                vec![instr.clone()]
            }
            SsaInstr::UnaryOp { defined_value, op, src } => {
                if let Some(src_const) = self.constants.get(src) {
                    if let Some(result) = self.evaluate_unary(*op, src_const) {
                        if let Some(func) = &self.current_func {
                            let const_index = func
                                .constants
                                .iter()
                                .position(|c| c == &result)
                                .unwrap_or(func.constants.len());
                            return vec![SsaInstr::LoadImm {
                                defined_value: *defined_value,
                                const_index,
                            }];
                        }
                    }
                }
                vec![instr.clone()]
            }
            SsaInstr::Move { defined_value, src } => {
                if let Some(src_const) = self.constants.get(src) {
                    if let Some(func) = &self.current_func {
                        let const_index = func
                            .constants
                            .iter()
                            .position(|c| c == src_const)
                            .unwrap_or(func.constants.len());
                        return vec![SsaInstr::LoadImm {
                            defined_value: *defined_value,
                            const_index,
                        }];
                    }
                }
                vec![instr.clone()]
            }
            SsaInstr::JumpIfFalse { src, target } => {
                if let Some(Value::Boolean(val)) = self.constants.get(src) {
                    if *val {
                        // Condition is always true - remove the jump (fall through)
                        vec![]
                    } else {
                        // Condition is always false - convert to unconditional jump
                        vec![SsaInstr::Jump { target: *target }]
                    }
                } else {
                    vec![instr.clone()]
                }
            }
            _ => vec![instr.clone()],
        }
    }
}
