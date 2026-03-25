# Design: Dominator Tree, Inter-block GVN, LICM, and Peephole Optimizer

**Date:** 2026-03-25
**Status:** Approved

## Context

The Inklang SSA optimization pipeline currently runs:

```
CP → GVN (intra-block) → AlgebraicSimpl → CopyProp → DCE
```

Three runtime-performance gaps remain:

1. **GVN is intra-block only** — expressions available in a dominator block are not reused in dominated blocks.
2. **No loop optimization** — loop-invariant computations are re-executed every iteration.
3. **Post-SSA linear IR has trivial redundancies** — jump-to-next and self-move instructions survive into the bytecode.

This spec adds a shared dominator tree infrastructure and three new optimization passes to close those gaps.

---

## Updated Pipeline

```
CP → GVN (inter-block) → AlgebraicSimpl → CopyProp → DCE → LICM
                                                              ↓
                        Liveness → RegAlloc → SpillInsert → Peephole → Codegen
```

LICM runs last in the SSA sequence so it operates on already-optimized code. Peephole runs on the final physical-register linear IR, after spill insertion.

---

## Part 1: Shared Foundation — `src/inklang/ssa/dominance.rs`

### Motivation

The SSA builder already computes immediate dominators (`idoms`) and dominance frontiers internally. Inter-block GVN and LICM both require dominator tree queries. A shared module avoids duplicate computation and a duplicate implementation to maintain.

### `DominatorTree` struct

```rust
pub struct DominatorTree {
    /// Immediate dominator of each block. Entry block has no idom.
    idoms: HashMap<usize, usize>,
    /// Children in the dominator tree: block → blocks it immediately dominates.
    children: HashMap<usize, Vec<usize>>,
    /// Dominance frontiers (retained for SSA builder use).
    frontiers: HashMap<usize, HashSet<usize>>,
    /// Entry block ID.
    entry: usize,
}
```

### Public API

```rust
impl DominatorTree {
    /// Compute dominator tree for the given SSA function.
    pub fn compute(func: &SsaFunction) -> Self;

    /// Immediate dominator of `block_id`. None for the entry block.
    pub fn idom(&self, block_id: usize) -> Option<usize>;

    /// Blocks immediately dominated by `block_id` (direct children in the dom tree).
    pub fn children(&self, block_id: usize) -> &[usize];

    /// True if block `a` dominates block `b` (a is an ancestor of b in the dom tree,
    /// or a == b).
    pub fn dominates(&self, a: usize, b: usize) -> bool;

    /// Blocks in reverse post-order (RPO). Entry block first.
    /// RPO guarantees a dominator is visited before its dominated blocks.
    pub fn rpo(&self, func: &SsaFunction) -> Vec<usize>;

    /// Back edges: (tail, head) pairs where head dominates tail.
    /// Used by LICM for natural loop detection.
    pub fn back_edges(&self, func: &SsaFunction) -> Vec<(usize, usize)>;

    /// Dominance frontier of `block_id`.
    pub fn frontier(&self, block_id: usize) -> &HashSet<usize>;
}
```

### SSA builder refactor

`SsaBuilder`'s private `DominanceFrontier` struct is replaced with `DominatorTree`. Phi placement code uses `dom_tree.frontier(block_id)` instead of the private equivalent. No behavior change — pure refactor.

### Algorithm

Immediate dominators are computed with the Cooper–Harvey–Kennedy algorithm (simple iterative dataflow, O(n²) in the worst case but fast in practice for typical CFG sizes). RPO is computed via iterative DFS from the entry block. Back edges are identified as edges `(B → H)` where `dom_tree.dominates(H, B)`.

---

## Part 2: Inter-block GVN — extend `src/inklang/ssa/passes/gvn.rs`

### Current behavior

`SsaGlobalValueNumberingPass::run` iterates over blocks in declaration order and calls `process_block` independently. Each block starts with an empty value table; entries do not propagate to other blocks.

### Extended behavior

**Dominator-based value numbering (DVNT):** walk blocks in dominator tree order (DFS, parent before children). Each block inherits its parent's value table. After processing its own instructions, it passes the extended table to each child. After all children finish, the block's own additions are discarded — siblings do not share entries with each other. Only the dominator → dominated relationship guarantees availability on all paths.

### What this catches

```
// Block 0 (dominates all successors):
v0 = x + y           // hashed, canonical = v0

// Block 1 (then-branch, dominated by 0):
v1 = x + y           // already in inherited table → Move v1 ← v0

// Block 2 (join, dominated by 0):
v2 = x + y           // already in inherited table → Move v2 ← v0
                     // current GVN misses this; DVNT catches it
```

### Interface change

`process_block` gains a `parent_table: &HashMap<ExprHash, SsaValue>` parameter. It initializes its local table as a clone of the parent table, then proceeds as before. The `run` method:

1. Builds `DominatorTree::compute(func)`.
2. Does a recursive DFS from the entry block, passing each block's completed table to its children.
3. Returns the modified function with `changed = true` if any block was modified.

No changes to `ExprHash` or the hash/invalidation logic.

---

## Part 3: LICM — new `src/inklang/ssa/passes/licm.rs`

### Algorithm

**Step 1 — Find natural loops.**

Use `DominatorTree::back_edges(func)`. For each back edge `(B → H)`:
- `H` is the **loop header**.
- The **loop body** is the set of CFG nodes that can reach `B` by backward traversal of the CFG without passing through `H`, plus `H` itself.
- Computed via reverse BFS from `B` stopping at `H`.

Multiple back edges to the same header produce nested or overlapping loops; treat each `(B, H)` pair as an independent natural loop for simplicity.

**Step 2 — Find loop-invariant instructions.**

An `SsaInstr` in a loop body block is **loop-invariant** if all of the following hold:

- It is not a phi function (phis must remain at block headers).
- It has no side effects: not `Call`, `StoreGlobal`, `SetIndex`, `SetField`, `NewArray`, `NewInstance`, `LoadGlobal`, `Break`, `Next`, `Return`, `CallHandler`, `RegisterEventHandler`, `InvokeEventHandler`, `AsyncCallInstr`, `SpawnInstr`, `AwaitInstr`.
- All values in `instr.used_values()` are either:
  - Defined outside the loop body, OR
  - The `defined_value` of another loop-invariant instruction being hoisted in this same analysis (iterative to fixed point).

**Conservative rule:** if the loop body contains any `Call` instruction, skip LICM for that loop entirely.

**Step 3 — Insert pre-header block.**

For each loop with loop-invariant instructions to hoist:

- Create a new `SsaBlock` with a fresh ID (max existing ID + 1).
- Redirect all non-back-edge predecessors of `H` to the pre-header.
- Set pre-header's sole successor = `H`.
- Update `H`'s predecessor list: replace non-back-edge predecessors with the pre-header ID.
- Update phi functions in `H`: operands from non-back-edge predecessors are re-keyed to the pre-header ID.

**Step 4 — Hoist instructions.**

Move loop-invariant instructions to the pre-header in dependency order (topological: an instruction whose operands are defined by other hoisted instructions must come after those). Append a `Jump` to `H` at the end of the pre-header.

Remove hoisted instructions from their original blocks.

### `SsaOptPass` implementation

```rust
pub struct SsaLicmPass;

impl SsaOptPass for SsaLicmPass {
    fn name(&self) -> &str { "SsaLicm" }
    fn run(&mut self, ssa_func: SsaFunction) -> SsaOptResult;
}
```

`run` is NOT run to fixed point by `run_pass` (LICM is not iterative — one pass suffices since a single application hoists all invariant instructions). It is called once.

### Pipeline position

LICM runs after DCE in `run_optimization_passes`:

```
CP → GVN → AlgebraicSimpl → CopyProp → DCE → LICM
```

Running after DCE ensures LICM does not hoist instructions that DCE would have eliminated anyway.

---

## Part 4: Peephole Optimizer — new `src/inklang/peephole.rs`

### Input / output

```rust
pub fn peephole_optimize(instrs: Vec<IrInstr>) -> Vec<IrInstr>;
```

Operates on the final physical-register linear `Vec<IrInstr>` — after `SpillInserter::insert`, before `IrCompiler::compile`.

### Patterns

**Pattern 1 — Jump-to-next elimination.**

Scan for any `IrInstr::Jump { target: L }` immediately followed by `IrInstr::Label { label: L }` (same label value). The jump unconditionally transfers to the next instruction — it is a no-op. Remove the `Jump`.

Implementation: single linear scan. If `instrs[i]` is `Jump { target }` and `instrs[i+1]` is `Label { label }` where `label == target`, skip `instrs[i]`.

**Pattern 2 — Self-move elimination.**

Scan for `IrInstr::Move { dst, src }` where `dst == src`. These appear when register allocation assigns the same physical register to both the destination and source virtual registers of a phi-deconstruction move. Remove the `Move`.

### Pipeline integration

In `src/inklang/mod.rs`, `compile_with_grammar`:

```rust
let resolved = SpillInserter::new().insert(ssa_result.instrs, &alloc, &ranges);
let resolved = peephole::peephole_optimize(resolved);  // new
let codegen_result = codegen::LoweredResult { instrs: resolved, ... };
```

---

## Testing Strategy

### `dominance.rs`

- `DominatorTree::compute` on a simple linear CFG: each block dominates all successors.
- Diamond CFG (entry → A, entry → B, A → exit, B → exit): entry dominates all; A does not dominate B; exit dominated by entry.
- `dominates(a, b)` is reflexive: `dominates(x, x)` is always true.
- `rpo` visits entry first and each block after its dominator.
- `back_edges` detects a loop back-edge correctly; returns empty for a DAG.

### Inter-block GVN

- Expression in entry block is not recomputed in dominated successor (cross-block deduplication).
- Expression in one branch of an `if` is NOT eliminated in the other branch (siblings don't share tables).
- Expression in both branches IS eliminated after the join if it was in the dominator (entry).
- `changed = false` when no cross-block duplicates exist.

### LICM

- Loop-invariant `BinaryOp` (no loop variables) is hoisted to pre-header.
- Loop-variant instruction (operand defined inside loop) is NOT hoisted.
- Loop with a `Call` instruction: no hoisting occurs.
- Pre-header is correctly inserted: non-back-edge predecessors redirected, `H`'s phi operands re-keyed.
- Dependency order: if instruction A feeds instruction B and both are invariant, A appears before B in the pre-header.

### Peephole

- `Jump L; Label L` → Jump removed.
- `Jump L1; Label L2` (different labels) → unchanged.
- `Move r0, r0` → removed.
- `Move r0, r1` (different regs) → unchanged.
- Empty input → empty output, no panic.
- Mixed stream: only matching patterns removed, others preserved.

### Integration

All existing 224 lib tests and 16 round-trip integration tests continue to pass after each change.

---

## File Checklist

| File | Change |
|---|---|
| `src/inklang/ssa/dominance.rs` | New — `DominatorTree` |
| `src/inklang/ssa/mod.rs` | Re-export `DominatorTree`; add LICM to pipeline |
| `src/inklang/ssa/passes/mod.rs` | Add `pub mod licm` |
| `src/inklang/ssa/passes/licm.rs` | New — `SsaLicmPass` |
| `src/inklang/ssa/passes/gvn.rs` | Extend with DVNT inter-block propagation |
| `src/inklang/ssa/builder.rs` | Refactor to use `DominatorTree` |
| `src/inklang/peephole.rs` | New — `peephole_optimize` |
| `src/inklang/mod.rs` | Wire peephole between spill insert and codegen |
