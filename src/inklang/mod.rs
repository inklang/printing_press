pub mod error;
pub mod token;
pub mod lexer;
pub mod ast;
pub mod value;
pub mod parser;
pub mod constant_fold;
pub mod lowerer;
pub mod ir;
pub mod ssa;
pub mod liveness;
pub mod register_alloc;
pub mod spill_insert;
pub mod codegen;
pub mod chunk;
pub mod serialize;

pub use serialize::{SerialScript, SerialChunk, SerialValue, SerialConfigField};

use codegen::IrCompiler;
use constant_fold::ConstantFolder;
use liveness::LivenessAnalyzer;
use lowerer::AstLowerer;
use parser::Parser;
use register_alloc::RegisterAllocator;
use spill_insert::SpillInserter;

/// Compile Inklang source code to a SerialScript (JSON).
///
/// # Pipeline
/// 1. Tokenize → 2. Parse → 3. Constant Fold → 4. Lower to IR → 5. SSA Round-trip → 6. Register Alloc → 7. Codegen → 8. Serialize
pub fn compile(source: &str, name: &str) -> SerialScript {
    // 1. Tokenize
    let tokens = lexer::tokenize(source);

    // 2. Parse
    let ast = Parser::new(tokens).parse().expect("Parse error");

    // 3. Constant fold
    let folded = ConstantFolder::new().fold(&ast);

    // 4. Lower to IR
    let lowered = AstLowerer::new().lower(&folded);

    // 5. SSA round-trip
    let ssa_result = ssa::optimized_ssa_round_trip(
        lowered.instrs,
        lowered.constants,
        lowered.arity,
    );

    // 6. Liveness + register allocation + spill
    let ranges = LivenessAnalyzer::new().analyze(&ssa_result.instrs);
    let mut allocator = RegisterAllocator::new();
    let alloc = allocator.allocate(&ranges, lowered.arity);
    let resolved = SpillInserter::new().insert(ssa_result.instrs, &alloc, &ranges);

    // 7. Codegen
    let codegen_result = codegen::LoweredResult {
        instrs: resolved,
        constants: ssa_result.constants,
        arity: lowered.arity,
    };
    let mut compiler = IrCompiler::new();
    let chunk = compiler.compile(codegen_result);

    // 8. Serialize
    SerialScript::from_chunk(name, &chunk)
}
