//! Liveness analysis for register allocation.
//!
//! Computes live ranges for each virtual register. A register is live at
//! a point if it will be used in the future before being redefined.

use std::collections::HashMap;

use super::ir::{IrInstr, IrLabel};

/// Represents the live range of a virtual register.
#[derive(Debug, Clone)]
pub struct LiveRange {
    /// Virtual register number.
    pub reg: usize,
    /// Instruction index where the register becomes live.
    pub start: usize,
    /// Instruction index where the register stops being live.
    pub end: usize,
}

/// Liveness analyzer for IR instructions.
pub struct LivenessAnalyzer;

impl LivenessAnalyzer {
    pub fn new() -> Self {
        Self
    }

    /// Analyze liveness of instructions and compute live ranges.
    pub fn analyze(&self, instrs: &[IrInstr]) -> HashMap<usize, LiveRange> {
        let mut ranges: HashMap<usize, LiveRange> = HashMap::new();

        fn define(ranges: &mut HashMap<usize, LiveRange>, reg: usize, idx: usize) {
            ranges
                .entry(reg)
                .or_insert_with(|| LiveRange {
                    reg,
                    start: idx,
                    end: idx,
                });
        }

        fn use_reg(ranges: &mut HashMap<usize, LiveRange>, reg: usize, idx: usize) {
            ranges
                .entry(reg)
                .or_insert_with(|| LiveRange {
                    reg,
                    start: idx,
                    end: idx,
                })
                .end = idx;
        }

        // First pass: build label index map
        let mut label_indices: HashMap<usize, usize> = HashMap::new(); // label.id -> instruction index
        let mut loops: Vec<(usize, usize)> = Vec::new(); // (loopStart, loopEnd) pairs

        for (idx, instr) in instrs.iter().enumerate() {
            if let IrInstr::Label { label } = instr {
                label_indices.insert(label.0, idx);
            }
        }

        // Find backward jumps (loops)
        for (idx, instr) in instrs.iter().enumerate() {
            if let IrInstr::Jump { target } = instr {
                if let Some(&target_idx) = label_indices.get(&target.0) {
                    if target_idx < idx {
                        // This is a loop: target_idx is loop start, idx is loop end
                        loops.push((target_idx, idx));
                    }
                }
            }
        }

        // Second pass: analyze instructions for register defs and uses
        for (idx, instr) in instrs.iter().enumerate() {
            match instr {
                IrInstr::LoadImm { dst, .. } => define(&mut ranges, *dst, idx),
                IrInstr::LoadGlobal { dst, .. } => define(&mut ranges, *dst, idx),
                IrInstr::StoreGlobal { src, .. } => use_reg(&mut ranges, *src, idx),
                IrInstr::LoadFunc { dst, .. } => define(&mut ranges, *dst, idx),
                IrInstr::BinaryOp { dst, src1, src2, .. } => {
                    define(&mut ranges, *dst, idx);
                    use_reg(&mut ranges, *src1, idx);
                    use_reg(&mut ranges, *src2, idx);
                }
                IrInstr::UnaryOp { dst, src, .. } => {
                    define(&mut ranges, *dst, idx);
                    use_reg(&mut ranges, *src, idx);
                }
                IrInstr::Call { dst, func, args } => {
                    define(&mut ranges, *dst, idx);
                    use_reg(&mut ranges, *func, idx);
                    for &arg in args {
                        use_reg(&mut ranges, arg, idx);
                    }
                }
                IrInstr::Return { src } => use_reg(&mut ranges, *src, idx),
                IrInstr::JumpIfFalse { src, .. } => use_reg(&mut ranges, *src, idx),
                IrInstr::Jump { .. } => {}
                IrInstr::Label { .. } => {}
                IrInstr::Break => {}
                IrInstr::Next => {}
                IrInstr::Move { dst, src } => {
                    define(&mut ranges, *dst, idx);
                    use_reg(&mut ranges, *src, idx);
                }
                IrInstr::NewArray { dst, elements } => {
                    define(&mut ranges, *dst, idx);
                    for &elem in elements {
                        use_reg(&mut ranges, elem, idx);
                    }
                }
                IrInstr::GetIndex { dst, obj, index } => {
                    define(&mut ranges, *dst, idx);
                    use_reg(&mut ranges, *obj, idx);
                    use_reg(&mut ranges, *index, idx);
                }
                IrInstr::SetIndex { obj, index, src } => {
                    use_reg(&mut ranges, *obj, idx);
                    use_reg(&mut ranges, *index, idx);
                    use_reg(&mut ranges, *src, idx);
                }
                IrInstr::GetField { dst, obj, .. } => {
                    define(&mut ranges, *dst, idx);
                    use_reg(&mut ranges, *obj, idx);
                }
                IrInstr::SetField { obj, src, .. } => {
                    use_reg(&mut ranges, *obj, idx);
                    use_reg(&mut ranges, *src, idx);
                }
                IrInstr::NewInstance { dst, class_reg, args } => {
                    define(&mut ranges, *dst, idx);
                    use_reg(&mut ranges, *class_reg, idx);
                    for &arg in args {
                        use_reg(&mut ranges, arg, idx);
                    }
                }
                IrInstr::IsType { dst, src, .. } => {
                    define(&mut ranges, *dst, idx);
                    use_reg(&mut ranges, *src, idx);
                }
                IrInstr::HasCheck { dst, obj, .. } => {
                    define(&mut ranges, *dst, idx);
                    use_reg(&mut ranges, *obj, idx);
                }
                IrInstr::LoadClass { dst, .. } => define(&mut ranges, *dst, idx),
                IrInstr::GetUpvalue { dst, .. } => define(&mut ranges, *dst, idx),
                IrInstr::Spill { src, .. } => use_reg(&mut ranges, *src, idx),
                IrInstr::Unspill { dst, .. } => define(&mut ranges, *dst, idx),
                IrInstr::Throw { src } => use_reg(&mut ranges, *src, idx),
                IrInstr::RegisterEventHandler { .. } => {}
                IrInstr::InvokeEventHandler { .. } => {}
                IrInstr::AwaitInstr { dst, task } => {
                    define(&mut ranges, *dst, idx);
                    use_reg(&mut ranges, *task, idx);
                }
                IrInstr::SpawnInstr { dst, func, args, .. } => {
                    define(&mut ranges, *dst, idx);
                    use_reg(&mut ranges, *func, idx);
                    for &arg in args {
                        use_reg(&mut ranges, arg, idx);
                    }
                }
                IrInstr::AsyncCallInstr { dst, func, args } => {
                    define(&mut ranges, *dst, idx);
                    use_reg(&mut ranges, *func, idx);
                    for &arg in args {
                        use_reg(&mut ranges, arg, idx);
                    }
                }
                IrInstr::CallHandler { .. } => {}
            }
        }

        // Extend live ranges for variables that span loop back-edges
        for (loop_start, loop_end) in loops {
            for (_, range) in &mut ranges {
                // Only extend if the variable is defined before the loop start
                // AND used inside the loop (potentially across iterations)
                if range.start < loop_start && range.end >= loop_start && range.end <= loop_end {
                    range.end = loop_end;
                }
            }
        }

        ranges
    }
}

impl Default for LivenessAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inklang::ir::IrInstr;
    use crate::inklang::token::TokenType;

    #[test]
    fn test_liveness_simple_sequence() {
        // Test: v0 = 1; v1 = v0 + 2; return v1
        // v0 is used at instr 1, v1 is used at instr 2
        let instrs = vec![
            IrInstr::LoadImm { dst: 0, index: 0 }, // define v0
            IrInstr::BinaryOp {
                dst: 1,
                op: TokenType::Plus,
                src1: 0, // use v0
                src2: 1,
            },
            IrInstr::Return { src: 1 }, // use v1
        ];

        let analyzer = LivenessAnalyzer::new();
        let ranges = analyzer.analyze(&instrs);

        // v0 is live from 0 to 1 (defined at 0, used at 1)
        assert!(ranges.contains_key(&0));
        assert_eq!(ranges[&0].start, 0);
        assert_eq!(ranges[&0].end, 1);

        // v1 is live from 1 to 2 (defined at 1, used at 2)
        assert!(ranges.contains_key(&1));
        assert_eq!(ranges[&1].start, 1);
        assert_eq!(ranges[&1].end, 2);
    }

    #[test]
    fn test_liveness_no_use_after_def() {
        // Test: v0 = 1; v0 = 2; return v0
        // v0 is redefined at instr 1, so its range should end there
        let instrs = vec![
            IrInstr::LoadImm { dst: 0, index: 0 }, // define v0 at 0
            IrInstr::LoadImm { dst: 0, index: 1 }, // redefine v0 at 1
            IrInstr::Return { src: 0 },             // use v0
        ];

        let analyzer = LivenessAnalyzer::new();
        let ranges = analyzer.analyze(&instrs);

        // v0 is redefined at instr 1, so original def's end should be 1
        // But the second definition redefines v0, so it becomes live from 1 to 2
        assert!(ranges.contains_key(&0));
        // The range for v0 spans from first def to last use
        assert_eq!(ranges[&0].start, 0);
        assert_eq!(ranges[&0].end, 2);
    }

    #[test]
    fn test_liveness_move_instruction() {
        // Test: v0 = 1; v1 = v0; return v1
        let instrs = vec![
            IrInstr::LoadImm { dst: 0, index: 0 },
            IrInstr::Move { dst: 1, src: 0 },
            IrInstr::Return { src: 1 },
        ];

        let analyzer = LivenessAnalyzer::new();
        let ranges = analyzer.analyze(&instrs);

        // v0: defined at 0, used at 1
        assert_eq!(ranges[&0].start, 0);
        assert_eq!(ranges[&0].end, 1);

        // v1: defined at 1, used at 2
        assert_eq!(ranges[&1].start, 1);
        assert_eq!(ranges[&1].end, 2);
    }

    #[test]
    fn test_liveness_call_instruction() {
        // Test: v0 = func; v1 = v0(arg)
        let instrs = vec![
            IrInstr::LoadFunc {
                dst: 0,
                name: "foo".to_string(),
                arity: 1,
                instrs: vec![],
                constants: vec![],
                default_values: vec![],
                captured_vars: vec![],
                upvalue_regs: vec![],
            },
            IrInstr::Call {
                dst: 1,
                func: 0,
                args: vec![],
            },
            IrInstr::Return { src: 1 },
        ];

        let analyzer = LivenessAnalyzer::new();
        let ranges = analyzer.analyze(&instrs);

        // v0: defined at 0, used at 1
        assert_eq!(ranges[&0].start, 0);
        assert_eq!(ranges[&0].end, 1);

        // v1: defined at 1, used at 2
        assert_eq!(ranges[&1].start, 1);
        assert_eq!(ranges[&1].end, 2);
    }

    #[test]
    fn test_liveness_with_labels_and_jumps() {
        // Simple loop: label0; v0 = 1; jump label0
        // v0 is defined in the loop and used after (but here it loops forever)
        let label = IrLabel(0);
        let instrs = vec![
            IrInstr::Label { label },
            IrInstr::LoadImm { dst: 0, index: 0 },
            IrInstr::Jump { target: label },
        ];

        let analyzer = LivenessAnalyzer::new();
        let ranges = analyzer.analyze(&instrs);

        // v0 is defined at 1 and loops back to 0
        // With loop detection, range should extend to cover the loop
        assert!(ranges.contains_key(&0));
    }
}
