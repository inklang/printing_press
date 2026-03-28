//! IR to bytecode compilation.
//!
//! Converts the lowered IR instructions into packed bytecode words.

use std::collections::HashMap;

use super::chunk::{Chunk, ClassInfo, FunctionDefaults, OpCode};
use super::ir::{IrInstr, IrLabel, MethodInfo};
use super::token::TokenType;
use super::value::Value;
use super::liveness::LivenessAnalyzer;
use super::register_alloc::RegisterAllocator;
use super::spill_insert::SpillInserter;
use super::ssa;

/// Result of compiling a single method.
struct CompiledMethod {
    chunk: Chunk,
    spill_slot_count: usize,
}

/// IR to bytecode compiler.
pub struct IrCompiler;

impl IrCompiler {
    pub fn new() -> Self {
        Self
    }

    /// Compile IR instructions into a bytecode Chunk.
    pub fn compile(&mut self, result: LoweredResult) -> Chunk {
        let mut chunk = Chunk::new();
        chunk.constants.extend(result.constants.clone());

        // First pass: resolve label positions (labels don't emit code)
        let label_offsets = resolve_labels(&result.instrs);

        // Second pass: emit bytecode
        for instr in &result.instrs {
            emit_instr(&mut chunk, instr, &label_offsets);
        }

        chunk
    }

}

impl Default for IrCompiler {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of lowering (from lowerer module).
#[derive(Debug, Clone)]
pub struct LoweredResult {
    pub instrs: Vec<IrInstr>,
    pub constants: Vec<Value>,
    pub arity: usize,
}

/// Resolve label positions in the instruction stream.
fn resolve_labels(instrs: &[IrInstr]) -> HashMap<usize, usize> {
    let mut label_offsets = HashMap::new();
    let mut offset = 0;

    for instr in instrs {
        if let IrInstr::Label { label } = instr {
            label_offsets.insert(label.0, offset);
        } else {
            offset += 1;
        }
    }

    label_offsets
}

/// Emit bytecode for a single instruction.
fn emit_instr(chunk: &mut Chunk, instr: &IrInstr, label_offsets: &HashMap<usize, usize>) {
    match instr {
        IrInstr::LoadImm { dst, index } => {
            chunk.write(OpCode::LoadImm, *dst, 0, 0, *index);
        }
        IrInstr::LoadGlobal { dst, name } => {
            let name_idx = chunk.add_string(name);
            chunk.write(OpCode::LoadGlobal, *dst, 0, 0, name_idx);
        }
        IrInstr::StoreGlobal { name, src } => {
            let name_idx = chunk.add_string(name);
            chunk.write(OpCode::StoreGlobal, 0, *src, 0, name_idx);
        }
        IrInstr::Move { dst, src } => {
            chunk.write(OpCode::Move, *dst, *src, 0, 0);
        }
        IrInstr::BinaryOp { dst, op, src1, src2 } => {
            let opcode = match op {
                TokenType::Plus => OpCode::Add,
                TokenType::Minus => OpCode::Sub,
                TokenType::Star => OpCode::Mul,
                TokenType::Slash => OpCode::Div,
                TokenType::EqEq => OpCode::Eq,
                TokenType::BangEq => OpCode::Neq,
                TokenType::Lt => OpCode::Lt,
                TokenType::Lte => OpCode::Lte,
                TokenType::Gt => OpCode::Gt,
                TokenType::Gte => OpCode::Gte,
                TokenType::Percent => OpCode::Mod,
                TokenType::DotDot => OpCode::Range,
                TokenType::Pow => OpCode::Pow,
                _ => panic!("Unknown binary op: {:?}", op),
            };
            chunk.write(opcode, *dst, *src1, *src2, 0);
        }
        IrInstr::UnaryOp { dst, op, src } => {
            let opcode = match op {
                TokenType::Minus => OpCode::Neg,
                TokenType::Bang | TokenType::KwNot => OpCode::Not,
                _ => panic!("Unknown unary op: {:?}", op),
            };
            chunk.write(opcode, *dst, *src, 0, 0);
        }
        IrInstr::Jump { target } => {
            let offset = label_offsets.get(&target.0)
                .copied()
                .expect("Jump target label not found");
            chunk.write(OpCode::Jump, 0, 0, 0, offset);
        }
        IrInstr::JumpIfFalse { src, target } => {
            let offset = label_offsets.get(&target.0)
                .copied()
                .expect("JumpIfFalse target label not found");
            chunk.write(OpCode::JumpIfFalse, 0, *src, 0, offset);
        }
        IrInstr::Label { .. } => {
            // Skip - resolved in first pass
        }
        IrInstr::Call { dst, func, args } => {
            for &arg in args {
                chunk.write(OpCode::PushArg, 0, arg, 0, 0);
            }
            chunk.write(OpCode::Call, *dst, *func, 0, args.len());
        }
        IrInstr::Return { src } => {
            chunk.write(OpCode::Return, 0, *src, 0, 0);
        }
        IrInstr::Break => {
            chunk.write(OpCode::Break, 0, 0, 0, 0);
        }
        IrInstr::Next => {
            chunk.write(OpCode::Next, 0, 0, 0, 0);
        }
        IrInstr::NewArray { dst, elements } => {
            for &elem in elements {
                chunk.write(OpCode::PushArg, 0, elem, 0, 0);
            }
            chunk.write(OpCode::NewArray, *dst, 0, 0, elements.len());
        }
        IrInstr::GetIndex { dst, obj, index } => {
            chunk.write(OpCode::GetIndex, *dst, *obj, *index, 0);
        }
        IrInstr::SetIndex { obj, index, src } => {
            chunk.write(OpCode::SetIndex, 0, *obj, *index, *src);
        }
        IrInstr::GetField { dst, obj, name } => {
            let name_idx = chunk.add_string(name);
            chunk.write(OpCode::GetField, *dst, *obj, 0, name_idx);
        }
        IrInstr::SetField { obj, name, src } => {
            let name_idx = chunk.add_string(name);
            chunk.write(OpCode::SetField, 0, *obj, *src, name_idx);
        }
        IrInstr::GetUpvalue { dst, upvalue_index } => {
            chunk.write(OpCode::GetUpvalue, *dst, 0, 0, *upvalue_index);
        }
        IrInstr::NewInstance { dst, class_reg, args } => {
            for &arg in args {
                chunk.write(OpCode::PushArg, 0, arg, 0, 0);
            }
            chunk.write(OpCode::NewInstance, *dst, *class_reg, 0, args.len());
        }
        IrInstr::IsType { dst, src, type_name } => {
            let type_idx = chunk.add_string(type_name);
            chunk.write(OpCode::IsType, *dst, *src, 0, type_idx);
        }
        IrInstr::HasCheck { dst, obj, field_name } => {
            let field_idx = chunk.add_string(field_name);
            chunk.write(OpCode::Has, *dst, *obj, 0, field_idx);
        }
        IrInstr::LoadClass { dst, name, super_class, methods } => {
            // Pre-allocate function slots
            let method_names: Vec<String> = methods.keys().cloned().collect();
            let method_start_index = chunk.functions.len();

            for _method_name in &method_names {
                chunk.functions.push(Box::new(Chunk::new()));
            }

            // Compile all methods
            let mut compiled_methods: HashMap<String, CompiledMethod> = HashMap::new();
            for (method_name, method_info) in methods {
                // Note: in Rust we compile sequentially; parallel compilation would require more setup
                let result = compile_method_sequential(method_info);
                compiled_methods.insert(method_name.clone(), result);
            }

            // Replace placeholders with compiled chunks
            let mut method_func_indices: HashMap<String, usize> = HashMap::new();
            for (slot_idx, method_name) in method_names.iter().enumerate() {
                let compiled = compiled_methods.remove(method_name).expect("Method not found");
                let actual_idx = method_start_index + slot_idx;
                chunk.functions[actual_idx] = Box::new(compiled.chunk);
                chunk.functions[actual_idx].spill_slot_count = compiled.spill_slot_count;
                method_func_indices.insert(method_name.clone(), actual_idx);
            }

            // Add class info
            let class_idx = chunk.classes.len();
            chunk.classes.push(ClassInfo {
                name: name.clone(),
                super_class: super_class.clone(),
                methods: method_func_indices,
            });
            chunk.write(OpCode::BuildClass, *dst, 0, 0, class_idx);
        }
        IrInstr::LoadFunc { dst, name: _, arity: _, instrs, constants, default_values, captured_vars, upvalue_regs: _ } => {
            // Compile the function body
            let compiled = compile_function_body(instrs, constants, 0);
            let idx = chunk.functions.len();
            chunk.functions.push(Box::new(compiled.chunk));

            // Store upvalue info
            if !captured_vars.is_empty() {
                chunk.function_upvalues.insert(idx, (captured_vars.len(), vec![]));
            }

            // Compile default value expressions
            let default_chunk_indices: Vec<Option<usize>> = default_values.iter().map(|default_info| {
                if let Some(info) = default_info {
                    let compiled = compile_function_body(&info.instrs, &info.constants, 0);
                    let default_idx = chunk.functions.len();
                    chunk.functions.push(Box::new(compiled.chunk));
                    Some(default_idx)
                } else {
                    None
                }
            }).collect();

            // Ensure functionDefaults has enough entries
            while chunk.function_defaults.len() <= idx {
                chunk.function_defaults.push(FunctionDefaults { default_chunks: vec![] });
            }
            chunk.function_defaults[idx] = FunctionDefaults { default_chunks: default_chunk_indices };

            chunk.write(OpCode::LoadFunc, *dst, 0, 0, idx);
        }
        IrInstr::Spill { slot, src } => {
            chunk.write(OpCode::Spill, 0, *src, 0, *slot);
        }
        IrInstr::Unspill { dst, slot } => {
            chunk.write(OpCode::Unspill, *dst, 0, 0, *slot);
        }
        IrInstr::Throw { src } => {
            chunk.write(OpCode::Throw, 0, *src, 0, 0);
        }
        IrInstr::RegisterEventHandler { .. } => {
            // Registered at runtime, no bytecode
        }
        IrInstr::InvokeEventHandler { .. } => {
            // Invoked at runtime, no bytecode
        }
        IrInstr::AwaitInstr { dst, task } => {
            chunk.write(OpCode::Await, *dst, *task, 0, 0);
        }
        IrInstr::SpawnInstr { dst, func, args, virtual_ } => {
            for &arg in args {
                chunk.write(OpCode::PushArg, 0, arg, 0, 0);
            }
            let opcode = if *virtual_ { OpCode::SpawnVirtual } else { OpCode::Spawn };
            chunk.write(opcode, *dst, *func, 0, args.len());
        }
        IrInstr::AsyncCallInstr { dst, func, args } => {
            for &arg in args {
                chunk.write(OpCode::PushArg, 0, arg, 0, 0);
            }
            chunk.write(OpCode::AsyncCall, *dst, *func, 0, args.len());
        }
        IrInstr::CallHandler { keyword, decl_name, rule_bodies } => {
            use super::chunk::CstNodeEntry;

            // Compile each rule body into chunk.functions and build CST children
            let mut decl_body_entries: Vec<CstNodeEntry> = Vec::new();
            for rule_body in rule_bodies {
                let compiled = compile_function_body(&rule_body.instrs, &rule_body.constants, 0);
                let func_idx = chunk.functions.len();
                chunk.functions.push(Box::new(compiled.chunk));

                // Build children for this RuleMatch entry
                let mut children: Vec<CstNodeEntry> = Vec::new();
                if let Some(ref kw) = rule_body.leading_keyword {
                    children.push(CstNodeEntry::Keyword { value: kw.clone() });
                }
                children.push(CstNodeEntry::FunctionBlock { func_idx });

                decl_body_entries.push(CstNodeEntry::RuleMatch {
                    rule_name: rule_body.rule_name.clone(),
                    children,
                });
            }

            // Add the Declaration node to cst_table and emit CALL_HANDLER
            let cst_idx = chunk.cst_table.len();
            chunk.cst_table.push(CstNodeEntry::Declaration {
                keyword: keyword.clone(),
                name: decl_name.clone(),
                body: decl_body_entries,
            });
            chunk.write(OpCode::CallHandler, 0, 0, 0, cst_idx);
        }
    }
}

/// Compile a method sequentially (used instead of ForkJoinPool in Rust).
fn compile_method_sequential(method_info: &MethodInfo) -> CompiledMethod {
    compile_function_body(&method_info.instrs, &method_info.constants, method_info.arity)
}

/// Compile a function body through the full pipeline: SSA, liveness, register alloc, spill.
fn compile_function_body(instrs: &[IrInstr], constants: &[Value], arity: usize) -> CompiledMethod {
    let ssa_result = ssa::optimized_ssa_round_trip(
        instrs.to_vec(),
        constants.to_vec(),
        arity,
    );

    let ranges = LivenessAnalyzer::new().analyze(&ssa_result.instrs);
    let mut allocator = RegisterAllocator::new();
    let alloc = allocator.allocate(&ranges, arity);
    let resolved = SpillInserter::new().insert(ssa_result.instrs, &alloc, &ranges);

    let lowered_result = LoweredResult {
        instrs: resolved,
        constants: ssa_result.constants,
        arity,
    };

    let chunk = IrCompiler::new().compile(lowered_result);

    CompiledMethod {
        chunk,
        spill_slot_count: alloc.spill_slot_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inklang::token::TokenType;

    #[test]
    fn test_compile_load_imm() {
        let result = LoweredResult {
            instrs: vec![
                IrInstr::LoadImm { dst: 0, index: 0 },
            ],
            constants: vec![Value::Int(42)],
            arity: 0,
        };

        let mut compiler = IrCompiler::new();
        let chunk = compiler.compile(result);

        assert!(!chunk.code.is_empty());
        // First word: opcode=0x00 (LoadImm), dst=0, rest=0
        assert_eq!(chunk.code[0] & 0xFF, 0x00);
    }

    #[test]
    fn test_compile_binary_op() {
        let result = LoweredResult {
            instrs: vec![
                IrInstr::LoadImm { dst: 0, index: 0 },
                IrInstr::LoadImm { dst: 1, index: 1 },
                IrInstr::BinaryOp {
                    dst: 2,
                    op: TokenType::Plus,
                    src1: 0,
                    src2: 1,
                },
            ],
            constants: vec![Value::Int(1), Value::Int(2)],
            arity: 0,
        };

        let mut compiler = IrCompiler::new();
        let chunk = compiler.compile(result);

        assert_eq!(chunk.code.len(), 3);
        // Third instruction: opcode=0x08 (Add), dst=2, src1=0, src2=1
        let word = chunk.code[2];
        assert_eq!(word & 0xFF, 0x08); // Add
        assert_eq!((word >> 8) & 0x0F, 2); // dst
        assert_eq!((word >> 12) & 0x0F, 0); // src1
        assert_eq!((word >> 16) & 0x0F, 1); // src2
    }

    #[test]
    fn test_compile_jump() {
        let result = LoweredResult {
            instrs: vec![
                IrInstr::Jump { target: IrLabel(1) },
                IrInstr::Label { label: IrLabel(1) },
            ],
            constants: vec![],
            arity: 0,
        };

        let mut compiler = IrCompiler::new();
        let chunk = compiler.compile(result);

        // Should have 1 instruction (Jump), Label doesn't emit code
        assert_eq!(chunk.code.len(), 1);
        assert_eq!(chunk.code[0] & 0xFF, 0x14); // Jump
    }

    #[test]
    fn test_chunk_add_string() {
        let mut chunk = Chunk::new();
        let idx1 = chunk.add_string("hello");
        let idx2 = chunk.add_string("world");
        let idx1_again = chunk.add_string("hello");

        assert_eq!(idx1, 0);
        assert_eq!(idx2, 1);
        assert_eq!(idx1_again, 0); // Should return existing index
        assert_eq!(chunk.strings.len(), 2);
    }
}
