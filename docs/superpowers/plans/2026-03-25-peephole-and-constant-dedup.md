# Peephole Optimization and Constant Table Deduplication Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate duplicate constants at lowering time and remove wasteful self-moves and jump-to-next instructions after register allocation.

**Architecture:** Two independent changes: (1) `add_constant` in the lowerer does a linear scan before pushing, deduplicating at source; (2) a new `peephole::run` function does a single forward scan over `Vec<IrInstr>` post-spill-insertion, dropping self-moves and redundant unconditional jumps before codegen.

**Tech Stack:** Rust, `cargo test`

---

## Chunk 1: Constant Table Deduplication

### Task 1: Add deduplication to `add_constant`

**Files:**
- Modify: `src/inklang/lowerer.rs:109-113`

- [ ] **Step 1: Write the failing test**

Add to the existing `#[cfg(test)] mod tests` block at the bottom of `src/inklang/lowerer.rs` (after the last `#[test]` fn, before the closing `}`):

```rust
#[test]
fn test_add_constant_dedup_first_constant() {
    let mut lowerer = AstLowerer::new();
    let idx = lowerer.add_constant(Value::Int(1));
    assert_eq!(idx, 0);
    assert_eq!(lowerer.constants, vec![Value::Int(1)]);
}

#[test]
fn test_add_constant_dedup_duplicate_returns_existing() {
    let mut lowerer = AstLowerer::new();
    let idx0 = lowerer.add_constant(Value::Int(1));
    let idx1 = lowerer.add_constant(Value::Int(1));
    assert_eq!(idx0, 0);
    assert_eq!(idx1, 0);
    assert_eq!(lowerer.constants.len(), 1);
}

#[test]
fn test_add_constant_dedup_different_values() {
    let mut lowerer = AstLowerer::new();
    let a = lowerer.add_constant(Value::Int(1));
    let b = lowerer.add_constant(Value::Int(2));
    assert_eq!(a, 0);
    assert_eq!(b, 1);
    assert_eq!(lowerer.constants.len(), 2);
}

#[test]
fn test_add_constant_dedup_string() {
    let mut lowerer = AstLowerer::new();
    let a = lowerer.add_constant(Value::String("foo".to_string()));
    let b = lowerer.add_constant(Value::String("foo".to_string()));
    assert_eq!(a, b);
    assert_eq!(lowerer.constants.len(), 1);
}

#[test]
fn test_add_constant_dedup_boolean() {
    let mut lowerer = AstLowerer::new();
    let a = lowerer.add_constant(Value::Boolean(true));
    let b = lowerer.add_constant(Value::Boolean(true));
    assert_eq!(a, b);
    assert_eq!(lowerer.constants.len(), 1);
}

#[test]
fn test_add_constant_dedup_null() {
    let mut lowerer = AstLowerer::new();
    let a = lowerer.add_constant(Value::Null);
    let b = lowerer.add_constant(Value::Null);
    assert_eq!(a, b);
    assert_eq!(lowerer.constants.len(), 1);
}

#[test]
fn test_add_constant_dedup_mixed_unique() {
    let mut lowerer = AstLowerer::new();
    let a = lowerer.add_constant(Value::Int(0));
    let b = lowerer.add_constant(Value::Int(1));
    let c = lowerer.add_constant(Value::Int(0)); // duplicate
    let d = lowerer.add_constant(Value::Int(2));
    assert_eq!(a, 0);
    assert_eq!(b, 1);
    assert_eq!(c, 0); // returns existing
    assert_eq!(d, 2);
    assert_eq!(lowerer.constants.len(), 3);
}

#[test]
fn test_add_constant_dedup_float() {
    let mut lowerer = AstLowerer::new();
    let a = lowerer.add_constant(Value::Float(1.0));
    let b = lowerer.add_constant(Value::Float(1.0));
    assert_eq!(a, b);
    assert_eq!(lowerer.constants.len(), 1);
}

#[test]
fn test_add_constant_dedup_distinct_floats() {
    let mut lowerer = AstLowerer::new();
    let a = lowerer.add_constant(Value::Float(1.0));
    let b = lowerer.add_constant(Value::Float(2.0));
    assert_eq!(a, 0);
    assert_eq!(b, 1);
}

#[test]
fn test_add_constant_dedup_nan_not_deduplicated() {
    // f32::NAN != f32::NAN under IEEE 754, so two NaN entries are expected
    let mut lowerer = AstLowerer::new();
    let a = lowerer.add_constant(Value::Float(f32::NAN));
    let b = lowerer.add_constant(Value::Float(f32::NAN));
    assert_ne!(a, b);
    assert_eq!(lowerer.constants.len(), 2);
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test test_add_constant_dedup 2>&1 | tail -20
```

Expected: several tests FAIL (currently `add_constant` always pushes, so duplicate tests will fail).

- [ ] **Step 3: Implement deduplication**

In `src/inklang/lowerer.rs`, replace lines 109-113:

```rust
    /// Add a constant to the constants table and return its index.
    fn add_constant(&mut self, value: Value) -> usize {
        self.constants.push(value);
        self.constants.len() - 1
    }
```

with:

```rust
    /// Add a constant to the constants table and return its index.
    /// Returns the existing index if an equal value is already present.
    fn add_constant(&mut self, value: Value) -> usize {
        if let Some(idx) = self.constants.iter().position(|c| c == &value) {
            return idx;
        }
        self.constants.push(value);
        self.constants.len() - 1
    }
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test test_add_constant_dedup 2>&1 | tail -20
```

Expected: all 10 `test_add_constant_dedup_*` tests PASS.

- [ ] **Step 5: Run the full test suite**

```bash
cargo test 2>&1 | tail -20
```

Expected: all tests pass. If any test breaks, it means something was relying on duplicate constants having distinct indices — investigate before committing.

- [ ] **Step 6: Commit**

```bash
git add src/inklang/lowerer.rs
git commit -m "feat: deduplicate constants table in add_constant"
```

---

## Chunk 2: Peephole Pass

### Task 2: Create `peephole.rs` with tests and implementation

**Files:**
- Create: `src/inklang/peephole.rs`
- Modify: `src/inklang/mod.rs:13` (add `pub mod peephole;`)
- Modify: `src/inklang/mod.rs:115` (insert `peephole::run` call)

- [ ] **Step 1: Create the module file with tests only (no implementation yet)**

Create `src/inklang/peephole.rs`:

```rust
//! Peephole optimization pass.
//!
//! Applied after register allocation and spill insertion, before codegen.
//! Eliminates two patterns:
//! - Self-moves: `Move { dst: r, src: r }` where dst == src
//! - Jump-to-next: `Jump { target: L }` where Label L immediately follows
//!   (with only other Labels in between)

use crate::inklang::ir::{IrInstr, IrLabel};

/// Run all peephole optimizations on a linear instruction stream.
/// Returns a new Vec with wasteful instructions removed.
pub fn run(instrs: Vec<IrInstr>) -> Vec<IrInstr> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inklang::ir::IrInstr;
    use crate::inklang::ir::IrLabel;

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
```

- [ ] **Step 2: Register the module in `mod.rs`**

In `src/inklang/mod.rs`, add `pub mod peephole;` after line 14 (`pub mod codegen;`):

```rust
pub mod codegen;
pub mod peephole;   // ← add this line
pub mod chunk;
```

- [ ] **Step 3: Run tests to verify they fail with `todo!()`**

```bash
cargo test --lib peephole 2>&1 | tail -20
```

Expected: tests compile but panic with `not yet implemented`.

- [ ] **Step 4: Implement `peephole::run`**

Replace the `todo!()` stub in `src/inklang/peephole.rs`:

```rust
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
            // (with only Label instructions in between)
            IrInstr::Jump { target } => {
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
            }
            _ => {
                output.push(instrs[i].clone());
                i += 1;
            }
        }
    }

    output
}
```

- [ ] **Step 5: Run peephole tests**

```bash
cargo test --lib peephole 2>&1 | tail -30
```

Expected: all peephole tests PASS.

- [ ] **Step 6: Wire the pass into the pipeline**

In `src/inklang/mod.rs`, update `compile_with_grammar` to call `peephole::run` after spill insertion.

Replace:

```rust
    // 6. Liveness + register allocation + spill
    let ranges = LivenessAnalyzer::new().analyze(&ssa_result.instrs);
    let mut allocator = RegisterAllocator::new();
    let alloc = allocator.allocate(&ranges, lowered.arity);
    let resolved = SpillInserter::new().insert(ssa_result.instrs, &alloc, &ranges);

    // 7. Codegen
```

with:

```rust
    // 6. Liveness + register allocation + spill
    let ranges = LivenessAnalyzer::new().analyze(&ssa_result.instrs);
    let mut allocator = RegisterAllocator::new();
    let alloc = allocator.allocate(&ranges, lowered.arity);
    let resolved = SpillInserter::new().insert(ssa_result.instrs, &alloc, &ranges);

    // 6b. Peephole cleanup
    let resolved = peephole::run(resolved);

    // 7. Codegen
```

Also update the pipeline comment at the top of `compile` and `compile_with_grammar` from:
```
/// 1. Tokenize → 2. Parse → 3. Constant Fold → 4. Lower to IR → 5. SSA Round-trip → 6. Register Alloc → 7. Codegen → 8. Serialize
```
to:
```
/// 1. Tokenize → 2. Parse → 3. Constant Fold → 4. Lower to IR → 5. SSA Round-trip → 6. Register Alloc → 6b. Peephole → 7. Codegen → 8. Serialize
```

- [ ] **Step 7: Run the full test suite**

```bash
cargo test 2>&1 | tail -30
```

Expected: all tests pass.

- [ ] **Step 8: Commit**

```bash
git add src/inklang/peephole.rs src/inklang/mod.rs
git commit -m "feat: add peephole pass eliminating self-moves and jump-to-next"
```
