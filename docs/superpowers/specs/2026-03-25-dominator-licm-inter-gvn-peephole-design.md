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
CP → GVN (inter-block) → AlgebraicSimpl → CopyProp → DCE → LICM (one-shot)
                                                              ↓
                                                         Deconstruct
                                                              ↓
                        Liveness → RegAlloc → SpillInsert → Peephole → Codegen
```

LICM runs last in the SSA sequence so it operates on already-optimized code. The SSA Deconstruct step (already present in `optimized_ssa_round_trip`) follows LICM before handing off to Liveness. Peephole runs on the final physical-register linear IR, after spill insertion.

---

## Part 1: Shared Foundation — `src/inklang/ssa/dominance.rs`

### Motivation

The SSA builder already computes immediate dominators (`idoms`) and dominance frontiers privately inside `CfgBlock`-typed data. Inter-block GVN and LICM both need dominator tree queries on `SsaFunction` blocks. A shared module eliminates duplicate code and provides one authoritative implementation to maintain.

### Two constructors

`DominatorTree` is constructed from two distinct call sites with different graph types:

```rust
impl DominatorTree {
    /// Build from a finished SsaFunction (used by optimization passes).
    /// SsaBlock already carries predecessors/successors, so no extra data needed.
    pub fn from_ssa(func: &SsaFunction) -> Self;

    /// Build from raw CfgBlock slices (used inside SsaBuilder during construction,
    /// before the SsaFunction exists).
    pub fn from_cfg(blocks: &[CfgBlock], entry: usize) -> Self;
}
```

`CfgBlock` must be made `pub(super)` so that `dominance.rs` can accept it. In Rust, `pub(super)` on an item in `src/inklang/ssa/builder.rs` makes it visible to `src/inklang/ssa/mod.rs` and all sibling submodules declared there — including `dominance.rs`. No re-export in `mod.rs` is needed. It does not need to be public outside `ssa`.

The `SsaBuilder` refactor replaces its private `DominanceFrontier` with `DominatorTree::from_cfg`. The `DominanceFrontier` struct and its `impl` block are **deleted entirely**. Optimization passes call `DominatorTree::from_ssa`.

### `DominatorTree` struct

```rust
pub struct DominatorTree {
    /// Immediate dominator of each block. Entry block has no idom.
    idoms: HashMap<usize, usize>,
    /// Children in the dominator tree: block → blocks it immediately dominates.
    /// All block IDs are pre-populated with empty Vecs during compute.
    children: HashMap<usize, Vec<usize>>,
    /// Dominance frontiers (retained for SSA builder phi placement).
    frontiers: HashMap<usize, HashSet<usize>>,
    /// Entry block ID.
    entry: usize,
    /// Blocks in reverse post-order. Precomputed during construction.
    rpo_order: Vec<usize>,
    /// Back edges: (tail, head) pairs where head dominates tail. Precomputed.
    back_edge_list: Vec<(usize, usize)>,
}
```

All fields derived from the graph (RPO, back edges) are **precomputed during construction** so query methods are cheap without re-traversal. The `func` argument is not re-passed to query methods.

`dominates(a, b)` is implemented by climbing the idom chain from `b` toward the root, returning true if `a` is reached or `a == b`. This is O(tree depth), which is acceptable for typical CFG depths. An O(1) implementation using precomputed DFS entry/exit timestamps is not required.

### Public API

```rust
impl DominatorTree {
    /// Immediate dominator of `block_id`. None for the entry block.
    pub fn idom(&self, block_id: usize) -> Option<usize>;

    /// Blocks immediately dominated by `block_id` (direct children in the dom tree).
    /// Returns an empty slice for blocks with no children.
    pub fn children(&self, block_id: usize) -> &[usize];

    /// True if block `a` dominates block `b` (a is an ancestor of b, or a == b).
    pub fn dominates(&self, a: usize, b: usize) -> bool;

    /// Blocks in reverse post-order. Entry block first.
    /// Guarantees each dominator appears before its dominated blocks.
    pub fn rpo(&self) -> &[usize];

    /// Back edges: (tail, head) pairs where head dominates tail.
    pub fn back_edges(&self) -> &[(usize, usize)];

    /// Dominance frontier of `block_id`.
    pub fn frontier(&self, block_id: usize) -> &HashSet<usize>;

    /// Iterated dominance frontier of a set of blocks.
    /// Required by SsaBuilder for phi placement.
    pub fn iterated_frontier(&self, blocks: &HashSet<usize>) -> HashSet<usize>;
}
```

**`children` note:** `children: HashMap<usize, Vec<usize>>` is pre-populated with an empty `Vec` for every block during construction, so `self.children.get(block_id).map(Vec::as_slice).unwrap_or(&[])` is always valid — no missing-key case at runtime.

### Algorithm

Immediate dominators: Cooper–Harvey–Kennedy iterative dataflow (simple, O(n²) worst case, fast in practice). The `intersect` helper used in CHK climbs the idom chain by comparing **RPO positions** (indices into `rpo_order`), not raw block IDs. Blocks must be mapped to their RPO index via a precomputed `HashMap<block_id, rpo_index>` before comparison — the condition `while rpo_pos[a] != rpo_pos[b]` uses positions, not IDs. The existing `DominanceFrontier::compute` in `builder.rs` compares raw block IDs (`while a > b`) which is only correct when block IDs happen to be assigned in RPO order. `DominatorTree::from_cfg` fixes this by using true RPO position comparison. This is a **correctness improvement**, not a pure structural refactor — in CFGs where block IDs are not in RPO order the results may differ. The existing builder tests validate correctness of phi placement on the new implementation. During construction, the entry block is stored with `idoms[entry] = entry` as a sentinel. The `idom()` accessor returns `None` when `block_id == self.entry`, preventing infinite loops in `dominates()`. RPO: iterative DFS from entry, reverse of post-order. Back edges: edges `(B → H)` where `dominates(H, B)`, collected during RPO traversal. Dominance frontiers: standard algorithm using `idoms`.

### SSA builder refactor

`DominanceFrontier` private struct in `builder.rs` is replaced with `DominatorTree::from_cfg`. The `dom_frontier: DominanceFrontier` field on `SsaBuilder` is **removed**, and the `DominanceFrontier::compute` call in `build` is deleted.

`build` constructs `DominatorTree::from_cfg(&builder.cfg_blocks, entry_block)` **once** after `build_cfg` completes, and passes it to both call sites:
- `place_phi_functions` gains a `dom_tree: &DominatorTree` parameter and calls `dom_tree.iterated_frontier(&def_block_set)`.
- `rename_variables` gains a `dom_tree: &DominatorTree` parameter (and no longer needs to construct one internally).

Existing builder tests validate correctness. One new targeted test (see Testing section) validates phi placement on a canonical diamond CFG.

**`rename_variables` / `rename_block` migration:**

`rename_variables` gains a `dom_tree: &DominatorTree` parameter (still private, still only called from `build`). Its internal `let dom_tree = self.dom_frontier.dominator_tree()` line is removed. In `build`, the call site changes from:

```rust
// Before:
builder.rename_variables();

// After:
builder.rename_variables(&dom_tree);  // dom_tree built once earlier in build()
```

The call to `self.rename_block` inside `rename_variables` is otherwise unchanged except the `dom_tree` type changes.

`rename_block`'s third parameter changes from `dom_tree: &HashMap<usize, Vec<usize>>` to `dom_tree: &DominatorTree`. The recursive child-iteration at the bottom of `rename_block` changes from:

```rust
// Before:
if let Some(children) = dom_tree.get(&block_id) {
    for &child_id in children {
        self.rename_block(child_id, dom_tree, counters, stacks);
    }
}

// After:
for &child_id in dom_tree.children(block_id) {
    self.rename_block(child_id, dom_tree, counters, stacks);
}
```

`dom_tree.children(block_id)` returns `&[usize]` (never panics — all block IDs are pre-populated during construction), so the `Option` unwrap is eliminated.

---

## Part 2: Inter-block GVN — extend `src/inklang/ssa/passes/gvn.rs`

### Current behavior

`run` iterates over blocks in declaration order and calls `process_block` independently. Each block starts with an empty value table; entries do not propagate across blocks.

### Extended behavior

**Dominator-based value numbering (DVNT):** process blocks in **RPO order** (from `dom_tree.rpo()`). RPO guarantees every dominator appears before its dominated children. Each block inherits its immediate dominator's completed value table. Siblings never share tables — each child receives an independent clone of its idom's table.

### Soundness: value table inheritance

The existing `process_block` removes an entry from `value_table` when a side-effecting instruction invalidates its hash. The invariant maintained is: **entries in `value_table` at block exit are safe to reuse in dominated blocks** — any entry whose hash was invalidated mid-block has already been removed before the block's processing completes.

**This is a behavioral change to the existing `gvn.rs` code, not merely a structural one.** Inspection of the current `process_block` implementation reveals that when a hash H is added to `invalidated_by_side_effect`, the corresponding entry in `value_table` is **not** removed. This means stale entries currently linger in `value_table` within a single block (harmless today because tables are discarded at block exit). With inter-block propagation the table is passed to child blocks, so lingering stale entries would cause unsound rewrites.

The fix: at every point in `process_block` where a hash H is inserted into `invalidated_by_side_effect`, immediately also call `value_table.remove(&H)`. After this change the two maps are disjoint at all times, making the table safe to clone and pass to children.

**Note:** In practice, `compute_hash` currently returns `None` for all side-effecting instructions (including `Call`, `LoadGlobal`, `SetIndex`, etc.), so `invalidated_by_side_effect` is always empty under the current implementation. The explicit removal is therefore a **defensive invariant** that costs nothing today but prevents unsound cross-block propagation if `compute_hash` is ever extended to hash side-effecting instructions.

**Additional invalidation for load/store aliasing:** `SetIndex` and `SetField` instructions can alias with `GetIndex` and `GetField` instructions on the same collection. When a `SetIndex` or `SetField` instruction is encountered in `process_block`, **before** calling `try_gvn` for that instruction, remove all `GetIndex` and `GetField` entries from `value_table`:

```rust
value_table.retain(|k, _| !matches!(k, ExprHash::GetIndex(..) | ExprHash::GetField(..)));
```

This prevents a parent block's `GetIndex` result from being incorrectly reused in a child block that follows a `SetIndex` that modified the same collection. `HasCheck` entries are not purged (they key on field presence, not field values, and `SetField` does not change which fields exist).

`process_block` signature change:
```rust
fn process_block(
    &self,
    block: &mut SsaBlock,
    parent_table: HashMap<ExprHash, SsaValue>,
) -> (bool, HashMap<ExprHash, SsaValue>)
// returns (changed, table_for_children)
```

The returned table is cloned once per child (dominator tree children may be multiple).

**Ownership strategy for the `run` method:** Rust does not allow simultaneously holding `&mut SsaBlock` across a recursive call that also borrows `ssa_func`. The `run` method must avoid recursive mutable borrowing. The correct approach: process blocks in **RPO order** (from `dom_tree.rpo()`), which guarantees every dominator is processed before its dominated children. Maintain a `HashMap<block_id, table>` that stores the completed table for each block. For each block in RPO order:

1. Look up the block's idom: `dom_tree.idom(block_id)`.
2. Clone the idom's table from the map (or start with an empty table for the entry block).
3. Find the block index in `ssa_func.blocks` by ID (a linear scan or a precomputed `block_id → vec_index` map).
4. Call `process_block(&mut ssa_func.blocks[idx], parent_table)`.
5. Store the returned table in the map under `block_id`.

This processes each block exactly once with a single mutable borrow scoped to the `process_block` call, with no overlapping borrows.

### What this catches

```
// Block 0 (dominates all):
v0 = x + y

// Block 1 (then-branch, dominated by 0):
v1 = x + y   // → Move v1 ← v0  (already caught by intra-block GVN if in same block)

// Block 2 (join, dominated by 0):
v2 = x + y   // → Move v2 ← v0  (inter-block GVN catches this; current pass does not)
```

### `run` method change

Builds `DominatorTree::from_ssa(&ssa_func)`, then iterates blocks in RPO order using the ownership strategy described above. The `changed` flag accumulates across all blocks. Otherwise identical to current behavior.

---

## Part 3: LICM — new `src/inklang/ssa/passes/licm.rs`

### Algorithm

**Step 1 — Find natural loops.**

Use `DominatorTree::from_ssa(func).back_edges()`. Group back edges by header: for each unique header `H`, collect all back-edge tails `{B₁, B₂, …}` that point to `H`.

For each unique header `H`:
- The **loop body** is the **union** of reverse-BFS bodies from each tail `Bᵢ` to `H`, plus `H` itself. Run reverse BFS from every `Bᵢ` simultaneously (or sequentially, unioning results), stopping at `H`.
- This merged body is used for all subsequent steps (Step 2 invariant analysis, Step 3 pre-header insertion).

Treating multiple back edges to the same header as separate independent loops is unsound: instructions in one sub-body may use values defined in another sub-body, making them loop-variant with respect to the full loop.

**Step 2 — Find loop-invariant instructions.**

An `SsaInstr` in a loop body block is **loop-invariant** if all of:

- It is not a phi function (phis must remain at block headers).
- It has no side effects. Side-effecting instructions (excluded from hoisting):
  `Call`, `CallHandler`, `StoreGlobal`, `SetIndex`, `SetField`, `NewArray`, `NewInstance`,
  `LoadGlobal`, `Break`, `Next`, `Return`, `RegisterEventHandler`, `InvokeEventHandler`,
  `AsyncCallInstr`, `SpawnInstr`, `AwaitInstr`.
- All values in `instr.used_values()` are either defined outside the loop body OR are the `defined_value` of another loop-invariant instruction being hoisted in this analysis (computed iteratively to fixed point).

**Conservative loop-level bail-out:** if the loop body contains any `Call` OR `CallHandler` instruction anywhere, skip LICM for that loop entirely. Both can invoke arbitrary user code with unpredictable side effects that make it unsafe to reason about instruction movement even for nominally-pure instructions elsewhere in the loop. The per-instruction side-effect check above is still applied when the bail-out does not trigger.

**Step 3 — Insert pre-header block.**

For each loop with loop-invariant instructions to hoist:

- Create a new `SsaBlock` with ID = `func.blocks.iter().map(|b| b.id).max().unwrap_or(0) + 1`. **When processing multiple loops, recompute this max after each pre-header insertion** — do not cache the maximum upfront, as the prior pre-header's ID must be included.
- **Insert** the pre-header into `func.blocks` at the index immediately before the loop header's current position (not appended to the end). This is a stylistic convention — RPO is derived from graph traversal, not `Vec` order, so physical position has no correctness impact. Inserting before the header keeps the `Vec` layout readable in dumps.
- Redirect all non-back-edge predecessors of `H` to the pre-header.
- Set pre-header's sole successor = `H`; pre-header's predecessors = former non-back-edge predecessors of `H`.
- Update `H`'s predecessor list: replace former non-back-edge predecessors with the pre-header ID.
- Update phi functions in `H`: operands keyed to former non-back-edge predecessors are re-keyed to the pre-header ID.

**Step 4 — Hoist instructions.**

Append loop-invariant instructions to the pre-header in **topological dependency order**: if instruction A's `defined_value` is used by instruction B and both are being hoisted, A appears before B. The set of hoisted instructions always forms a DAG — by definition every hoisted instruction's operands are defined outside the loop body, so no two hoisted instructions can form a dependency cycle. A simple iterative approach works: repeatedly append any hoisted instruction whose `used_values()` are all either (a) defined in a block outside the loop body, or (b) the `defined_value` of an instruction already appended to the pre-header block in the current hoisting pass — until the set of remaining hoisted instructions is exhausted.

**Pre-header terminator protocol:** The pre-header's final instruction must be `SsaInstr::Jump { target: label }` where `label` is an `IrLabel` that appears as `SsaInstr::Label { label }` at the start of `H.instrs`. The exact steps:

1. Scan `H.instrs` for the first `SsaInstr::Label { label }`. If found, use that `label` as the jump target.
2. If `H.instrs` has no leading `Label` instruction, allocate a new one: scan all instructions in all blocks for the maximum `IrLabel` integer value, add 1 to get `new_label`, prepend `SsaInstr::Label { label: new_label }` to `H.instrs`, set `H.label = Some(new_label)` to keep the block's metadata consistent, and use `new_label` as the jump target.

Append `SsaInstr::Jump { target }` as the last instruction of the pre-header block.

Remove hoisted instructions from their original blocks.

### `SsaOptPass` implementation

```rust
pub struct SsaLicmPass;

impl SsaLicmPass {
    pub fn new() -> Self { SsaLicmPass }
}

impl SsaOptPass for SsaLicmPass {
    fn name(&self) -> &str { "SsaLicm" }
    fn run(&mut self, ssa_func: SsaFunction) -> SsaOptResult;
}
```

### Pipeline integration — one-shot, not fixed-point

LICM is called **once directly** in `run_optimization_passes`, NOT through `run_pass` (which loops to fixed point). A single application hoists all invariant instructions; re-running would attempt to re-hoist already-hoisted instructions and must not occur.

`SsaOptResult` (already defined in `passes/mod.rs`) has two fields: `func: SsaFunction` and `changed: bool`. The existing `run_optimization_passes` already declares `let mut optimized = false;` and accumulates via `optimized = result.changed || optimized`. The LICM addition follows the same pattern:

```rust
fn run_optimization_passes(mut ssa_func: SsaFunction) -> SsaOptResult {
    let mut optimized = false;
    // ... CP, GVN, AlgebraicSimpl, CopyProp, DCE via run_pass as before ...

    // LICM: one-shot, not via run_pass
    let licm_result = SsaLicmPass::new().run(ssa_func);
    ssa_func = licm_result.func;
    optimized = licm_result.changed || optimized;

    SsaOptResult { func: ssa_func, changed: optimized }
}
```

---

## Part 4: Peephole Optimizer — new `src/inklang/peephole.rs`

### Input / output

```rust
pub fn peephole_optimize(instrs: Vec<IrInstr>) -> Vec<IrInstr>;
```

Operates on the final physical-register linear `Vec<IrInstr>` — after `SpillInserter::insert`, before `IrCompiler::compile`.

### Patterns

The implementation performs a **single linear scan**, collecting kept instructions into a new `Vec<IrInstr>` (not modifying in-place with index deletion, which would cause O(n²) shifts). Each instruction is either pushed to the output or skipped.

**Pattern 1 — Jump-to-next elimination.**

Scan for `IrInstr::Jump { target: L }` where the immediately following instruction is `IrInstr::Label { label: L }` (same label value). The jump transfers to the next instruction — it is a no-op. Skip the `Jump` (do not push to output).

```
if i + 1 < instrs.len()
   && instrs[i] is Jump { target }
   && instrs[i+1] is Label { label }
   && label == target
→ skip instrs[i]
```

**Pattern 2 — Self-move elimination.**

Scan for `IrInstr::Move { dst, src }` where `dst == src`. These appear when register allocation assigns the same physical register to both the destination and source of a phi-deconstruction move. Skip the `Move`.

### Pipeline integration

In `src/inklang/mod.rs`, `compile_with_grammar`, insert the peephole call between `SpillInserter::insert` and the codegen call. The surrounding code is unchanged; only the one new line is added:

```rust
let resolved = SpillInserter::new().insert(ssa_result.instrs, &alloc, &ranges);
let resolved = peephole::peephole_optimize(resolved);   // new line
// ... existing codegen call follows unchanged
```

---

## Testing Strategy

### `dominance.rs`

- Linear CFG (A → B → C): each block dominates all successors; `rpo` = [A, B, C]; no back edges.
- Diamond CFG (entry → A, entry → B, A → exit, B → exit): entry dominates all; A does not dominate B; B does not dominate A; exit dominated by entry only; `dominates(entry, exit) = true`, `dominates(A, B) = false`.
- `dominates(x, x) = true` for all x (reflexive).
- Single back edge: loop CFG (entry → header → body → header, body → exit); `back_edges` = [(body, header)].
- DAG: `back_edges` returns empty.
- `iterated_frontier` on a single block returns the same as `frontier`.
- `children` for a block with no children returns an empty slice (not a panic).

### Builder refactor

- Diamond CFG compiled through `SsaBuilder`: phi functions placed at the join block — same result before and after replacing `DominanceFrontier` with `DominatorTree::from_cfg`. Test by constructing a known-SSA-form function and asserting phi count and placement positions are identical.

### Inter-block GVN

- Expression in entry block is not recomputed in a dominated successor (cross-block deduplication fires).
- Expression in one branch of an `if` is NOT eliminated in the sibling branch (siblings do not share tables).
- Expression in the entry block that is followed by a side-effecting instruction in that same block is NOT propagated to children (invalidated entry is removed before propagation).
- `changed = false` when no cross-block duplicates exist.

### LICM

- Loop-invariant `BinaryOp` (no operands defined inside loop) is hoisted to pre-header.
- Loop-variant instruction (operand defined inside loop body) is NOT hoisted.
- Loop with a `Call` instruction: no hoisting occurs for the entire loop.
- Loop with a `CallHandler` instruction: no hoisting occurs for the entire loop.
- Pre-header is inserted immediately before the loop header in `func.blocks` (not at the end).
- Non-back-edge predecessors of header are redirected to pre-header; phi operands in header are re-keyed.
- Dependency order: if invariant instruction A feeds invariant instruction B, A precedes B in the pre-header.
- `changed = false` when no invariant instructions exist.

### Peephole

- `Jump L; Label L` → Jump removed.
- `Jump L1; Label L2` (different labels) → unchanged.
- `Move r0, r0` → removed.
- `Move r0, r1` (different regs) → unchanged.
- `Jump L` at last position (no following instruction) → unchanged, no panic (bounds check respected).
- Empty input → empty output, no panic.
- Mixed stream: only matching patterns removed, others preserved in order.

### Integration

All existing 224 lib tests and 16 round-trip integration tests continue to pass after each change.

---

## File Checklist

| File | Change |
|---|---|
| `src/inklang/ssa/dominance.rs` | New — `DominatorTree` with `from_ssa` and `from_cfg` |
| `src/inklang/ssa/mod.rs` | Re-export `DominatorTree`; add LICM one-shot call inside `run_optimization_passes` (after DCE, before returning) |
| `src/inklang/ssa/passes/mod.rs` | Add `pub mod licm` |
| `src/inklang/ssa/passes/licm.rs` | New — `SsaLicmPass` |
| `src/inklang/ssa/passes/gvn.rs` | Extend with DVNT inter-block propagation; enforce invalidation-removal invariant |
| `src/inklang/ssa/builder.rs` | Delete `DominanceFrontier` struct + impls; remove `dom_frontier` field; build `DominatorTree::from_cfg` once in `build` and pass to `place_phi_functions` + `rename_variables`; update `rename_block` parameter from `&HashMap` to `&DominatorTree`; make `CfgBlock` `pub(super)` |
| `src/inklang/peephole.rs` | New — `peephole_optimize` |
| `src/inklang/mod.rs` | Wire peephole between spill insert and codegen |
