//! SSA function representation.
//!
//! Represents a function in SSA form containing SSA blocks,
//! constants, and metadata.

use super::block::SsaBlock;
use super::value::SsaValue;
use crate::inklang::value::Value;
use std::collections::HashMap;
use std::fmt;

/// Phi function for merging values at control flow joins.
/// result = phi(src1 from block1, src2 from block2, ...)
///
/// In SSA form, phi functions appear at the start of blocks that have
/// multiple predecessors. Each phi selects the value from the predecessor
/// that was actually taken at runtime.
#[derive(Debug, Clone)]
pub struct PhiFunction {
    /// The SSA value defined by this phi.
    pub result: SsaValue,
    /// Operands keyed by predecessor block ID.
    /// Maps predecessor block ID -> value from that predecessor.
    pub operands: HashMap<usize, SsaValue>,
}

impl PhiFunction {
    /// Create a new phi function with the given result and operands.
    pub fn new(result: SsaValue, operands: HashMap<usize, SsaValue>) -> Self {
        PhiFunction { result, operands }
    }

    /// Get the operand for a specific predecessor block.
    pub fn operand_for(&self, block_id: usize) -> Option<SsaValue> {
        self.operands.get(&block_id).copied()
    }

    /// Create a new phi with an operand replaced.
    pub fn with_operand(&self, block_id: usize, value: SsaValue) -> Self {
        let mut new_operands = self.operands.clone();
        new_operands.insert(block_id, value);
        PhiFunction {
            result: self.result,
            operands: new_operands,
        }
    }
}

impl fmt::Display for PhiFunction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let operand_str = self
            .operands
            .iter()
            .map(|(block_id, value)| format!("{} from B{}", value, block_id))
            .collect::<Vec<_>>()
            .join(", ");
        write!(f, "{} = phi({})", self.result, operand_str)
    }
}

/// Represents a function in SSA form.
/// Contains SSA blocks, constants, and metadata.
#[derive(Debug, Clone)]
pub struct SsaFunction {
    /// SSA blocks in this function.
    pub blocks: Vec<SsaBlock>,
    /// Constants used by this function.
    pub constants: Vec<Value>,
    /// Entry block ID.
    pub entry_block: usize,
    /// Exit block IDs.
    pub exit_blocks: Vec<usize>,
    /// Number of function parameters (registers 0..arity-1).
    pub arity: usize,
}

impl SsaFunction {
    /// Create a new SSA function.
    pub fn new(
        blocks: Vec<SsaBlock>,
        constants: Vec<Value>,
        entry_block: usize,
        exit_blocks: Vec<usize>,
        arity: usize,
    ) -> Self {
        SsaFunction {
            blocks,
            constants,
            entry_block,
            exit_blocks,
            arity,
        }
    }

    /// Get an SSA block by its ID.
    pub fn get_block(&self, id: usize) -> Option<&SsaBlock> {
        self.blocks.iter().find(|b| b.id == id)
    }

    /// Get a mutable SSA block by its ID.
    pub fn get_block_mut(&mut self, id: usize) -> Option<&mut SsaBlock> {
        self.blocks.iter_mut().find(|b| b.id == id)
    }

    /// Get all phi functions in the function.
    pub fn all_phi_functions(&self) -> Vec<(usize, &PhiFunction)> {
        self.blocks
            .iter()
            .flat_map(|block| block.phi_functions.iter().map(move |phi| (block.id, phi)))
            .collect()
    }

    /// Build a def map: for each SsaValue, find the instruction/block that defines it.
    pub fn build_def_map(&self) -> HashMap<SsaValue, (usize, Option<usize>)> {
        let mut def_map = HashMap::new();

        for block in &self.blocks {
            // Phi definitions
            for phi in &block.phi_functions {
                def_map.insert(phi.result, (block.id, None));
            }
            // Regular instruction definitions
            for (instr_idx, instr) in block.instrs.iter().enumerate() {
                if let Some(def_val) = instr.defined_value() {
                    def_map.insert(def_val, (block.id, Some(instr_idx)));
                }
            }
        }

        def_map
    }

    /// Build a use map: for each SsaValue, list all (block_id, instruction_index) that use it.
    pub fn build_use_map(&self) -> HashMap<SsaValue, Vec<(usize, usize)>> {
        let mut use_map: HashMap<SsaValue, Vec<(usize, usize)>> = HashMap::new();

        for block in &self.blocks {
            // Phi uses
            for phi in &block.phi_functions {
                for used_val in phi.operands.values() {
                    use_map.entry(*used_val).or_default().push((block.id, 0));
                }
            }
            // Regular instruction uses
            for (instr_idx, instr) in block.instrs.iter().enumerate() {
                for used_val in instr.used_values() {
                    use_map.entry(used_val).or_default().push((block.id, instr_idx));
                }
            }
        }

        use_map
    }

    /// Dump the SSA function for debugging.
    pub fn dump(&self) -> String {
        let mut sb = String::new();
        sb.push_str("SSA Function:\n");
        sb.push_str(&format!("  Entry: Block{}\n", self.entry_block));
        sb.push_str(&format!(
            "  Exits: {:?}\n",
            self.exit_blocks.iter().map(|id| format!("Block{}", id)).collect::<Vec<_>>()
        ));
        sb.push('\n');

        for block in &self.blocks {
            sb.push_str(&format!("  Block{}:\n", block.id));
            if let Some(label) = block.label {
                sb.push_str(&format!("    Label: L{}\n", label.0));
            }
            sb.push_str(&format!("    Predecessors: {:?}\n", block.predecessors));
            sb.push_str(&format!("    Successors: {:?}\n", block.successors));

            if !block.phi_functions.is_empty() {
                sb.push_str("    Phi Functions:\n");
                for phi in &block.phi_functions {
                    sb.push_str(&format!("      {}\n", phi));
                }
            }

            sb.push_str("    Instructions:\n");
            for instr in &block.instrs {
                sb.push_str(&format!("      {}\n", instr));
            }
        }

        sb
    }
}
