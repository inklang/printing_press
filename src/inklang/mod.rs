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
pub mod grammar;

pub use serialize::{SerialScript, SerialChunk, SerialValue, SerialConfigField};

use codegen::IrCompiler;
use constant_fold::ConstantFolder;
use grammar::MergedGrammar;
use liveness::LivenessAnalyzer;
use lowerer::AstLowerer;
use parser::Parser;
use register_alloc::RegisterAllocator;
use spill_insert::SpillInserter;
use thiserror::Error;

/// Compile error types.
#[derive(Debug, Error)]
pub enum CompileError {
    #[error("Lexing error: {0}")]
    Lexing(String),
    #[error("Parsing error: {0}")]
    Parsing(String),
    #[error("Compilation error: {0}")]
    Compilation(String),
}

/// Compile Inklang source code to a SerialScript (JSON).
///
/// # Pipeline
/// 1. Tokenize → 2. Parse → 3. Constant Fold → 4. Lower to IR → 5. SSA Round-trip → 6. Register Alloc → 7. Codegen → 8. Serialize
pub fn compile(source: &str, name: &str) -> Result<SerialScript, CompileError> {
    compile_with_grammar(source, name, None)
}

/// Compile Inklang source code with a grammar to a SerialScript (JSON).
///
/// # Pipeline
/// 1. Tokenize → 2. Parse (with grammar) → 3. Constant Fold → 4. Lower to IR → 5. SSA Round-trip → 6. Register Alloc → 7. Codegen → 8. Serialize
pub fn compile_with_grammar(source: &str, name: &str, grammar: Option<&MergedGrammar>) -> Result<SerialScript, CompileError> {
    // 1. Tokenize
    let tokens = lexer::tokenize(source);

    // 2. Parse
    let ast = Parser::new(tokens, grammar)
        .parse()
        .map_err(|e| CompileError::Parsing(e.to_string()))?;

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
    Ok(SerialScript::from_chunk(name, &chunk))
}
