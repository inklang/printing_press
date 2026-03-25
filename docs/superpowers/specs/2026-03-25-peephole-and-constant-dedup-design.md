# Peephole Optimization and Constant Table Deduplication

**Date**: 2026-03-25
**Status**: Approved

## Overview

Two medium-impact optimizations targeting the final stages of the Inklang compiler pipeline:

1. **Constant table deduplication** — fix `add_constant` in the lowerer to return existing indices for duplicate values, enabling CP and GVN to see a single canonical constant index per value.
2. **Peephole pass** — a new linear IR cleanup pass applied after register allocation and spill insertion, eliminating jump-to-next-instruction and self-move patterns before codegen.

---

## 1. Constant Table Deduplication

### Problem

`AstLowerer::add_constant` unconditionally pushes to `self.constants`:

```rust
fn add_constant(&mut self, value: Value) -> usize {
    self.constants.push(value);
    self.constants.len() - 1
}
```

If `Int(0)` appears at index 0 and index 5, the CP and GVN passes treat them as distinct `LoadImm` instructions and miss folds — they key on `const_index`, not the value itself.

### Fix

**File**: `src/inklang/lowerer.rs`

Linear-scan deduplication in `add_constant`:

```rust
fn add_constant(&mut self, value: Value) -> usize {
    if let Some(idx) = self.constants.iter().position(|c| c == &value) {
        return idx;
    }
    self.constants.push(value);
    self.constants.len() - 1
}
```

`Value` already derives `PartialEq`. The constants table is small for typical programs, so O(n) per call is acceptable. No other files change.

### Effect

All callers of `add_constant` get deduplication automatically. Each unique compile-time constant maps to exactly one index, so CP and GVN see canonical indices and can fold more expressions.

---

## 2. Peephole Pass

### Problem

After SSA deconstruction + register allocation + spill insertion, two wasteful patterns can survive into the final `Vec<IrInstr>`:

- **Self-moves**: `Move { dst: r, src: r }` — introduced when two SSA values that were copies land on the same physical register after allocation.
- **Jump-to-next**: `Jump { target: L }` where `Label { label: L }` is the very next non-redundant instruction — introduced by control flow normalization in the lowerer and SSA deconstruction.

These patterns reach codegen unchanged and emit real bytecode instructions that do nothing.

### Architecture

**New file**: `src/inklang/peephole.rs`

**Public API**:
```rust
pub fn run(instrs: Vec<IrInstr>) -> Vec<IrInstr>
```

Single forward pass. Builds a new `Vec<IrInstr>` by filtering patterns. No mutation of the deconstructor, register allocator, or codegen.

**Integration** in `src/inklang/mod.rs`, between SpillInserter and IrCompiler:
```rust
let resolved = SpillInserter::new().insert(ssa_result.instrs, &alloc, &ranges);
let resolved = peephole::run(resolved);  // new
// ... codegen ...
```

### Algorithm

Two patterns, one forward scan:

**Self-move elimination**
While iterating, if the current instruction is `Move { dst, src }` and `dst == src`, skip it.

**Jump-to-next elimination**
While iterating, if the current instruction is `Jump { target: L }`, scan forward in the remaining instructions skipping any `Label` instructions. If the first non-`Label` instruction encountered is `Label { label: L }` (i.e., the target label is immediately reachable without executing any real instructions), skip the `Jump`.

Both patterns are independent; one pass suffices.

### Tests

In `src/inklang/peephole.rs` `#[cfg(test)]`:

| Test | Input | Expected output |
|------|-------|-----------------|
| Self-move dropped | `[Move{dst:1, src:1}]` | `[]` |
| Non-self-move kept | `[Move{dst:1, src:2}]` | `[Move{dst:1, src:2}]` |
| Jump-to-next dropped | `[Jump{L0}, Label{L0}]` | `[Label{L0}]` |
| Jump-to-next with intervening labels dropped | `[Jump{L1}, Label{L0}, Label{L1}]` | `[Label{L0}, Label{L1}]` |
| Jump to distant label kept | `[Jump{L0}, LoadImm{...}, Label{L0}]` | unchanged |

---

## Pipeline Summary

```
Lowerer (add_constant deduplicates)
  → SSA round-trip (CP/GVN now see canonical indices)
    → Liveness + RegAlloc + SpillInserter
      → peephole::run  (new)
        → IrCompiler → Chunk
```

## Files Changed

| File | Change |
|------|--------|
| `src/inklang/lowerer.rs` | `add_constant`: linear scan before push |
| `src/inklang/peephole.rs` | New file: `pub fn run(Vec<IrInstr>) -> Vec<IrInstr>` |
| `src/inklang/mod.rs` | `pub mod peephole;` declaration + call between spill and codegen |
