//! Debug test to identify hang location

use printing_press::inklang::{lexer, parser::Parser, constant_fold::ConstantFolder, lowerer::AstLowerer, ssa::{self, builder::SsaBuilder}};

#[test]
fn test_stage_1_lexer_simple() {
    let source = "let x = 5";
    let tokens = lexer::tokenize(source);
    println!("Tokens: {:?}", tokens.len());
    assert!(!tokens.is_empty());
}

#[test]
fn test_stage_2_parser_simple() {
    let source = "let x = 5";
    let tokens = lexer::tokenize(source);
    let ast = Parser::new(tokens).parse().expect("Parse error");
    println!("AST stmts: {:?}", ast.len());
    assert!(!ast.is_empty());
}

#[test]
fn test_stage_3_constant_fold_simple() {
    let source = "let x = 5";
    let tokens = lexer::tokenize(source);
    let ast = Parser::new(tokens).parse().expect("Parse error");
    let folded = ConstantFolder::new().fold(&ast);
    println!("Folded stmts: {:?}", folded.len());
    assert!(!folded.is_empty());
}

#[test]
fn test_stage_4_lower_simple() {
    let source = "let x = 5";
    let tokens = lexer::tokenize(source);
    let ast = Parser::new(tokens).parse().expect("Parse error");
    let folded = ConstantFolder::new().fold(&ast);
    let lowered = AstLowerer::new().lower(&folded);
    println!("Lowered instrs: {:?}", lowered.instrs.len());
    assert!(!lowered.instrs.is_empty());
}

#[test]
fn test_stage_5_ssa_build_only() {
    let source = "let x = 5";
    let tokens = lexer::tokenize(source);
    let ast = Parser::new(tokens).parse().expect("Parse error");
    let folded = ConstantFolder::new().fold(&ast);
    let lowered = AstLowerer::new().lower(&folded);
    println!("About to run SSA build on {} instrs", lowered.instrs.len());
    let ssa_func = SsaBuilder::build(lowered.instrs.clone(), lowered.constants.clone(), lowered.arity);
    println!("SSA build done, blocks: {}", ssa_func.blocks.len());
    assert!(!ssa_func.blocks.is_empty());
}

#[test]
fn test_stage_5_ssa_simple() {
    let source = "let x = 5";
    let tokens = lexer::tokenize(source);
    let ast = Parser::new(tokens).parse().expect("Parse error");
    let folded = ConstantFolder::new().fold(&ast);
    let lowered = AstLowerer::new().lower(&folded);
    println!("About to run full SSA round-trip on {} instrs", lowered.instrs.len());
    let ssa_result = ssa::optimized_ssa_round_trip(
        lowered.instrs,
        lowered.constants,
        lowered.arity,
    );
    println!("SSA result: {} instrs", ssa_result.instrs.len());
    assert!(!ssa_result.instrs.is_empty());
}