//! Spill insertion for register allocation.
//!
//! Inserts SPILL and UNSPILL instructions for virtual registers that were
//! spilled during register allocation.

use std::collections::{HashMap, HashSet};

use super::ir::IrInstr;
use super::liveness::LiveRange;
use super::register_alloc::AllocResult;

/// Spill inserter for rewriting instructions with physical registers.
pub struct SpillInserter;

impl SpillInserter {
    pub fn new() -> Self {
        Self
    }

    /// Insert SPILL/UNSPILL instructions for spilled registers.
    ///
    /// Rewrites all virtual register references to physical registers,
    /// inserting UNSPILL before first use and SPILL after last use.
    pub fn insert(
        &self,
        instrs: Vec<IrInstr>,
        alloc: &AllocResult,
        ranges: &HashMap<usize, LiveRange>,
    ) -> Vec<IrInstr> {
        let allocation = &alloc.mapping;
        let spills = &alloc.spills;

        // Precompute which physical registers are live at each instruction index.
        // Spilled virtuals are excluded: they don't occupy a physical register
        // continuously (the physical they were initially assigned was stolen).
        let live_phys_at: Vec<HashSet<usize>> = (0..instrs.len())
            .map(|i| {
                ranges
                    .values()
                    .filter(|range| range.start <= i && i <= range.end && !spills.contains_key(&range.reg))
                    .filter_map(|range| allocation.get(&range.reg).copied())
                    .collect()
            })
            .collect();

        let mut result: Vec<IrInstr> = Vec::with_capacity(instrs.len() * 2);

        for (i, instr) in instrs.into_iter().enumerate() {
            // Per-instruction: track which temp regs have been claimed
            let mut claimed_temps: HashSet<usize> = HashSet::new();

            // Pre and post instructions for this original instruction
            let mut pre_instrs: Vec<IrInstr> = Vec::new();
            let mut post_instrs: Vec<IrInstr> = Vec::new();

            // Pick a temporary physical register that's not live and not claimed
            fn pick_temp(live_phys_at: &HashSet<usize>, claimed_temps: &mut HashSet<usize>, num_regs: usize) -> Option<usize> {
                (0..num_regs)
                    .find(|r| !live_phys_at.contains(r) && !claimed_temps.contains(r))
                    .map(|r| {
                        claimed_temps.insert(r);
                        r
                    })
            }

            // Resolve a source register to a physical register.
            // If spilled: insert Unspill before this instruction.
            fn resolve_src(
                reg: usize,
                spills: &HashMap<usize, usize>,
                allocation: &HashMap<usize, usize>,
                live_phys_at: &HashSet<usize>,
                claimed_temps: &mut HashSet<usize>,
                pre_instrs: &mut Vec<IrInstr>,
                num_regs: usize,
            ) -> usize {
                if let Some(&slot) = spills.get(&reg) {
                    if let Some(temp) = pick_temp(live_phys_at, claimed_temps, num_regs) {
                        pre_instrs.push(IrInstr::Unspill { dst: temp, slot });
                        temp
                    } else {
                        panic!(
                            "Function exceeds register pressure: all {} registers live simultaneously",
                            num_regs
                        )
                    }
                } else {
                    *allocation.get(&reg).unwrap_or_else(|| {
                        panic!("Virtual register v{} has no physical allocation", reg)
                    })
                }
            }

            // Resolve a destination register to a physical register.
            // If spilled: insert Spill after this instruction.
            fn resolve_dst(
                reg: usize,
                spills: &HashMap<usize, usize>,
                allocation: &HashMap<usize, usize>,
                live_phys_at: &HashSet<usize>,
                claimed_temps: &mut HashSet<usize>,
                post_instrs: &mut Vec<IrInstr>,
                num_regs: usize,
            ) -> usize {
                if let Some(&slot) = spills.get(&reg) {
                    if let Some(temp) = pick_temp(live_phys_at, claimed_temps, num_regs) {
                        post_instrs.push(IrInstr::Spill { slot, src: temp });
                        temp
                    } else {
                        panic!(
                            "Function exceeds register pressure: all {} registers live simultaneously",
                            num_regs
                        )
                    }
                } else {
                    *allocation.get(&reg).unwrap_or_else(|| {
                        panic!("Virtual register v{} has no physical allocation", reg)
                    })
                }
            }

            let live_phys = &live_phys_at[i];

            let rewritten: IrInstr = match instr {
                IrInstr::LoadImm { dst, index } => IrInstr::LoadImm {
                    dst: resolve_dst(dst, spills, allocation, live_phys, &mut claimed_temps, &mut post_instrs, 16),
                    index,
                },
                IrInstr::LoadGlobal { dst, name } => IrInstr::LoadGlobal {
                    dst: resolve_dst(dst, spills, allocation, live_phys, &mut claimed_temps, &mut post_instrs, 16),
                    name,
                },
                IrInstr::StoreGlobal { name, src } => IrInstr::StoreGlobal {
                    name,
                    src: resolve_src(src, spills, allocation, live_phys, &mut claimed_temps, &mut pre_instrs, 16),
                },
                IrInstr::Move { dst, src } => IrInstr::Move {
                    src: resolve_src(src, spills, allocation, live_phys, &mut claimed_temps, &mut pre_instrs, 16),
                    dst: resolve_dst(dst, spills, allocation, live_phys, &mut claimed_temps, &mut post_instrs, 16),
                },
                IrInstr::BinaryOp { dst, op, src1, src2 } => IrInstr::BinaryOp {
                    src1: resolve_src(src1, spills, allocation, live_phys, &mut claimed_temps, &mut pre_instrs, 16),
                    src2: resolve_src(src2, spills, allocation, live_phys, &mut claimed_temps, &mut pre_instrs, 16),
                    dst: resolve_dst(dst, spills, allocation, live_phys, &mut claimed_temps, &mut post_instrs, 16),
                    op,
                },
                IrInstr::UnaryOp { dst, op, src } => IrInstr::UnaryOp {
                    src: resolve_src(src, spills, allocation, live_phys, &mut claimed_temps, &mut pre_instrs, 16),
                    dst: resolve_dst(dst, spills, allocation, live_phys, &mut claimed_temps, &mut post_instrs, 16),
                    op,
                },
                IrInstr::Call { dst, func, args } => IrInstr::Call {
                    func: resolve_src(func, spills, allocation, live_phys, &mut claimed_temps, &mut pre_instrs, 16),
                    args: args
                        .into_iter()
                        .map(|arg| resolve_src(arg, spills, allocation, live_phys, &mut claimed_temps, &mut pre_instrs, 16))
                        .collect(),
                    dst: resolve_dst(dst, spills, allocation, live_phys, &mut claimed_temps, &mut post_instrs, 16),
                },
                IrInstr::NewArray { dst, elements } => IrInstr::NewArray {
                    elements: elements
                        .into_iter()
                        .map(|elem| resolve_src(elem, spills, allocation, live_phys, &mut claimed_temps, &mut pre_instrs, 16))
                        .collect(),
                    dst: resolve_dst(dst, spills, allocation, live_phys, &mut claimed_temps, &mut post_instrs, 16),
                },
                IrInstr::GetIndex { dst, obj, index } => IrInstr::GetIndex {
                    obj: resolve_src(obj, spills, allocation, live_phys, &mut claimed_temps, &mut pre_instrs, 16),
                    index: resolve_src(index, spills, allocation, live_phys, &mut claimed_temps, &mut pre_instrs, 16),
                    dst: resolve_dst(dst, spills, allocation, live_phys, &mut claimed_temps, &mut post_instrs, 16),
                },
                IrInstr::SetIndex { obj, index, src } => IrInstr::SetIndex {
                    obj: resolve_src(obj, spills, allocation, live_phys, &mut claimed_temps, &mut pre_instrs, 16),
                    index: resolve_src(index, spills, allocation, live_phys, &mut claimed_temps, &mut pre_instrs, 16),
                    src: resolve_src(src, spills, allocation, live_phys, &mut claimed_temps, &mut pre_instrs, 16),
                },
                IrInstr::GetField { dst, obj, name } => IrInstr::GetField {
                    obj: resolve_src(obj, spills, allocation, live_phys, &mut claimed_temps, &mut pre_instrs, 16),
                    dst: resolve_dst(dst, spills, allocation, live_phys, &mut claimed_temps, &mut post_instrs, 16),
                    name,
                },
                IrInstr::SetField { obj, name, src } => IrInstr::SetField {
                    obj: resolve_src(obj, spills, allocation, live_phys, &mut claimed_temps, &mut pre_instrs, 16),
                    src: resolve_src(src, spills, allocation, live_phys, &mut claimed_temps, &mut pre_instrs, 16),
                    name,
                },
                IrInstr::NewInstance { dst, class_reg, args } => IrInstr::NewInstance {
                    class_reg: resolve_src(class_reg, spills, allocation, live_phys, &mut claimed_temps, &mut pre_instrs, 16),
                    args: args
                        .into_iter()
                        .map(|arg| resolve_src(arg, spills, allocation, live_phys, &mut claimed_temps, &mut pre_instrs, 16))
                        .collect(),
                    dst: resolve_dst(dst, spills, allocation, live_phys, &mut claimed_temps, &mut post_instrs, 16),
                },
                IrInstr::IsType { dst, src, type_name } => IrInstr::IsType {
                    src: resolve_src(src, spills, allocation, live_phys, &mut claimed_temps, &mut pre_instrs, 16),
                    dst: resolve_dst(dst, spills, allocation, live_phys, &mut claimed_temps, &mut post_instrs, 16),
                    type_name,
                },
                IrInstr::HasCheck { dst, obj, field_name } => IrInstr::HasCheck {
                    obj: resolve_src(obj, spills, allocation, live_phys, &mut claimed_temps, &mut pre_instrs, 16),
                    dst: resolve_dst(dst, spills, allocation, live_phys, &mut claimed_temps, &mut post_instrs, 16),
                    field_name,
                },
                IrInstr::LoadClass { dst, name, super_class, methods } => IrInstr::LoadClass {
                    dst: resolve_dst(dst, spills, allocation, live_phys, &mut claimed_temps, &mut post_instrs, 16),
                    name,
                    super_class,
                    methods,
                },
                IrInstr::Return { src } => IrInstr::Return {
                    src: resolve_src(src, spills, allocation, live_phys, &mut claimed_temps, &mut pre_instrs, 16),
                },
                IrInstr::JumpIfFalse { src, target } => IrInstr::JumpIfFalse {
                    src: resolve_src(src, spills, allocation, live_phys, &mut claimed_temps, &mut pre_instrs, 16),
                    target,
                },
                IrInstr::LoadFunc { dst, name, arity, instrs, constants, default_values, captured_vars, upvalue_regs } => IrInstr::LoadFunc {
                    dst: resolve_dst(dst, spills, allocation, live_phys, &mut claimed_temps, &mut post_instrs, 16),
                    name,
                    arity,
                    instrs,
                    constants,
                    default_values,
                    captured_vars,
                    upvalue_regs,
                },
                // These instructions pass through unchanged
                IrInstr::Label { .. } => instr,
                IrInstr::Jump { .. } => instr,
                IrInstr::Break => instr,
                IrInstr::Next => instr,
                IrInstr::Spill { .. } => instr,
                IrInstr::Unspill { .. } => instr,
                IrInstr::Throw { .. } => instr,
                IrInstr::RegisterEventHandler { .. } => instr,
                IrInstr::InvokeEventHandler { .. } => instr,
                IrInstr::GetUpvalue { .. } => instr,
                IrInstr::AwaitInstr { .. } => instr,
                IrInstr::SpawnInstr { .. } => instr,
                IrInstr::AsyncCallInstr { .. } => instr,
                IrInstr::CallHandler { .. } => instr,
            };

            result.extend(pre_instrs);
            result.push(rewritten);
            result.extend(post_instrs);
        }

        result
    }
}

impl Default for SpillInserter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inklang::liveness::LivenessAnalyzer;
    use crate::inklang::register_alloc::RegisterAllocator;
    use crate::inklang::token::TokenType;

    #[test]
    fn test_spill_insert_simple() {
        // Test that non-spilled registers just get their physical register
        let instrs = vec![
            IrInstr::LoadImm { dst: 0, index: 0 },
            IrInstr::LoadImm { dst: 1, index: 1 },
            IrInstr::BinaryOp {
                dst: 2,
                op: TokenType::Plus,
                src1: 0,
                src2: 1,
            },
            IrInstr::Return { src: 2 },
        ];

        let liveness = LivenessAnalyzer::new();
        let ranges = liveness.analyze(&instrs);

        let mut allocator = RegisterAllocator::new();
        let alloc = allocator.allocate(&ranges, 0);

        let inserter = SpillInserter::new();
        let result = inserter.insert(instrs, &alloc, &ranges);

        // No spills should be inserted
        assert!(!result.iter().any(|i| matches!(i, IrInstr::Spill { .. })));
        assert!(!result.iter().any(|i| matches!(i, IrInstr::Unspill { .. })));

        // Instructions should be rewritten with physical registers
        if let IrInstr::LoadImm { dst, .. } = &result[0] {
            assert!(*dst < 16); // Physical register
        }
    }

    #[test]
    fn test_spill_insert_with_spill() {
        // Test a more realistic scenario where spilling is needed.
        // Create 16 non-overlapping computations - each result used immediately.
        // This tests that spill insertion works correctly when we have
        // many sequential computations.

        // v0 = 1; v1 = 2; ... v15 = 16; return v0 + v1 + ... + v15
        let mut instrs = Vec::new();

        // Define 16 registers
        for i in 0..16 {
            instrs.push(IrInstr::LoadImm { dst: i, index: i });
        }

        // Use them all in a sum - all 16 regs will be live simultaneously at the Return
        // This will require some to be spilled since we only have 16 physical regs
        // and parameters also need registers.
        let mut sum_reg = 16;
        for i in 0..16 {
            let next_reg = sum_reg + 1;
            instrs.push(IrInstr::BinaryOp {
                dst: next_reg,
                op: TokenType::Plus,
                src1: sum_reg,
                src2: i,
            });
            sum_reg = next_reg;
        }

        instrs.push(IrInstr::Return { src: sum_reg });

        let liveness = LivenessAnalyzer::new();
        let ranges = liveness.analyze(&instrs);

        let mut allocator = RegisterAllocator::new();
        let alloc = allocator.allocate(&ranges, 0);

        let inserter = SpillInserter::new();
        let result = inserter.insert(instrs, &alloc, &ranges);

        // Result should be valid (no panics) and may contain spills
        // The important thing is it runs without crashing
        assert!(!result.is_empty());
    }

    #[test]
    fn test_spill_insert_preserves_semantics() {
        // Simple case: v0 = 1; return v0
        let instrs = vec![
            IrInstr::LoadImm { dst: 0, index: 0 },
            IrInstr::Return { src: 0 },
        ];

        let liveness = LivenessAnalyzer::new();
        let ranges = liveness.analyze(&instrs);

        let mut allocator = RegisterAllocator::new();
        let alloc = allocator.allocate(&ranges, 0);

        let inserter = SpillInserter::new();
        let result = inserter.insert(instrs.clone(), &alloc, &ranges);

        // Should have same number of instructions (no spills needed)
        assert_eq!(result.len(), 2);

        // The dst of LoadImm should be a valid physical register
        if let IrInstr::LoadImm { dst, .. } = &result[0] {
            assert!(*dst < 16);
        }
    }

    #[test]
    fn test_spill_insert_with_move() {
        let instrs = vec![
            IrInstr::LoadImm { dst: 0, index: 0 },
            IrInstr::Move { dst: 1, src: 0 },
            IrInstr::Return { src: 1 },
        ];

        let liveness = LivenessAnalyzer::new();
        let ranges = liveness.analyze(&instrs);

        let mut allocator = RegisterAllocator::new();
        let alloc = allocator.allocate(&ranges, 0);

        let inserter = SpillInserter::new();
        let result = inserter.insert(instrs, &alloc, &ranges);

        // Should have 3 instructions (no spills needed)
        assert_eq!(result.len(), 3);
    }
}
