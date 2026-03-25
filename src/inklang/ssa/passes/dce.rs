//! Dead Code Elimination (DCE) pass.
//!
//! In SSA form, dead code elimination is simpler because each definition
//! has a single reaching use chain. This pass:
//! 1. Marks all values used in terminators, stores, and side-effecting instructions
//! 2. Propagates liveness backwards through the def-use chains
//! 3. Removes instructions that define unused values
//! 4. Removes phi functions for unused values
//! 5. Removes unreachable blocks

use super::super::block::{SsaBlock, SsaInstr};
use super::super::function::SsaFunction;
use super::super::value::SsaValue;
use super::super::SsaOptPass;
use super::SsaOptResult;
use std::collections::HashSet;

/// Dead Code Elimination pass.
pub struct SsaDeadCodeEliminationPass;

impl SsaDeadCodeEliminationPass {
    pub fn new() -> Self {
        SsaDeadCodeEliminationPass
    }

    /// Find all reachable blocks using a worklist algorithm.
    fn find_reachable_blocks(&self, ssa_func: &SsaFunction) -> HashSet<usize> {
        let mut reachable = HashSet::new();
        let mut worklist = vec![ssa_func.entry_block];

        while let Some(block_id) = worklist.pop() {
            if reachable.contains(&block_id) {
                continue;
            }
            reachable.insert(block_id);

            let block = match ssa_func.get_block(block_id) {
                Some(b) => b,
                None => continue,
            };

            // All successors are potentially reachable
            for &succ in &block.successors {
                if !reachable.contains(&succ) {
                    worklist.push(succ);
                }
            }
        }

        reachable
    }

    /// Find all live SSA values using backward liveness analysis.
    fn find_live_values(&self, ssa_func: &SsaFunction, reachable: &HashSet<usize>) -> HashSet<SsaValue> {
        let mut live = HashSet::new();
        let mut changed = true;

        while changed {
            changed = false;

            // Process blocks in reverse order for liveness analysis
            let mut block_ids: Vec<usize> = reachable.iter().copied().collect();
            block_ids.sort_by(|a, b| b.cmp(a)); // Reverse order

            for block_id in &block_ids {
                let block = match ssa_func.get_block(*block_id) {
                    Some(b) => b,
                    None => continue,
                };

                // Process instructions in reverse order
                for instr in block.instrs.iter().rev() {
                    // If instruction has side effects or is a terminator, mark its uses as live
                    if self.has_side_effects(instr) || self.is_terminator(instr) {
                        for used in instr.used_values() {
                            if live.insert(used) {
                                changed = true;
                            }
                        }
                    }

                    // If defined value is live, mark uses as live
                    if let Some(defined) = instr.defined_value() {
                        if live.contains(&defined) {
                            for used in instr.used_values() {
                                if live.insert(used) {
                                    changed = true;
                                }
                            }
                        }
                    }
                }

                // Blocks with no successors represent implicit returns — the last defined
                // value in such a block is the function's return value and must be preserved.
                // Without this, DCE incorrectly removes the only instruction from programs
                // consisting of a single pure expression (e.g. `let x = 5` → one LoadImm).
                if block.successors.is_empty() {
                    if let Some(last_instr) = block.instrs.last() {
                        if let Some(defined) = last_instr.defined_value() {
                            if live.insert(defined) {
                                changed = true;
                            }
                        }
                    }
                }

                // Process phi functions
                for phi in &block.phi_functions {
                    if live.contains(&phi.result) {
                        // Only mark operands from reachable predecessors as live
                        for (&pred_id, operand) in &phi.operands {
                            if reachable.contains(&pred_id) && live.insert(*operand) {
                                changed = true;
                            }
                        }
                    }
                }
            }
        }

        live
    }

    /// Remove dead code (unreachable blocks and unused definitions).
    fn remove_dead_code(
        &self,
        ssa_func: &SsaFunction,
        reachable: &HashSet<usize>,
        live_values: &HashSet<SsaValue>,
    ) -> (Vec<SsaBlock>, bool) {
        let mut changed = false;
        let mut new_blocks = Vec::new();

        for block in &ssa_func.blocks {
            if !reachable.contains(&block.id) {
                changed = true;
                continue; // Remove unreachable block
            }

            let mut new_block = SsaBlock::new(block.id, block.label);
            new_block
                .predecessors
                .extend(block.predecessors.iter().filter(|p| reachable.contains(p)));
            new_block
                .successors
                .extend(block.successors.iter().filter(|s| reachable.contains(s)));

            // Keep only live phi functions
            for phi in &block.phi_functions {
                if live_values.contains(&phi.result) {
                    new_block.phi_functions.push(phi.clone());
                } else {
                    changed = true;
                }
            }

            // Keep only instructions that are live or have side effects
            for instr in &block.instrs {
                let defined = instr.defined_value();
                let should_keep = self.has_side_effects(instr)
                    || self.is_terminator(instr)
                    || defined.is_none()
                    || defined.map(|d| live_values.contains(&d)).unwrap_or(false);

                if should_keep {
                    new_block.instrs.push(instr.clone());
                } else {
                    changed = true;
                }
            }

            new_blocks.push(new_block);
        }

        (new_blocks, changed)
    }

    /// Check if an instruction has side effects.
    fn has_side_effects(&self, instr: &SsaInstr) -> bool {
        match instr {
            SsaInstr::StoreGlobal { .. } => true,
            SsaInstr::SetIndex { .. } => true,
            SsaInstr::SetField { .. } => true,
            SsaInstr::Call { .. } => true, // Conservative: assume all calls have side effects
            SsaInstr::Return { .. } => true,
            SsaInstr::LoadFunc { .. } => true,  // Function definitions should be kept
            SsaInstr::LoadClass { .. } => true,  // Class definitions should be kept
            _ => false,
        }
    }

    /// Check if an instruction is a terminator.
    fn is_terminator(&self, instr: &SsaInstr) -> bool {
        matches!(
            instr,
            SsaInstr::Return { .. }
                | SsaInstr::Jump { .. }
                | SsaInstr::JumpIfFalse { .. }
                | SsaInstr::Break
                | SsaInstr::Next
        )
    }
}

impl Default for SsaDeadCodeEliminationPass {
    fn default() -> Self {
        Self::new()
    }
}

impl SsaOptPass for SsaDeadCodeEliminationPass {
    fn name(&self) -> &str {
        "SsaDeadCodeElimination"
    }

    fn run(&mut self, ssa_func: SsaFunction) -> SsaOptResult {
        // Find reachable blocks
        let reachable = self.find_reachable_blocks(&ssa_func);

        // Find live values (used by reachable code)
        let live_values = self.find_live_values(&ssa_func, &reachable);

        // Remove dead instructions and unreachable blocks
        let (new_blocks, changed) = self.remove_dead_code(&ssa_func, &reachable, &live_values);

        if !changed {
            return SsaOptResult {
                func: ssa_func,
                changed: false,
            };
        }

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
