//! Global Value Numbering (GVN) pass.
//!
//! GVN detects when two expressions compute the same value and eliminates
//! redundant computations by reusing previously computed values.
//!
//! This implementation works per-block (intra-block GVN) for correctness.
//! In SSA form, each value is defined exactly once within a block, making
//! the hash-based approach straightforward and safe.

use super::super::block::{SsaBlock, SsaInstr};
use super::super::function::SsaFunction;
use super::super::value::SsaValue;
use super::super::SsaOptPass;
use super::SsaOptResult;
use crate::inklang::token::TokenType;
use std::collections::HashMap;

/// Global Value Numbering pass.
pub struct SsaGlobalValueNumberingPass {
    /// Current SSA function being processed.
    current_func: Option<SsaFunction>,
}

impl SsaGlobalValueNumberingPass {
    pub fn new() -> Self {
        SsaGlobalValueNumberingPass {
            current_func: None,
        }
    }

    /// Process a single block for GVN.
    /// Returns true if any changes were made.
    fn process_block(&self, block: &mut SsaBlock) -> bool {
        // Value table for this block: maps expression hash -> canonical SsaValue
        let mut value_table: HashMap<ExprHash, SsaValue> = HashMap::new();

        // Track if we've seen a side-effecting instruction since a given hash
        // This is conservative: after a side effect, we invalidate matching hashes
        let mut invalidated_by_side_effect: HashMap<ExprHash, bool> = HashMap::new();

        let mut changed = false;

        // Process all instructions in order
        let mut i = 0;
        while i < block.instrs.len() {
            let instr = &block.instrs[i];

            // Track side-effecting instructions - they invalidate matching hashes
            if self.has_side_effects(instr) {
                if let Some(hash) = self.compute_hash(instr) {
                    invalidated_by_side_effect.insert(hash, true);
                }
            }

            // Try to GVN this instruction
            if let Some(replacement) =
                self.try_gvn(instr, &value_table, &invalidated_by_side_effect)
            {
                block.instrs[i] = replacement;
                changed = true;
                // Don't add to value table - the replacement (Move) will be processed
                // and will add the canonical value instead
            } else {
                // No replacement - add this instruction's result to the value table
                if let Some(defined) = instr.defined_value() {
                    if let Some(hash) = self.compute_hash(instr) {
                        if !invalidated_by_side_effect.contains_key(&hash) {
                            // First occurrence wins (canonical representation)
                            if !value_table.contains_key(&hash) {
                                value_table.insert(hash, defined);
                            }
                        }
                    }
                }
            }
            i += 1;
        }

        changed
    }

    /// Try to find an existing canonical value for this expression.
    /// Returns a Move instruction if we can reuse a value, null otherwise.
    fn try_gvn(
        &self,
        instr: &SsaInstr,
        value_table: &HashMap<ExprHash, SsaValue>,
        invalidated: &HashMap<ExprHash, bool>,
    ) -> Option<SsaInstr> {
        // Can only GVN instructions that define a value
        let defined = instr.defined_value()?;

        // Skip instructions with side effects
        if self.has_side_effects(instr) {
            return None;
        }

        let hash = self.compute_hash(instr)?;

        // Can't reuse if invalidated by side effect
        if invalidated.contains_key(&hash) {
            return None;
        }

        // Check if we've seen this expression before in this block
        let canonical_value = value_table.get(&hash)?;

        // Don't replace with self
        if *canonical_value == defined {
            return None;
        }

        Some(SsaInstr::Move {
            defined_value: defined,
            src: *canonical_value,
        })
    }

    /// Compute a hash key for an instruction based on its opcode and operands.
    /// Returns None for instructions we can't GVN.
    fn compute_hash(&self, instr: &SsaInstr) -> Option<ExprHash> {
        match instr {
            SsaInstr::LoadImm { const_index, .. } => {
                Some(ExprHash::Const(*const_index))
            }
            SsaInstr::UnaryOp { op, src, .. } => Some(ExprHash::Unary(*op, *src)),
            SsaInstr::BinaryOp { op, src1, src2, .. } => {
                Some(ExprHash::Binary(*op, *src1, *src2))
            }
            SsaInstr::Move { src, .. } => Some(ExprHash::Move(*src)),
            SsaInstr::GetIndex { obj, index, .. } => Some(ExprHash::GetIndex(*obj, *index)),
            SsaInstr::GetField { obj, name, .. } => Some(ExprHash::GetField(*obj, name.clone())),
            SsaInstr::IsType { src, type_name, .. } => {
                Some(ExprHash::IsType(*src, type_name.clone()))
            }
            // Can't GVN: calls, loads, stores, allocations
            _ => None,
        }
    }

    /// Check if an instruction has side effects that prevent GVN.
    /// Instructions with side effects can't be reordered or eliminated.
    fn has_side_effects(&self, instr: &SsaInstr) -> bool {
        match instr {
            SsaInstr::Call { .. } => true,         // Could call arbitrary functions
            SsaInstr::StoreGlobal { .. } => true,
            SsaInstr::SetIndex { .. } => true,
            SsaInstr::SetField { .. } => true,
            SsaInstr::NewArray { .. } => true,    // Allocates new memory
            SsaInstr::NewInstance { .. } => true,
            SsaInstr::LoadGlobal { .. } => true,  // Could alias with stores
            SsaInstr::LoadFunc { .. } => false,   // Purely loads a function reference
            SsaInstr::LoadClass { .. } => false,
            SsaInstr::Break | SsaInstr::Next => true, // Control flow effects
            SsaInstr::Return { .. } => true,
            _ => false,
        }
    }
}

impl Default for SsaGlobalValueNumberingPass {
    fn default() -> Self {
        Self::new()
    }
}

impl SsaOptPass for SsaGlobalValueNumberingPass {
    fn name(&self) -> &str {
        "GlobalValueNumbering"
    }

    fn run(&mut self, mut ssa_func: SsaFunction) -> SsaOptResult {
        self.current_func = Some(ssa_func.clone());

        let mut changed = false;

        for block in &mut ssa_func.blocks {
            if self.process_block(block) {
                changed = true;
            }
        }

        SsaOptResult {
            func: ssa_func,
            changed,
        }
    }
}

/// Expression hash types - captures the essential structure of each expression
/// for equality comparison in GVN.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ExprHash {
    /// Use constant index for hashing - relies on constants table being same
    Const(usize),
    Move(SsaValue),
    Unary(TokenType, SsaValue),
    Binary(TokenType, SsaValue, SsaValue),
    GetIndex(SsaValue, SsaValue),
    GetField(SsaValue, String),
    IsType(SsaValue, String),
}

impl std::hash::Hash for ExprHash {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            ExprHash::Const(idx) => {
                0u8.hash(state);
                idx.hash(state);
            }
            ExprHash::Move(v) => {
                1u8.hash(state);
                v.hash(state);
            }
            ExprHash::Unary(op, v) => {
                2u8.hash(state);
                op.hash(state);
                v.hash(state);
            }
            ExprHash::Binary(op, v1, v2) => {
                3u8.hash(state);
                op.hash(state);
                v1.hash(state);
                v2.hash(state);
            }
            ExprHash::GetIndex(v1, v2) => {
                4u8.hash(state);
                v1.hash(state);
                v2.hash(state);
            }
            ExprHash::GetField(v, name) => {
                5u8.hash(state);
                v.hash(state);
                name.hash(state);
            }
            ExprHash::IsType(v, name) => {
                6u8.hash(state);
                v.hash(state);
                name.hash(state);
            }
        }
    }
}
