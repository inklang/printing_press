//! SSA optimization passes.
//!
//! Contains various SSA-based optimization passes that can be applied
//! to SSA functions to improve code quality before deconstruction.

pub mod constant_propagation;
pub mod gvn;
pub mod dce;

use super::function::SsaFunction;

/// Result of running an SSA optimization pass.
#[derive(Debug)]
pub struct SsaOptResult {
    /// The optimized SSA function.
    pub func: SsaFunction,
    /// Whether any changes were made.
    pub changed: bool,
}

/// Trait for SSA optimization passes.
pub trait SsaOptPass {
    /// Get the name of this pass.
    fn name(&self) -> &str;

    /// Run the optimization pass on the given SSA function.
    fn run(&mut self, ssa_func: SsaFunction) -> SsaOptResult;
}
