//! SSA deconstructor.
//!
//! Deconstructs SSA form back to normal IR.
//! Algorithm:
//! 1. Resolve phi functions into copy pairs keyed by predecessor block ID
//! 2. Assign unique registers to all SSA values (including phi operands)
//! 3. Emit IR with phi-resolution moves inserted in predecessor blocks before terminals
//! 4. Sequentialize parallel copies to handle dependencies

use super::function::SsaFunction;
use super::value::SsaValue;
use crate::inklang::ir::IrInstr;
use std::collections::HashMap;

/// SSA deconstructor - converts SSA form back to linear IR.
pub struct SsaDeconstructor {
    /// The SSA function being deconstructed.
    ssa_func: SsaFunction,
    /// Map from (baseReg, version) to a new register number.
    reg_map: HashMap<(usize, usize), usize>,
    /// Next available register number.
    next_reg: usize,
    /// Stores (dstSsaValue, srcSsaValue) per predecessor block.
    phi_copies: HashMap<usize, Vec<(SsaValue, SsaValue)>>,
}

impl SsaDeconstructor {
    /// Create a new SSA deconstructor.
    pub fn new(ssa_func: SsaFunction) -> Self {
        SsaDeconstructor {
            ssa_func,
            reg_map: HashMap::new(),
            next_reg: 0,
            phi_copies: HashMap::new(),
        }
    }

    /// Deconstruct SSA form back to IR instructions.
    pub fn deconstruct(&mut self) -> Vec<IrInstr> {
        if self.ssa_func.blocks.is_empty() {
            return Vec::new();
        }

        // Step 1: Resolve phis - collect copy pairs per predecessor
        self.resolve_phis();

        // Step 2: Assign registers (covers all SSA values including phi operands)
        self.assign_registers();

        // Step 3: Emit IR with phi-resolution moves in predecessor blocks
        let mut result = Vec::new();
        let blocks = self.ssa_func.blocks.clone();

        for block in &blocks {
            if let Some(label) = block.label {
                result.push(IrInstr::Label { label });
            }

            let instr_count = block.instrs.len();
            for (i, instr) in block.instrs.iter().enumerate() {
                let is_terminal = i == instr_count.saturating_sub(1) && self.is_terminal_instr(instr);

                // Insert phi-resolution moves before the terminal instruction
                if is_terminal {
                    self.emit_phi_moves(block.id, &mut result);
                }

                if let Some(ir_instr) = self.convert_instr(instr) {
                    result.push(ir_instr);
                }
            }

            // Fallthrough blocks: emit phi moves at end
            if block.instrs.is_empty() || !self.is_terminal_instr(block.instrs.last().unwrap()) {
                self.emit_phi_moves(block.id, &mut result);
            }
        }

        result
    }

    /// Resolve phi functions into copy pairs per predecessor block.
    fn resolve_phis(&mut self) {
        for block in &self.ssa_func.blocks {
            if block.phi_functions.is_empty() {
                continue;
            }
            for phi in &block.phi_functions {
                for (&pred_id, &src_value) in &phi.operands {
                    if src_value.base_reg == 0 && src_value.version == 0 {
                        // Skip undefined values
                        continue;
                    }
                    self.phi_copies
                        .entry(pred_id)
                        .or_default()
                        .push((phi.result, src_value));
                }
            }
        }
    }

    /// Emit phi-resolution moves for a given block.
    fn emit_phi_moves(&self, block_id: usize, result: &mut Vec<IrInstr>) {
        if let Some(copies) = self.phi_copies.get(&block_id) {
            let moves = self.sequentialize_copies_internal(copies);
            result.extend(moves);
        }
    }

    /// Sequentialize parallel copies to handle dependencies.
    fn sequentialize_copies_internal(&self, copies: &[(SsaValue, SsaValue)]) -> Vec<IrInstr> {
        let mut moves = Vec::new();
        let mut emitted = vec![false; copies.len()];

        // First pass: emit non-conflicting moves
        let mut changed = true;
        while changed {
            changed = false;
            for (i, &(dst, src)) in copies.iter().enumerate() {
                if emitted[i] {
                    continue;
                }
                let dst_reg = self.map_reg(&dst);
                let src_reg = self.map_reg(&src);
                if dst_reg == src_reg {
                    emitted[i] = true;
                    changed = true;
                    continue;
                }
                let conflicts_with_other = copies
                    .iter()
                    .enumerate()
                    .any(|(j, &(other_dst, _))| !emitted[j] && j != i && self.map_reg(&other_dst) == dst_reg);
                if !conflicts_with_other {
                    moves.push(IrInstr::Move {
                        dst: dst_reg,
                        src: src_reg,
                    });
                    emitted[i] = true;
                    changed = true;
                }
            }
        }

        // Second pass: handle circular dependencies with temp register
        let remaining: Vec<usize> = (0..copies.len()).filter(|&i| !emitted[i]).collect();
        if !remaining.is_empty() {
            let first_idx = remaining[0];
            let (first_dst, first_src) = copies[first_idx];
            let first_src_reg = self.map_reg(&first_src);
            let first_dst_reg = self.map_reg(&first_dst);
            let temp_reg = self.next_reg;
            moves.push(IrInstr::Move {
                dst: temp_reg,
                src: first_src_reg,
            });

            let mut current_dst = first_dst_reg;
            let mut chain_emitted = vec![first_idx];
            let mut found_next = true;
            while found_next {
                found_next = false;
                for &j in &remaining {
                    if chain_emitted.contains(&j) {
                        continue;
                    }
                    let (dst, src) = copies[j];
                    if self.map_reg(&src) == current_dst {
                        moves.push(IrInstr::Move {
                            dst: self.map_reg(&dst),
                            src: self.map_reg(&src),
                        });
                        current_dst = self.map_reg(&dst);
                        chain_emitted.push(j);
                        found_next = true;
                        break;
                    }
                }
            }
            moves.push(IrInstr::Move {
                dst: first_dst_reg,
                src: temp_reg,
            });

            // Any remaining non-cycle copies
            for &j in &remaining {
                if chain_emitted.contains(&j) {
                    continue;
                }
                let (dst, src) = copies[j];
                let dst_reg = self.map_reg(&dst);
                let src_reg = self.map_reg(&src);
                if dst_reg != src_reg {
                    moves.push(IrInstr::Move { dst: dst_reg, src: src_reg });
                }
            }
        }

        moves
    }

    /// Check if an instruction is a terminal (block-ending) instruction.
    fn is_terminal_instr(&self, instr: &super::block::SsaInstr) -> bool {
        use super::block::SsaInstr;
        matches!(
            instr,
            SsaInstr::Jump { .. }
                | SsaInstr::JumpIfFalse { .. }
                | SsaInstr::Return { .. }
                | SsaInstr::Break
                | SsaInstr::Next
        )
    }

    /// Assign register numbers to all SSA values.
    fn assign_registers(&mut self) {
        let mut all_values = Vec::new();

        for block in &self.ssa_func.blocks {
            for phi in &block.phi_functions {
                all_values.push(phi.result);
                for &operand in phi.operands.values() {
                    all_values.push(operand);
                }
            }
            for instr in &block.instrs {
                if let Some(def_val) = instr.defined_value() {
                    all_values.push(def_val);
                }
                for used_val in instr.used_values() {
                    all_values.push(used_val);
                }
            }
        }

        // Also collect SSA values from phi copies
        for copies in self.phi_copies.values() {
            for &(dst, src) in copies {
                all_values.push(dst);
                all_values.push(src);
            }
        }

        // Pre-assign parameter registers: SsaValue(i, 0) -> register i for i in 0..arity-1
        let arity = self.ssa_func.arity;
        for i in 0..arity {
            self.reg_map.insert((i, 0), i);
        }
        self.next_reg = arity;

        // Each unique (baseReg, version) pair gets its own register
        for value in all_values {
            let key = (value.base_reg, value.version);
            if !self.reg_map.contains_key(&key) {
                self.reg_map.insert(key, self.next_reg);
                self.next_reg += 1;
            }
        }

        // Handle undefined values
        if !self.reg_map.contains_key(&(0, 0)) {
            self.reg_map.insert((0, 0), self.next_reg);
            self.next_reg += 1;
        }
    }

    /// Convert an SSA instruction to an IR instruction.
    fn convert_instr(&self, instr: &super::block::SsaInstr) -> Option<IrInstr> {
        use super::block::SsaInstr;
        use crate::inklang::ir::IrInstr as IR;

        match instr {
            SsaInstr::LoadImm { defined_value, const_index } => {
                Some(IR::LoadImm {
                    dst: self.map_reg(defined_value),
                    index: *const_index,
                })
            }
            SsaInstr::LoadGlobal { defined_value, name } => {
                Some(IR::LoadGlobal {
                    dst: self.map_reg(defined_value),
                    name: name.clone(),
                })
            }
            SsaInstr::StoreGlobal { name, src } => {
                Some(IR::StoreGlobal {
                    name: name.clone(),
                    src: self.map_reg(src),
                })
            }
            SsaInstr::BinaryOp { defined_value, op, src1, src2 } => {
                Some(IR::BinaryOp {
                    dst: self.map_reg(defined_value),
                    op: *op,
                    src1: self.map_reg(src1),
                    src2: self.map_reg(src2),
                })
            }
            SsaInstr::UnaryOp { defined_value, op, src } => {
                Some(IR::UnaryOp {
                    dst: self.map_reg(defined_value),
                    op: *op,
                    src: self.map_reg(src),
                })
            }
            SsaInstr::Jump { target } => Some(IR::Jump { target: *target }),
            SsaInstr::JumpIfFalse { src, target } => {
                Some(IR::JumpIfFalse {
                    src: self.map_reg(src),
                    target: *target,
                })
            }
            SsaInstr::Label { label } => Some(IR::Label { label: *label }),
            SsaInstr::LoadFunc { defined_value, name, arity, instrs, constants, default_values, captured_vars, upvalue_regs } => {
                Some(IR::LoadFunc {
                    dst: self.map_reg(defined_value),
                    name: name.clone(),
                    arity: *arity,
                    instrs: instrs.clone(),
                    constants: constants.clone(),
                    default_values: default_values.clone(),
                    captured_vars: captured_vars.clone(),
                    upvalue_regs: upvalue_regs.clone(),
                })
            }
            SsaInstr::Call { defined_value, func, args } => {
                Some(IR::Call {
                    dst: self.map_reg(defined_value),
                    func: self.map_reg(func),
                    args: args.iter().map(|v| self.map_reg(v)).collect(),
                })
            }
            SsaInstr::Return { src } => {
                Some(IR::Return {
                    src: self.map_reg(src),
                })
            }
            SsaInstr::Move { defined_value, src } => {
                let dst_reg = self.map_reg(defined_value);
                let src_reg = self.map_reg(src);
                if dst_reg != src_reg {
                    Some(IR::Move { dst: dst_reg, src: src_reg })
                } else {
                    None
                }
            }
            SsaInstr::GetIndex { defined_value, obj, index } => {
                Some(IR::GetIndex {
                    dst: self.map_reg(defined_value),
                    obj: self.map_reg(obj),
                    index: self.map_reg(index),
                })
            }
            SsaInstr::SetIndex { obj, index, src } => {
                Some(IR::SetIndex {
                    obj: self.map_reg(obj),
                    index: self.map_reg(index),
                    src: self.map_reg(src),
                })
            }
            SsaInstr::NewArray { defined_value, elements } => {
                Some(IR::NewArray {
                    dst: self.map_reg(defined_value),
                    elements: elements.iter().map(|v| self.map_reg(v)).collect(),
                })
            }
            SsaInstr::GetField { defined_value, obj, name } => {
                Some(IR::GetField {
                    dst: self.map_reg(defined_value),
                    obj: self.map_reg(obj),
                    name: name.clone(),
                })
            }
            SsaInstr::SetField { obj, name, src } => {
                Some(IR::SetField {
                    obj: self.map_reg(obj),
                    name: name.clone(),
                    src: self.map_reg(src),
                })
            }
            SsaInstr::NewInstance { defined_value, class_reg, args } => {
                Some(IR::NewInstance {
                    dst: self.map_reg(defined_value),
                    class_reg: self.map_reg(class_reg),
                    args: args.iter().map(|v| self.map_reg(v)).collect(),
                })
            }
            SsaInstr::IsType { defined_value, src, type_name } => {
                Some(IR::IsType {
                    dst: self.map_reg(defined_value),
                    src: self.map_reg(src),
                    type_name: type_name.clone(),
                })
            }
            SsaInstr::LoadClass { defined_value, name, super_class, methods } => {
                Some(IR::LoadClass {
                    dst: self.map_reg(defined_value),
                    name: name.clone(),
                    super_class: super_class.clone(),
                    methods: methods.clone(),
                })
            }
            SsaInstr::Break => Some(IR::Break),
            SsaInstr::Next => Some(IR::Next),
            SsaInstr::CallHandler { handler_name, block_bodies } => {
                Some(IR::CallHandler {
                    handler_name: handler_name.clone(),
                    block_bodies: block_bodies.clone(),
                })
            }
        }
    }

    /// Map an SSA value to a register number.
    fn map_reg(&self, value: &SsaValue) -> usize {
        *self.reg_map.get(&(value.base_reg, value.version)).unwrap_or(&0)
    }
}

/// Deconstruct SSA form to IR instructions.
pub fn deconstruct(ssa_func: SsaFunction) -> Vec<IrInstr> {
    SsaDeconstructor::new(ssa_func).deconstruct()
}
