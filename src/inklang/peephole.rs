//! Peephole optimization pass.
//!
//! Applied after register allocation and spill insertion, before codegen.
//! Eliminates two patterns:
//! - Self-moves: `Move { dst: r, src: r }` where dst == src
//! - Jump-to-next: `Jump { target: L }` where Label L immediately follows
//!   (with only other Labels in between)

use crate::inklang::ir::IrInstr;

/// Run all peephole optimizations on a linear instruction stream.
/// Returns a new Vec with wasteful instructions removed.
pub fn run(instrs: Vec<IrInstr>) -> Vec<IrInstr> {
    let mut output = Vec::with_capacity(instrs.len());

    let mut i = 0;
    while i < instrs.len() {
        match &instrs[i] {
            // Drop self-moves: Move { dst: r, src: r }
            IrInstr::Move { dst, src } if dst == src => {
                i += 1;
            }
            // Drop unconditional jumps whose target label immediately follows
            // (with only Label instructions in between), but only if the jump
            // itself is reachable.
            //
            // Reachability is detected by checking `output.last()`: because
            // eliminated instructions are never pushed to `output`, the last
            // emitted instruction is always the most recently *kept* instruction.
            // If that is an unconditional Jump, control can never reach the
            // current instruction, so we preserve it rather than silently hiding
            // dead code (dead code elimination is a separate, future pass).
            IrInstr::Jump { target } => {
                let mut is_reachable = true;
                if let Some(last_instr) = output.last() {
                    if matches!(last_instr, IrInstr::Jump { .. }) {
                        is_reachable = false;
                    }
                }

                if is_reachable {
                    let target_label = *target;
                    let mut j = i + 1;
                    let mut found_before_real = false;
                    while j < instrs.len() {
                        match &instrs[j] {
                            IrInstr::Label { label } if *label == target_label => {
                                found_before_real = true;
                                break;
                            }
                            IrInstr::Label { .. } => {
                                j += 1;
                            }
                            _ => break,
                        }
                    }
                    if found_before_real {
                        // Jump is redundant — skip it
                        i += 1;
                    } else {
                        output.push(instrs[i].clone());
                        i += 1;
                    }
                } else {
                    // Jump is unreachable — preserve it (dead code elimination is out of scope)
                    output.push(instrs[i].clone());
                    i += 1;
                }
            }
            _ => {
                output.push(instrs[i].clone());
                i += 1;
            }
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inklang::ir::{IrInstr, IrLabel};

    // --- Self-move elimination ---

    #[test]
    fn test_self_move_dropped() {
        let input = vec![IrInstr::Move { dst: 1, src: 1 }];
        assert!(run(input).is_empty());
    }

    #[test]
    fn test_non_self_move_preserved() {
        let input = vec![IrInstr::Move { dst: 1, src: 2 }];
        let output = run(input);
        assert!(matches!(output[..], [IrInstr::Move { dst: 1, src: 2 }]));
    }

    #[test]
    fn test_multiple_self_moves_all_dropped() {
        let input = vec![
            IrInstr::Move { dst: 1, src: 1 },
            IrInstr::Move { dst: 2, src: 2 },
            IrInstr::Move { dst: 3, src: 3 },
        ];
        assert!(run(input).is_empty());
    }

    #[test]
    fn test_mixed_moves_self_dropped_real_kept() {
        let input = vec![
            IrInstr::Move { dst: 1, src: 1 },
            IrInstr::Move { dst: 2, src: 3 },
            IrInstr::Move { dst: 4, src: 4 },
        ];
        let output = run(input);
        assert_eq!(output.len(), 1);
        assert!(matches!(output[0], IrInstr::Move { dst: 2, src: 3 }));
    }

    #[test]
    fn test_self_move_among_other_instrs() {
        let input = vec![
            IrInstr::LoadImm { dst: 0, index: 0 },
            IrInstr::Move { dst: 1, src: 1 },
            IrInstr::Return { src: 0 },
        ];
        let output = run(input);
        assert_eq!(output.len(), 2);
        assert!(matches!(output[0], IrInstr::LoadImm { dst: 0, index: 0 }));
        assert!(matches!(output[1], IrInstr::Return { src: 0 }));
    }

    // --- Jump-to-next elimination ---

    #[test]
    fn test_jump_to_next_label_dropped() {
        let input = vec![
            IrInstr::Jump { target: IrLabel(0) },
            IrInstr::Label { label: IrLabel(0) },
        ];
        let output = run(input);
        assert_eq!(output.len(), 1);
        assert!(matches!(output[0], IrInstr::Label { label: IrLabel(0) }));
    }

    #[test]
    fn test_jump_with_intervening_labels_dropped() {
        // Jump{L1}, Label{L0}, Label{L1} — L1 is the target, found after only Labels
        let input = vec![
            IrInstr::Jump { target: IrLabel(1) },
            IrInstr::Label { label: IrLabel(0) },
            IrInstr::Label { label: IrLabel(1) },
        ];
        let output = run(input);
        assert_eq!(output.len(), 2);
        assert!(matches!(output[0], IrInstr::Label { label: IrLabel(0) }));
        assert!(matches!(output[1], IrInstr::Label { label: IrLabel(1) }));
    }

    #[test]
    fn test_jump_to_distant_label_preserved() {
        let input = vec![
            IrInstr::Jump { target: IrLabel(0) },
            IrInstr::LoadImm { dst: 0, index: 0 },
            IrInstr::Label { label: IrLabel(0) },
        ];
        let output = run(input.clone());
        assert_eq!(output.len(), 3);
    }

    #[test]
    fn test_jump_if_false_never_eliminated() {
        // JumpIfFalse is conditional — never eliminate even if label is next
        let input = vec![
            IrInstr::JumpIfFalse { src: 0, target: IrLabel(0) },
            IrInstr::Label { label: IrLabel(0) },
        ];
        let output = run(input);
        assert_eq!(output.len(), 2);
        assert!(matches!(output[0], IrInstr::JumpIfFalse { src: 0, target: IrLabel(0) }));
    }

    #[test]
    fn test_jump_dangling_no_target_preserved() {
        let input = vec![IrInstr::Jump { target: IrLabel(99) }];
        let output = run(input);
        assert_eq!(output.len(), 1);
        assert!(matches!(output[0], IrInstr::Jump { target: IrLabel(99) }));
    }

    #[test]
    fn test_two_consecutive_redundant_jumps_both_dropped() {
        let input = vec![
            IrInstr::Jump { target: IrLabel(0) },
            IrInstr::Label { label: IrLabel(0) },
            IrInstr::Jump { target: IrLabel(1) },
            IrInstr::Label { label: IrLabel(1) },
        ];
        let output = run(input);
        assert_eq!(output.len(), 2);
        assert!(matches!(output[0], IrInstr::Label { label: IrLabel(0) }));
        assert!(matches!(output[1], IrInstr::Label { label: IrLabel(1) }));
    }

    #[test]
    fn test_unreachable_jump_both_preserved() {
        // Jump{L0}, Jump{L1}, Label{L1} — second jump is unreachable dead code.
        // Dead code elimination is out of scope; both jumps are preserved.
        let input = vec![
            IrInstr::Jump { target: IrLabel(0) },
            IrInstr::Jump { target: IrLabel(1) },
            IrInstr::Label { label: IrLabel(1) },
        ];
        let output = run(input);
        assert_eq!(output.len(), 3);
    }

    // --- Combined ---

    #[test]
    fn test_combined_self_move_and_jump_to_next() {
        let input = vec![
            IrInstr::Move { dst: 1, src: 1 },
            IrInstr::Jump { target: IrLabel(0) },
            IrInstr::Label { label: IrLabel(0) },
        ];
        let output = run(input);
        assert_eq!(output.len(), 1);
        assert!(matches!(output[0], IrInstr::Label { label: IrLabel(0) }));
    }

    #[test]
    fn test_empty_input() {
        assert!(run(vec![]).is_empty());
    }

    #[test]
    fn test_no_optimizable_patterns() {
        let input = vec![
            IrInstr::LoadImm { dst: 0, index: 0 },
            IrInstr::Return { src: 0 },
        ];
        let output = run(input);
        assert_eq!(output.len(), 2);
        assert!(matches!(output[0], IrInstr::LoadImm { dst: 0, index: 0 }));
        assert!(matches!(output[1], IrInstr::Return { src: 0 }));
    }
}
