//! SSA (Static Single Assignment) infrastructure.
//!
//! This module provides the SSA building, optimization, and deconstruction
//! pipeline for the Inklang compiler.
//!
//! # SSA Form
//!
//! SSA form is an intermediate representation where each variable is
//! defined exactly once. This simplifies many compiler optimizations
//! and enables powerful optimization passes.
//!
//! # Pipeline
//!
//! 1. **Build**: Convert linear IR to SSA form using the Cytron algorithm
//! 2. **Optimize**: Apply SSA optimization passes (constant propagation, GVN, DCE)
//! 3. **Deconstruct**: Convert SSA form back to linear IR

pub mod builder;
pub mod block;
pub mod deconstructor;
pub mod function;
pub mod passes;
pub mod value;

pub use builder::SsaBuilder;
pub use deconstructor::deconstruct;
pub use function::{PhiFunction, SsaFunction};
pub use value::SsaValue;
pub use passes::{SsaOptPass, SsaOptResult};

use crate::inklang::ir::IrInstr;
use crate::inklang::value::Value;
use passes::constant_propagation::SsaConstantPropagationPass;
use passes::gvn::SsaGlobalValueNumberingPass;
use passes::dce::SsaDeadCodeEliminationPass;

/// Result of an SSA round-trip (build + optimize + deconstruct).
pub struct SsaRoundTripResult {
    /// The deconstructed IR instructions.
    pub instrs: Vec<IrInstr>,
    /// The constants table.
    pub constants: Vec<Value>,
    /// Whether any optimizations were applied.
    pub optimized: bool,
}

/// Run the full SSA pipeline: build SSA, apply optimizations, then deconstruct back to IR.
///
/// This is the main entry point for the SSA infrastructure.
pub fn optimized_ssa_round_trip(
    instrs: Vec<IrInstr>,
    constants: Vec<Value>,
    arity: usize,
) -> SsaRoundTripResult {
    // Step 1: Build SSA form
    let ssa_func = SsaBuilder::build(instrs, constants.clone(), arity);

    // Step 2: Apply optimization passes
    let optimized_func = run_optimization_passes(ssa_func);

    // Save constants before moving the function
    let result_constants = optimized_func.func.constants.clone();

    // Step 3: Deconstruct SSA back to IR
    let final_instrs = deconstruct(optimized_func.func);

    SsaRoundTripResult {
        instrs: final_instrs,
        constants: result_constants,
        optimized: optimized_func.changed,
    }
}

/// Run all SSA optimization passes on a function.
fn run_optimization_passes(mut ssa_func: SsaFunction) -> SsaOptResult {
    let mut optimized = false;

    // Constant propagation
    let cp_pass = &mut SsaConstantPropagationPass::new();
    let result = run_pass(cp_pass, ssa_func);
    ssa_func = result.func;
    optimized = result.changed || optimized;

    // Global Value Numbering
    let gvn_pass = &mut SsaGlobalValueNumberingPass::new();
    let result = run_pass(gvn_pass, ssa_func);
    ssa_func = result.func;
    optimized = result.changed || optimized;

    // Dead Code Elimination
    let dce_pass = &mut SsaDeadCodeEliminationPass::new();
    let result = run_pass(dce_pass, ssa_func);
    ssa_func = result.func;
    optimized = result.changed || optimized;

    SsaOptResult {
        func: ssa_func,
        changed: optimized,
    }
}

/// Run a single optimization pass.
fn run_pass<P: SsaOptPass>(pass: &mut P, mut ssa_func: SsaFunction) -> SsaOptResult {
    let mut iteration_changed = true;
    let mut total_changed = false;

    // Run passes to fixed point
    while iteration_changed {
        let result = pass.run(ssa_func);
        iteration_changed = result.changed;
        total_changed = total_changed || result.changed;
        ssa_func = result.func;
    }

    SsaOptResult {
        func: ssa_func,
        changed: total_changed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inklang::ir::{IrInstr, IrLabel};
    use crate::inklang::token::TokenType;
    use crate::inklang::value::Value;

    #[test]
    fn test_ssa_value() {
        let val = SsaValue::new(5, 3);
        assert_eq!(val.base_reg, 5);
        assert_eq!(val.version, 3);
    }

    #[test]
    fn test_ssa_builder_basic() {
        // Create a simple IR: r0 = 1; r1 = 2
        let constants = vec![Value::Int(1), Value::Int(2)];
        let instrs = vec![
            IrInstr::LoadImm { dst: 0, index: 0 }, // r0 = 1
            IrInstr::LoadImm { dst: 1, index: 1 }, // r1 = 2
        ];

        // Build SSA
        let ssa_func = SsaBuilder::build(instrs, constants, 0);

        // Should have at least one block
        assert!(!ssa_func.blocks.is_empty());
        assert_eq!(ssa_func.blocks.len(), 1);
    }

    #[test]
    fn test_ssa_phi_function() {
        use std::collections::HashMap;

        let phi = PhiFunction::new(
            SsaValue::new(0, 0),
            vec![(0, SsaValue::new(1, 0)), (1, SsaValue::new(2, 0))]
                .into_iter()
                .collect(),
        );

        assert_eq!(phi.result.base_reg, 0);
        assert_eq!(phi.result.version, 0);
        assert_eq!(phi.operands.len(), 2);
    }

    #[test]
    fn test_ssa_deconstruct_single_loadimm() {
        // Single instruction: r0 = LoadImm{index: 0}
        let constants = vec![Value::Int(5)];
        let instrs = vec![
            IrInstr::LoadImm { dst: 0, index: 0 },
        ];

        let ssa_func = SsaBuilder::build(instrs.clone(), constants.clone(), 0);
        eprintln!("SSA blocks: {:?}", ssa_func.blocks.len());
        for (i, block) in ssa_func.blocks.iter().enumerate() {
            eprintln!("  block {}: label={:?}, phi_count={}, instr_count={}",
                i, block.label, block.phi_functions.len(), block.instrs.len());
            for instr in &block.instrs {
                eprintln!("    instr: {:?}", instr);
            }
        }

        let result = deconstruct(ssa_func);
        eprintln!("Deconstructed: {:?}", result);
        assert!(!result.is_empty(), "Deconstruct of single LoadImm should not be empty");
    }

    #[test]
    fn test_ssa_round_trip_single_loadimm() {
        // Reproduce exactly what the failing test does
        let constants = vec![Value::Int(5)];
        let instrs = vec![
            IrInstr::LoadImm { dst: 0, index: 0 },
        ];

        let result = optimized_ssa_round_trip(instrs, constants, 0);
        eprintln!("Round-trip result: {} instrs, {:?}", result.instrs.len(), result.instrs);
        assert!(!result.instrs.is_empty(), "Round-trip should not produce empty result");
    }

}
