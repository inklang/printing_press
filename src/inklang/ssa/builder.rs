//! SSA builder.
//!
//! Builds SSA form from IR instructions using the Cytron algorithm:
//! 1. Build CFG from IR
//! 2. Compute dominance frontiers
//! 3. Find all registers that are defined
//! 4. Place phi functions at iterated dominance frontiers
//! 5. Rename variables using dominator tree traversal

use super::block::SsaBlock;
use super::function::{PhiFunction, SsaFunction};
use super::value::SsaValue;
use crate::inklang::ir::{IrInstr, IrLabel};
use crate::inklang::value::Value;
use std::collections::{HashMap, HashSet};

/// SSA builder - converts linear IR to SSA form.
pub struct SsaBuilder {
    /// Instructions to convert.
    instrs: Vec<IrInstr>,
    /// Constants table.
    constants: Vec<Value>,
    /// Function arity (number of parameters).
    arity: usize,
    /// CFG blocks.
    cfg_blocks: Vec<CfgBlock>,
    /// Dominance frontier computation.
    dom_frontier: DominanceFrontier,
    /// SSA blocks being built.
    ssa_blocks: Vec<SsaBlock>,
    /// Map from IR block ID to SSA block index.
    block_map: HashMap<usize, usize>,
    /// Set of registers that are defined (need phi functions).
    global_regs: HashSet<usize>,
    /// Blocks where each register is defined.
    def_blocks: HashMap<usize, HashSet<usize>>,
}

/// A basic block in the control flow graph.
#[derive(Debug, Clone)]
struct CfgBlock {
    id: usize,
    label: Option<IrLabel>,
    instrs: Vec<IrInstr>,
    predecessors: Vec<usize>,
    successors: Vec<usize>,
}

impl CfgBlock {
    fn new(id: usize, label: Option<IrLabel>) -> Self {
        CfgBlock {
            id,
            label,
            instrs: Vec::new(),
            predecessors: Vec::new(),
            successors: Vec::new(),
        }
    }
}

/// Dominance frontier computation.
#[derive(Debug)]
struct DominanceFrontier {
    /// Immediate dominators for each block.
    idoms: HashMap<usize, usize>,
    /// Dominance frontiers for each block.
    frontiers: HashMap<usize, HashSet<usize>>,
}

impl DominanceFrontier {
    /// Compute dominance frontiers for a CFG.
    fn compute(cfg_blocks: &[CfgBlock], entry_block: usize) -> Self {
        let mut idoms = HashMap::new();
        let mut frontiers = HashMap::new();

        // Initialize frontiers
        for block in cfg_blocks {
            frontiers.insert(block.id, HashSet::new());
        }

        // Initialize idoms
        idoms.insert(entry_block, entry_block);
        for block in cfg_blocks {
            if block.id != entry_block {
                idoms.insert(block.id, entry_block);
            }
        }

        // Iterative refinement for immediate dominators
        let mut changed = true;
        while changed {
            changed = false;
            for block in cfg_blocks {
                if block.id == entry_block {
                    continue;
                }

                // Find intersection of predecessors' dominators
                let intersect_fn = |a: usize, b: usize| -> usize {
                    let mut a = a;
                    let mut b = b;
                    while a != b {
                        while a > b {
                            a = *idoms.get(&a).unwrap_or(&a);
                        }
                        while b > a {
                            b = *idoms.get(&b).unwrap_or(&b);
                        }
                    }
                    a
                };
                let mut new_idom: Option<usize> = None;
                for &pred_id in &block.predecessors {
                    if let Some(&pred_idom) = idoms.get(&pred_id) {
                        new_idom = Some(new_idom.map_or(pred_idom, |cur| intersect_fn(cur, pred_idom)));
                    }
                }

                if let Some(new_idom) = new_idom {
                    if idoms.get(&block.id).copied() != Some(new_idom) {
                        idoms.insert(block.id, new_idom);
                        changed = true;
                    }
                }
            }
        }

        // Compute dominance frontiers
        for block in cfg_blocks {
            if block.predecessors.len() >= 2 {
                for &pred_id in &block.predecessors {
                    let mut runner = pred_id;
                    let block_idom = *idoms.get(&block.id).unwrap_or(&block.id);
                    while runner != block_idom && runner != entry_block {
                        if let Some(df) = frontiers.get_mut(&runner) {
                            df.insert(block.id);
                        }
                        runner = *idoms.get(&runner).unwrap_or(&entry_block);
                    }
                }
            }
        }

        DominanceFrontier { idoms, frontiers }
    }

    /// Get the dominance frontier for a block.
    fn frontier(&self, block_id: usize) -> HashSet<usize> {
        self.frontiers.get(&block_id).cloned().unwrap_or_default()
    }

    /// Compute the iterated dominance frontier for a set of blocks.
    fn iterated_frontier(&self, blocks: &HashSet<usize>) -> HashSet<usize> {
        let mut result = HashSet::new();
        let mut worklist = blocks.clone();
        let mut processed = HashSet::new();

        while let Some(block_id) = worklist.iter().next().copied() {
            worklist.remove(&block_id);

            if processed.contains(&block_id) {
                continue;
            }
            processed.insert(block_id);

            for &y in &self.frontier(block_id) {
                if result.insert(y) {
                    // If Y is newly added, its DF needs processing too
                    if !blocks.contains(&y) && !processed.contains(&y) {
                        worklist.insert(y);
                    }
                }
            }
        }

        result
    }

    /// Get the dominator tree as a map from block ID to children block IDs.
    fn dominator_tree(&self) -> HashMap<usize, Vec<usize>> {
        let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
        for (&block_id, &idom) in &self.idoms {
            children.entry(block_id).or_default();
            if block_id != idom {
                children.entry(idom).or_default().push(block_id);
            }
        }
        children
    }
}

impl SsaBuilder {
    /// Build SSA form from IR instructions.
    pub fn build(instrs: Vec<IrInstr>, constants: Vec<Value>, arity: usize) -> SsaFunction {
        let mut builder = SsaBuilder {
            instrs,
            constants,
            arity,
            cfg_blocks: Vec::new(),
            dom_frontier: DominanceFrontier {
                idoms: HashMap::new(),
                frontiers: HashMap::new(),
            },
            ssa_blocks: Vec::new(),
            block_map: HashMap::new(),
            global_regs: HashSet::new(),
            def_blocks: HashMap::new(),
        };

        if builder.instrs.is_empty() {
            return SsaFunction::new(Vec::new(), builder.constants, 0, Vec::new(), arity);
        }

        // Step 1: Build CFG from IR
        builder.build_cfg();

        // Step 2: Compute dominance frontiers
        let entry_block = builder.cfg_blocks.first().map(|b| b.id).unwrap_or(0);
        builder.dom_frontier = DominanceFrontier::compute(&builder.cfg_blocks, entry_block);

        // Step 3: Create SSA blocks with empty phi lists
        builder.create_ssa_blocks();

        // Step 4: Find global registers and their definition blocks
        builder.analyze_definitions();

        // Step 5: Place phi functions
        builder.place_phi_functions();

        // Step 6: Convert instructions (with placeholder registers)
        builder.convert_instructions();

        // Step 7: Rename variables
        builder.rename_variables();

        // Find entry and exit blocks
        let entry = builder.cfg_blocks.first().map(|b| b.id).unwrap_or(0);
        let exits: Vec<usize> = builder
            .cfg_blocks
            .iter()
            .filter(|b| b.successors.is_empty())
            .map(|b| b.id)
            .collect();

        SsaFunction::new(builder.ssa_blocks, builder.constants, entry, exits, arity)
    }

    /// Build the control flow graph from IR instructions.
    fn build_cfg(&mut self) {
        let mut current_block: Option<CfgBlock> = None;
        let mut block_id_counter = 0;
        let mut label_to_block: HashMap<usize, usize> = HashMap::new();

        // Create entry block
        let entry_id = block_id_counter;
        block_id_counter += 1;
        current_block = Some(CfgBlock::new(entry_id, None));

        for instr in &self.instrs {
            match instr {
                IrInstr::Label { label } => {
                    // Save current block
                    if let Some(block) = current_block.take() {
                        if !block.instrs.is_empty() || block.label.is_some() {
                            self.cfg_blocks.push(block);
                        }
                    }

                    // Check if this label already has a block
                    let label_id = label.0;
                    if let Some(&existing_id) = label_to_block.get(&label_id) {
                        // Add to existing block
                        if let Some(existing) = self.cfg_blocks.iter_mut().find(|b| b.id == existing_id) {
                            existing.instrs.push(IrInstr::Label { label: *label });
                        }
                    } else {
                        // Create new block for this label
                        let new_block_id = block_id_counter;
                        block_id_counter += 1;
                        label_to_block.insert(label_id, new_block_id);
                        let mut block = CfgBlock::new(new_block_id, Some(*label));
                        block.instrs.push(IrInstr::Label { label: *label });
                        self.cfg_blocks.push(block);
                    }
                    current_block = None;
                }
                IrInstr::Jump { target } => {
                    if let Some(block) = current_block.take() {
                        let mut new_block = block;
                        new_block.instrs.push(IrInstr::Jump { target: *target });
                        let target_id = target.0;
                        if !label_to_block.contains_key(&target_id) {
                            label_to_block.insert(target_id, target_id);
                        }
                        new_block.successors.push(target_id);
                        self.cfg_blocks.push(new_block);
                    }
                    current_block = None;
                }
                IrInstr::JumpIfFalse { src, target } => {
                    if let Some(block) = current_block.take() {
                        let mut new_block = block;
                        new_block.instrs.push(IrInstr::JumpIfFalse { src: *src, target: *target });
                        let target_id = target.0;
                        if !label_to_block.contains_key(&target_id) {
                            label_to_block.insert(target_id, target_id);
                        }
                        new_block.successors.push(target_id);
                        self.cfg_blocks.push(new_block);
                    }
                    current_block = None;
                }
                IrInstr::Return { .. } | IrInstr::Break | IrInstr::Next => {
                    if let Some(block) = current_block.take() {
                        let mut new_block = block;
                        new_block.instrs.push(instr.clone());
                        self.cfg_blocks.push(new_block);
                    }
                    current_block = None;
                }
                _ => {
                    if current_block.is_none() {
                        let new_block_id = block_id_counter;
                        block_id_counter += 1;
                        current_block = Some(CfgBlock::new(new_block_id, None));
                    }
                    if let Some(ref mut block) = current_block {
                        block.instrs.push(instr.clone());
                    }
                }
            }
        }

        // Save final block
        if let Some(block) = current_block {
            if !block.instrs.is_empty() {
                self.cfg_blocks.push(block);
            }
        }

        // Ensure all labels have blocks
        let mut missing_blocks: Vec<usize> = Vec::new();
        for block in &self.cfg_blocks {
            for &succ_id in &block.successors {
                if !self.cfg_blocks.iter().any(|b| b.id == succ_id) {
                    missing_blocks.push(succ_id);
                }
            }
        }
        for succ_id in missing_blocks {
            let new_block = CfgBlock::new(succ_id, Some(IrLabel(succ_id)));
            self.cfg_blocks.push(new_block);
        }

        // Set up predecessors - collect all the data first
        let predecessor_updates: Vec<(usize, usize)> = self.cfg_blocks
            .iter()
            .flat_map(|block| {
                block.successors.iter().map(move |&succ_id| (block.id, succ_id))
            })
            .collect();

        for (pred_id, succ_id) in predecessor_updates {
            if let Some(succ) = self.cfg_blocks.iter_mut().find(|b| b.id == succ_id) {
                if !succ.predecessors.contains(&pred_id) {
                    succ.predecessors.push(pred_id);
                }
            }
        }

        // Handle fall-through edges
        for i in 0..self.cfg_blocks.len() {
            let curr_id = self.cfg_blocks[i].id;
            let curr_end = self.cfg_blocks[i].instrs.last();

            // If block doesn't end with a terminal, fall through to next block
            let ends_with_terminal = matches!(
                curr_end,
                Some(IrInstr::Jump { .. })
                    | Some(IrInstr::JumpIfFalse { .. })
                    | Some(IrInstr::Return { .. })
                    | Some(IrInstr::Break)
                    | Some(IrInstr::Next)
            );

            if !ends_with_terminal && i + 1 < self.cfg_blocks.len() {
                let next_id = self.cfg_blocks[i + 1].id;
                if !self.cfg_blocks[i].successors.contains(&next_id) {
                    self.cfg_blocks[i].successors.push(next_id);
                }
                if !self.cfg_blocks[i + 1].predecessors.contains(&curr_id) {
                    self.cfg_blocks[i + 1].predecessors.push(curr_id);
                }
            }
        }
    }

    /// Create SSA blocks corresponding to IR blocks.
    fn create_ssa_blocks(&mut self) {
        for block in &self.cfg_blocks {
            let ssa_block = SsaBlock::new(block.id, block.label);
            self.ssa_blocks.push(ssa_block);
            self.block_map.insert(block.id, self.ssa_blocks.len() - 1);
        }

        // Set up predecessors and successors
        for (i, block) in self.cfg_blocks.iter().enumerate() {
            self.ssa_blocks[i].predecessors.extend_from_slice(&block.predecessors);
            self.ssa_blocks[i].successors.extend_from_slice(&block.successors);
        }
    }

    /// Analyze which registers are defined in which blocks.
    fn analyze_definitions(&mut self) {
        // Track where each register is defined
        for block in &self.cfg_blocks {
            for instr in &block.instrs {
                if let Some(def_reg) = Self::get_defined_reg(instr) {
                    self.global_regs.insert(def_reg);
                    self.def_blocks.entry(def_reg).or_default().insert(block.id);
                }
            }
        }

        // Also track which registers are used across blocks
        let mut used_in_block = HashMap::new();

        for block in &self.cfg_blocks {
            let mut used = HashSet::new();
            for instr in &block.instrs {
                used.extend(Self::get_used_regs(instr));
            }
            used_in_block.insert(block.id, used);
        }

        // A register is global if it's used in a different block than where it's defined
        // or defined in multiple blocks
        let to_remove: Vec<usize> = self
            .global_regs
            .iter()
            .filter(|&reg| {
                let def_block_set = self.def_blocks.get(reg);
                if let Some(def_set) = def_block_set {
                    if def_set.len() <= 1 {
                        let def_block_id = *def_set.iter().next().unwrap_or(&0);
                        for (&block_id, used) in &used_in_block {
                            if used.contains(reg) && block_id != def_block_id {
                                return false; // Is global
                            }
                        }
                        return true; // Not global
                    }
                }
                false // Is global (defined in multiple blocks)
            })
            .copied()
            .collect();

        for reg in to_remove {
            self.global_regs.remove(&reg);
        }
    }

    /// Place phi functions at iterated dominance frontiers for each global register.
    fn place_phi_functions(&mut self) {
        for &reg in &self.global_regs {
            let def_block_set: HashSet<usize> = self.def_blocks.get(&reg).cloned().unwrap_or_default();
            let idf = self.dom_frontier.iterated_frontier(&def_block_set);

            for &block_id in &idf {
                // Add phi if block has at least one predecessor
                if let Some(&ssa_block_idx) = self.block_map.get(&block_id) {
                    let ssa_block = &mut self.ssa_blocks[ssa_block_idx];
                    if ssa_block.predecessors.len() >= 1 {
                        // Create phi with placeholder operands (will be filled during renaming)
                        let mut operands = HashMap::new();
                        for &pred_id in &ssa_block.predecessors {
                            operands.insert(pred_id, SsaValue::new(reg, 0)); // Placeholder version
                        }

                        let phi = PhiFunction::new(SsaValue::new(reg, 0), operands);
                        ssa_block.phi_functions.push(phi);
                    }
                }
            }
        }
    }

    /// Convert IR instructions to SSA instructions with placeholder registers.
    fn convert_instructions(&mut self) {
        for (i, block) in self.cfg_blocks.iter().enumerate() {
            for instr in &block.instrs {
                if let Some(ssa_instr) = convert_instr(instr) {
                    self.ssa_blocks[i].instrs.push(ssa_instr);
                }
            }
        }
    }

    /// Rename variables using dominator tree traversal.
    fn rename_variables(&mut self) {
        let entry_block = self.cfg_blocks.first().map(|b| b.id).unwrap_or(0);

        // Counters for each register (next version to assign)
        let mut counters: HashMap<usize, usize> = HashMap::new();
        // Stack of current versions for each register
        let mut stacks: HashMap<usize, Vec<usize>> = HashMap::new();

        // Initialize counters and stacks for global registers
        for &reg in &self.global_regs {
            counters.insert(reg, 0);
            stacks.insert(reg, Vec::new());
        }

        // Also track parameters
        for i in 0..self.arity {
            counters.insert(i, 0);
            stacks.insert(i, vec![0]); // Parameters start at version 0
        }

        let dom_tree = self.dom_frontier.dominator_tree();
        self.rename_block(
            entry_block,
            &dom_tree,
            &mut counters,
            &mut stacks,
        );
    }

    /// Rename variables in a single block.
    fn rename_block(
        &mut self,
        block_id: usize,
        dom_tree: &HashMap<usize, Vec<usize>>,
        counters: &mut HashMap<usize, usize>,
        stacks: &mut HashMap<usize, Vec<usize>>,
    ) {
        let mut push_counts: HashMap<usize, usize> = HashMap::new();

        // Find the SSA block index
        let ssa_block_idx = match self.block_map.get(&block_id) {
            Some(&idx) => idx,
            None => return,
        };

        // Process phi functions first
        for phi in &mut self.ssa_blocks[ssa_block_idx].phi_functions {
            let base_reg = phi.result.base_reg;
            let new_version = Self::new_version_internal(base_reg, counters, stacks);
            let mut new_operands = HashMap::new();
            for (&pred_id, _) in &phi.operands {
                // Use current version from predecessor's stack
                let pred_version = stacks.get(&base_reg).and_then(|s| s.last()).copied().unwrap_or(0);
                new_operands.insert(pred_id, SsaValue::new(base_reg, pred_version));
            }
            phi.result = SsaValue::new(base_reg, new_version);
            phi.operands = new_operands;
            *push_counts.entry(base_reg).or_default() += 1;
        }

        // Process instructions
        for instr in &mut self.ssa_blocks[ssa_block_idx].instrs {
            Self::rename_instr_internal(instr, &mut push_counts, counters, stacks);
        }

        // Update phi operands in successor blocks
        let successors: Vec<usize> = self.ssa_blocks[ssa_block_idx].successors.clone();
        for &succ_id in &successors {
            if let Some(&succ_idx) = self.block_map.get(&succ_id) {
                for phi in &mut self.ssa_blocks[succ_idx].phi_functions {
                    let base_reg = phi.result.base_reg;
                    if let Some(&current_version) = stacks.get(&base_reg).and_then(|s| s.last()) {
                        phi.operands.insert(block_id, SsaValue::new(base_reg, current_version));
                    }
                }
            }
        }

        // Recursively process children in dominator tree
        if let Some(children) = dom_tree.get(&block_id) {
            for &child_id in children {
                self.rename_block(child_id, dom_tree, counters, stacks);
            }
        }

        // Pop stacks after all dominated children have been processed
        for (&reg, &count) in &push_counts {
            if let Some(stack) = stacks.get_mut(&reg) {
                for _ in 0..count {
                    stack.pop();
                }
            }
        }
    }

    /// Generate a new version for a register and push it onto the stack.
    fn new_version_internal(base_reg: usize, counters: &mut HashMap<usize, usize>, stacks: &mut HashMap<usize, Vec<usize>>) -> usize {
        let counter = *counters.get(&base_reg).unwrap_or(&0);
        counters.insert(base_reg, counter + 1);
        stacks.entry(base_reg).or_default().push(counter);
        counter
    }

    /// Rename operands in an instruction.
    fn rename_instr_internal(
        instr: &mut super::block::SsaInstr,
        push_counts: &mut HashMap<usize, usize>,
        counters: &mut HashMap<usize, usize>,
        stacks: &mut HashMap<usize, Vec<usize>>,
    ) {
        use super::block::SsaInstr;
        use super::value::SsaValue;

        fn get_current_value(base_reg: usize, stacks: &HashMap<usize, Vec<usize>>) -> SsaValue {
            if let Some(stack) = stacks.get(&base_reg) {
                if let Some(&version) = stack.last() {
                    return SsaValue::new(base_reg, version);
                }
            }
            SsaValue::new(base_reg, 0)
        }

        fn new_version(base_reg: usize, counters: &mut HashMap<usize, usize>, stacks: &mut HashMap<usize, Vec<usize>>) -> usize {
            let counter = *counters.get(&base_reg).unwrap_or(&0);
            counters.insert(base_reg, counter + 1);
            stacks.entry(base_reg).or_default().push(counter);
            counter
        }

        match instr {
            SsaInstr::LoadImm { defined_value, .. } => {
                let base_reg = defined_value.base_reg;
                let new_version = new_version(base_reg, counters, stacks);
                *defined_value = SsaValue::new(base_reg, new_version);
                *push_counts.entry(base_reg).or_default() += 1;
            }
            SsaInstr::LoadGlobal { defined_value, .. } => {
                let base_reg = defined_value.base_reg;
                let new_version = new_version(base_reg, counters, stacks);
                *defined_value = SsaValue::new(base_reg, new_version);
                *push_counts.entry(base_reg).or_default() += 1;
            }
            SsaInstr::StoreGlobal { src, .. } => {
                *src = get_current_value(src.base_reg, stacks);
            }
            SsaInstr::BinaryOp { defined_value, src1, src2, .. } => {
                let new_src1 = get_current_value(src1.base_reg, stacks);
                let new_src2 = get_current_value(src2.base_reg, stacks);
                *src1 = new_src1;
                *src2 = new_src2;
                let base_reg = defined_value.base_reg;
                let new_version = new_version(base_reg, counters, stacks);
                *defined_value = SsaValue::new(base_reg, new_version);
                *push_counts.entry(base_reg).or_default() += 1;
            }
            SsaInstr::UnaryOp { defined_value, src, .. } => {
                let new_src = get_current_value(src.base_reg, stacks);
                *src = new_src;
                let base_reg = defined_value.base_reg;
                let new_version = new_version(base_reg, counters, stacks);
                *defined_value = SsaValue::new(base_reg, new_version);
                *push_counts.entry(base_reg).or_default() += 1;
            }
            SsaInstr::Jump { .. } => {}
            SsaInstr::JumpIfFalse { src, .. } => {
                let new_src = get_current_value(src.base_reg, stacks);
                *src = new_src;
            }
            SsaInstr::Label { .. } => {}
            SsaInstr::LoadFunc { defined_value, .. } => {
                let base_reg = defined_value.base_reg;
                let new_version = new_version(base_reg, counters, stacks);
                *defined_value = SsaValue::new(base_reg, new_version);
                *push_counts.entry(base_reg).or_default() += 1;
            }
            SsaInstr::Call { defined_value, func, args, .. } => {
                let new_func = get_current_value(func.base_reg, stacks);
                *func = new_func;
                for arg in args {
                    *arg = get_current_value(arg.base_reg, stacks);
                }
                let base_reg = defined_value.base_reg;
                let new_version = new_version(base_reg, counters, stacks);
                *defined_value = SsaValue::new(base_reg, new_version);
                *push_counts.entry(base_reg).or_default() += 1;
            }
            SsaInstr::Return { src } => {
                *src = get_current_value(src.base_reg, stacks);
            }
            SsaInstr::Move { defined_value, src } => {
                *src = get_current_value(src.base_reg, stacks);
                let base_reg = defined_value.base_reg;
                let new_version = new_version(base_reg, counters, stacks);
                *defined_value = SsaValue::new(base_reg, new_version);
                *push_counts.entry(base_reg).or_default() += 1;
            }
            SsaInstr::GetIndex { defined_value, obj, index, .. } => {
                *obj = get_current_value(obj.base_reg, stacks);
                *index = get_current_value(index.base_reg, stacks);
                let base_reg = defined_value.base_reg;
                let new_version = new_version(base_reg, counters, stacks);
                *defined_value = SsaValue::new(base_reg, new_version);
                *push_counts.entry(base_reg).or_default() += 1;
            }
            SsaInstr::SetIndex { obj, index, src, .. } => {
                *obj = get_current_value(obj.base_reg, stacks);
                *index = get_current_value(index.base_reg, stacks);
                *src = get_current_value(src.base_reg, stacks);
            }
            SsaInstr::NewArray { defined_value, elements, .. } => {
                for elem in elements {
                    *elem = get_current_value(elem.base_reg, stacks);
                }
                let base_reg = defined_value.base_reg;
                let new_version = new_version(base_reg, counters, stacks);
                *defined_value = SsaValue::new(base_reg, new_version);
                *push_counts.entry(base_reg).or_default() += 1;
            }
            SsaInstr::GetField { defined_value, obj, .. } => {
                *obj = get_current_value(obj.base_reg, stacks);
                let base_reg = defined_value.base_reg;
                let new_version = new_version(base_reg, counters, stacks);
                *defined_value = SsaValue::new(base_reg, new_version);
                *push_counts.entry(base_reg).or_default() += 1;
            }
            SsaInstr::SetField { obj, src, .. } => {
                *obj = get_current_value(obj.base_reg, stacks);
                *src = get_current_value(src.base_reg, stacks);
            }
            SsaInstr::NewInstance { defined_value, class_reg, args, .. } => {
                *class_reg = get_current_value(class_reg.base_reg, stacks);
                for arg in args {
                    *arg = get_current_value(arg.base_reg, stacks);
                }
                let base_reg = defined_value.base_reg;
                let new_version = new_version(base_reg, counters, stacks);
                *defined_value = SsaValue::new(base_reg, new_version);
                *push_counts.entry(base_reg).or_default() += 1;
            }
            SsaInstr::IsType { defined_value, src, .. } => {
                *src = get_current_value(src.base_reg, stacks);
                let base_reg = defined_value.base_reg;
                let new_version = new_version(base_reg, counters, stacks);
                *defined_value = SsaValue::new(base_reg, new_version);
                *push_counts.entry(base_reg).or_default() += 1;
            }
            SsaInstr::HasCheck { defined_value, obj, .. } => {
                *obj = get_current_value(obj.base_reg, stacks);
                let base_reg = defined_value.base_reg;
                let new_version = new_version(base_reg, counters, stacks);
                *defined_value = SsaValue::new(base_reg, new_version);
                *push_counts.entry(base_reg).or_default() += 1;
            }
            SsaInstr::LoadClass { defined_value, .. } => {
                let base_reg = defined_value.base_reg;
                let new_version = new_version(base_reg, counters, stacks);
                *defined_value = SsaValue::new(base_reg, new_version);
                *push_counts.entry(base_reg).or_default() += 1;
            }
            SsaInstr::Break | SsaInstr::Next | SsaInstr::CallHandler { .. } => {}
        }
    }

    /// Get the register defined by an instruction, if any.
    fn get_defined_reg(instr: &IrInstr) -> Option<usize> {
        use crate::inklang::ir::IrInstr as IR;
        match instr {
            IR::LoadImm { dst, .. } => Some(*dst),
            IR::LoadGlobal { dst, .. } => Some(*dst),
            IR::BinaryOp { dst, .. } => Some(*dst),
            IR::UnaryOp { dst, .. } => Some(*dst),
            IR::LoadFunc { dst, .. } => Some(*dst),
            IR::Call { dst, .. } => Some(*dst),
            IR::Move { dst, .. } => Some(*dst),
            IR::GetIndex { dst, .. } => Some(*dst),
            IR::NewArray { dst, .. } => Some(*dst),
            IR::GetField { dst, .. } => Some(*dst),
            IR::NewInstance { dst, .. } => Some(*dst),
            IR::IsType { dst, .. } => Some(*dst),
            IR::HasCheck { dst, .. } => Some(*dst),
            IR::LoadClass { dst, .. } => Some(*dst),
            IR::GetUpvalue { dst, .. } => Some(*dst),
            _ => None,
        }
    }

    /// Get all registers used by an instruction.
    fn get_used_regs(instr: &IrInstr) -> Vec<usize> {
        use crate::inklang::ir::IrInstr as IR;
        match instr {
            IR::BinaryOp { src1, src2, .. } => vec![*src1, *src2],
            IR::UnaryOp { src, .. } => vec![*src],
            IR::Call { func, args, .. } => {
                let mut regs = vec![*func];
                regs.extend_from_slice(args);
                regs
            }
            IR::Return { src } => vec![*src],
            IR::JumpIfFalse { src, .. } => vec![*src],
            IR::StoreGlobal { src, .. } => vec![*src],
            IR::Move { src, .. } => vec![*src],
            IR::NewArray { elements, .. } => elements.clone(),
            IR::GetIndex { obj, index, .. } => vec![*obj, *index],
            IR::SetIndex { obj, index, src } => vec![*obj, *index, *src],
            IR::GetField { obj, .. } => vec![*obj],
            IR::SetField { obj, src, .. } => vec![*obj, *src],
            IR::NewInstance { class_reg, args, .. } => {
                let mut regs = vec![*class_reg];
                regs.extend_from_slice(args);
                regs
            }
            IR::IsType { src, .. } => vec![*src],
            IR::HasCheck { obj, .. } => vec![*obj],
            IR::Throw { src } => vec![*src],
            IR::GetUpvalue { dst, .. } => vec![*dst],
            _ => vec![],
        }
    }
}

/// Convert a single IR instruction to SSA form.
fn convert_instr(instr: &IrInstr) -> Option<super::block::SsaInstr> {
    use super::block::SsaInstr;
    use super::value::SsaValue;
    use crate::inklang::ir::IrInstr as IR;

    match instr {
        IR::LoadImm { dst, index } => {
            Some(SsaInstr::LoadImm { defined_value: SsaValue::new(*dst, 0), const_index: *index })
        }
        IR::LoadGlobal { dst, name } => {
            Some(SsaInstr::LoadGlobal { defined_value: SsaValue::new(*dst, 0), name: name.clone() })
        }
        IR::StoreGlobal { name, src } => {
            Some(SsaInstr::StoreGlobal { name: name.clone(), src: SsaValue::new(*src, 0) })
        }
        IR::BinaryOp { dst, op, src1, src2 } => {
            Some(SsaInstr::BinaryOp {
                defined_value: SsaValue::new(*dst, 0),
                op: *op,
                src1: SsaValue::new(*src1, 0),
                src2: SsaValue::new(*src2, 0),
            })
        }
        IR::UnaryOp { dst, op, src } => {
            Some(SsaInstr::UnaryOp {
                defined_value: SsaValue::new(*dst, 0),
                op: *op,
                src: SsaValue::new(*src, 0),
            })
        }
        IR::Jump { target } => Some(SsaInstr::Jump { target: *target }),
        IR::JumpIfFalse { src, target } => {
            Some(SsaInstr::JumpIfFalse { src: SsaValue::new(*src, 0), target: *target })
        }
        IR::Label { label } => Some(SsaInstr::Label { label: *label }),
        IR::LoadFunc { dst, name, arity, instrs, constants, default_values, captured_vars, upvalue_regs } => {
            Some(SsaInstr::LoadFunc {
                defined_value: SsaValue::new(*dst, 0),
                name: name.clone(),
                arity: *arity,
                instrs: instrs.clone(),
                constants: constants.clone(),
                default_values: default_values.clone(),
                captured_vars: captured_vars.clone(),
                upvalue_regs: upvalue_regs.clone(),
            })
        }
        IR::Call { dst, func, args } => {
            Some(SsaInstr::Call {
                defined_value: SsaValue::new(*dst, 0),
                func: SsaValue::new(*func, 0),
                args: args.iter().map(|&r| SsaValue::new(r, 0)).collect(),
            })
        }
        IR::Return { src } => {
            Some(SsaInstr::Return { src: SsaValue::new(*src, 0) })
        }
        IR::Move { dst, src } => {
            Some(SsaInstr::Move {
                defined_value: SsaValue::new(*dst, 0),
                src: SsaValue::new(*src, 0),
            })
        }
        IR::GetIndex { dst, obj, index } => {
            Some(SsaInstr::GetIndex {
                defined_value: SsaValue::new(*dst, 0),
                obj: SsaValue::new(*obj, 0),
                index: SsaValue::new(*index, 0),
            })
        }
        IR::SetIndex { obj, index, src } => {
            Some(SsaInstr::SetIndex {
                obj: SsaValue::new(*obj, 0),
                index: SsaValue::new(*index, 0),
                src: SsaValue::new(*src, 0),
            })
        }
        IR::NewArray { dst, elements } => {
            Some(SsaInstr::NewArray {
                defined_value: SsaValue::new(*dst, 0),
                elements: elements.iter().map(|&r| SsaValue::new(r, 0)).collect(),
            })
        }
        IR::GetField { dst, obj, name } => {
            Some(SsaInstr::GetField {
                defined_value: SsaValue::new(*dst, 0),
                obj: SsaValue::new(*obj, 0),
                name: name.clone(),
            })
        }
        IR::SetField { obj, name, src } => {
            Some(SsaInstr::SetField {
                obj: SsaValue::new(*obj, 0),
                name: name.clone(),
                src: SsaValue::new(*src, 0),
            })
        }
        IR::NewInstance { dst, class_reg, args } => {
            Some(SsaInstr::NewInstance {
                defined_value: SsaValue::new(*dst, 0),
                class_reg: SsaValue::new(*class_reg, 0),
                args: args.iter().map(|&r| SsaValue::new(r, 0)).collect(),
            })
        }
        IR::IsType { dst, src, type_name } => {
            Some(SsaInstr::IsType {
                defined_value: SsaValue::new(*dst, 0),
                src: SsaValue::new(*src, 0),
                type_name: type_name.clone(),
            })
        }
        IR::HasCheck { dst, obj, field_name } => {
            Some(SsaInstr::HasCheck {
                defined_value: SsaValue::new(*dst, 0),
                obj: SsaValue::new(*obj, 0),
                field_name: field_name.clone(),
            })
        }
        IR::LoadClass { dst, name, super_class, methods } => {
            Some(SsaInstr::LoadClass {
                defined_value: SsaValue::new(*dst, 0),
                name: name.clone(),
                super_class: super_class.clone(),
                methods: methods.clone(),
            })
        }
        IR::Break => Some(SsaInstr::Break),
        IR::Next => Some(SsaInstr::Next),
        IR::CallHandler { handler_name, block_bodies } => {
            Some(SsaInstr::CallHandler {
                handler_name: handler_name.clone(),
                block_bodies: block_bodies.clone(),
            })
        }
        // These are not converted to SSA
        IR::Spill { .. } | IR::Unspill { .. } | IR::Throw { .. } |
        IR::RegisterEventHandler { .. } | IR::InvokeEventHandler { .. } |
        IR::AwaitInstr { .. } | IR::SpawnInstr { .. } | IR::AsyncCallInstr { .. } |
        IR::GetUpvalue { .. } => None,
    }
}
