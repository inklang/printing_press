//! AST to IR lowering.
//!
//! Transforms the AST into a linear IR with explicit control flow.
//! Allocates virtual registers for temporaries and handles closures
//! by capturing variables from enclosing scopes.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};

use super::ast::{Expr, Param, Stmt};
use super::ir::{DefaultValueInfo, IrInstr, IrLabel, MethodInfo, RuleBodyIr};
use super::token::TokenType;
use super::value::Value;

/// Result of lowering an AST to IR.
#[derive(Debug, Clone)]
pub struct LoweredResult {
    /// Instructions in the lowered function.
    pub instrs: Vec<IrInstr>,
    /// Constants used by the function.
    pub constants: Vec<Value>,
    /// Nested functions defined within this scope.
    pub functions: Vec<LoweredResult>,
    /// Arity of the function (number of parameters).
    pub arity: usize,
}

/// AST lowerer - transforms AST into IR.
#[derive(Debug, Clone)]
pub struct AstLowerer {
    /// Instructions emitted by this lowerer.
    instrs: Vec<IrInstr>,
    /// Constants table.
    constants: Vec<Value>,
    /// Nested functions.
    functions: Vec<LoweredResult>,
    /// Register counter for allocating new registers.
    reg_counter: Cell<usize>,
    /// Label counter for allocating new labels.
    label_counter: Cell<usize>,
    /// Local variables: name -> register index.
    locals: HashMap<String, usize>,
    /// Constants (const declarations).
    const_locals: HashSet<String>,
    /// Current break label (for break statements).
    break_label: Option<IrLabel>,
    /// Current next label (for next statements).
    next_label: Option<IrLabel>,
    /// Lambda counter for generating unique lambda names.
    lambda_counter: Cell<usize>,
    /// Captured variables: var_name -> upvalue_index.
    captured_vars: HashMap<String, usize>,
    /// Snapshot of enclosing scope's locals (for closure capture detection).
    enclosing_locals: HashMap<String, usize>,
    /// Field names for class methods (for resolving bare identifiers to self.field).
    field_names: HashSet<String>,
    /// Names of async functions.
    async_functions: HashSet<String>,
}

impl AstLowerer {
    /// Create a new AST lowerer.
    pub fn new() -> Self {
        Self {
            instrs: Vec::new(),
            constants: Vec::new(),
            functions: Vec::new(),
            reg_counter: Cell::new(0),
            label_counter: Cell::new(0),
            locals: HashMap::new(),
            const_locals: HashSet::new(),
            break_label: None,
            next_label: None,
            lambda_counter: Cell::new(0),
            captured_vars: HashMap::new(),
            enclosing_locals: HashMap::new(),
            field_names: HashSet::new(),
            async_functions: HashSet::new(),
        }
    }

    /// Lower an AST to IR.
    pub fn lower(&mut self, stmts: &[Stmt]) -> LoweredResult {
        for stmt in stmts {
            self.lower_stmt(stmt);
        }
        LoweredResult {
            instrs: self.instrs.clone(),
            constants: self.constants.clone(),
            functions: std::mem::take(&mut self.functions),
            arity: 0, // Top-level script has no parameters
        }
    }

    /// Allocate a fresh virtual register.
    fn fresh_reg(&self) -> usize {
        let reg = self.reg_counter.get();
        self.reg_counter.set(reg + 1);
        reg
    }

    /// Allocate a fresh label.
    fn fresh_label(&self) -> IrLabel {
        let label = IrLabel(self.label_counter.get());
        self.label_counter.set(label.0 + 1);
        label
    }

    /// Add a constant to the constants table and return its index.
    /// Returns the existing index if an equal value is already present.
    fn add_constant(&mut self, value: Value) -> usize {
        if let Some(idx) = self.constants.iter().position(|c| c == &value) {
            return idx;
        }
        self.constants.push(value);
        self.constants.len() - 1
    }

    /// Emit an instruction.
    fn emit(&mut self, instr: IrInstr) {
        self.instrs.push(instr);
    }

    /// Lower a statement.
    fn lower_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let { name, value, .. } => self.lower_var(name, Some(value)),
            Stmt::Const { name, value, .. } => {
                self.lower_var(name, Some(value));
                if let Some(_reg) = self.locals.get(&name.lexeme) {
                    self.const_locals.insert(name.lexeme.clone());
                }
            }
            Stmt::Expr(expr) => {
                let dst = self.fresh_reg();
                self.lower_expr(expr, dst);
            }
            Stmt::Block(stmts) => self.lower_block(stmts),
            Stmt::Fn {
                name,
                params,
                body,
                is_async,
                ..
            } => self.lower_func(name, params, body, *is_async),
            Stmt::If {
                condition,
                then_branch,
                else_branch,
            } => self.lower_if(condition, then_branch, else_branch.as_deref()),
            Stmt::While { condition, body } => self.lower_while(condition, body),
            Stmt::For {
                variable,
                iterable,
                body,
            } => self.lower_for(variable, iterable, body),
            Stmt::Return(value) => self.lower_return(value.as_ref()),
            Stmt::Break => {
                let target = self.break_label.expect("break outside loop");
                self.emit(IrInstr::Jump { target });
            }
            Stmt::Next => {
                let target = self.next_label.expect("next outside loop");
                self.emit(IrInstr::Jump { target });
            }
            Stmt::Class {
                name,
                superclass,
                body,
                ..
            } => self.lower_class(name, superclass.as_ref(), body),
            Stmt::Enum { name, variants } => self.lower_enum(name, variants),
            Stmt::Import(path) => self.lower_import(path),
            Stmt::ImportFrom { path, items } => self.lower_import_from(path, items),
            Stmt::EventDecl { name, params } => self.lower_event_decl(name, params),
            Stmt::On { event, handler } => self.lower_on(event, handler),
            Stmt::Enable(body) => self.lower_block(&[body.as_ref().clone()]),
            Stmt::Disable(body) => self.lower_block(&[body.as_ref().clone()]),
            Stmt::Throw(expr) => {
                let src = self.fresh_reg();
                self.lower_expr(expr, src);
                self.emit(IrInstr::Throw { src });
            }
            Stmt::Try {
                body,
                catch_var,
                catch_body,
                finally_body,
            } => self.lower_try(body, catch_var.as_ref(), catch_body.as_deref(), finally_body.as_deref()),
            Stmt::Config { name, .. } => {
                // Config values are loaded at runtime - no IR emitted
                self.locals.insert(name.lexeme.clone(), self.fresh_reg());
            }
            Stmt::Table { name, fields } => self.lower_table(name, fields),
            Stmt::AnnotationDef { .. } => {
                // Annotation declarations are compile-time only - no IR emitted
            }
            Stmt::GrammarDecl { keyword, name, rules } => {
                // Lower each rule's body independently, then emit CallHandler.
                let mut rule_bodies: Vec<RuleBodyIr> = Vec::new();
                for rule in rules {
                    let mut body_lowerer = AstLowerer::new();
                    let result = body_lowerer.lower(&rule.body);
                    rule_bodies.push(RuleBodyIr {
                        rule_name: rule.rule_name.clone(),
                        leading_keyword: rule.leading_keyword.clone(),
                        instrs: result.instrs,
                        constants: result.constants,
                    });
                }
                self.instrs.push(IrInstr::CallHandler {
                    keyword: keyword.clone(),
                    decl_name: name.clone(),
                    rule_bodies,
                });
            }
        }
    }

    /// Lower a variable declaration.
    fn lower_var(&mut self, name: &super::token::Token, value: Option<&Expr>) {
        let dst = self.fresh_reg();
        self.locals.insert(name.lexeme.clone(), dst);
        if let Some(expr) = value {
            self.lower_expr(expr, dst);
        } else {
            let index = self.add_constant(Value::Null);
            self.emit(IrInstr::LoadImm { dst, index });
        }
    }

    /// Lower a block of statements.
    fn lower_block(&mut self, stmts: &[Stmt]) {
        let before_locals: HashMap<String, usize> = self.locals.iter().map(|(k, v)| (k.clone(), *v)).collect();
        let before_consts = self.const_locals.clone();

        for stmt in stmts {
            self.lower_stmt(stmt);
        }

        // Restore locals to their state before the block
        self.locals.retain(|k, _| before_locals.contains_key(k));
        self.const_locals.retain(|k| before_consts.contains(k));
    }

    /// Lower an if statement.
    fn lower_if(&mut self, condition: &Expr, then_branch: &Stmt, else_branch: Option<&Stmt>) {
        let else_label = self.fresh_label();
        let end_label = self.fresh_label();
        let cond_reg = self.fresh_reg();
        self.lower_expr(condition, cond_reg);
        self.emit(IrInstr::JumpIfFalse {
            src: cond_reg,
            target: else_label,
        });
        self.lower_stmt(then_branch);
        self.emit(IrInstr::Jump { target: end_label });
        self.emit(IrInstr::Label { label: else_label });
        if let Some(eb) = else_branch {
            self.lower_else_branch(eb);
        }
        self.emit(IrInstr::Label { label: end_label });
    }

    /// Lower an else branch.
    fn lower_else_branch(&mut self, branch: &Stmt) {
        match branch {
            Stmt::Block(stmts) => self.lower_block(stmts),
            Stmt::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.lower_if(condition, then_branch, else_branch.as_deref());
            }
            _ => self.lower_stmt(branch),
        }
    }

    /// Lower a while loop.
    fn lower_while(&mut self, condition: &Expr, body: &Stmt) {
        let top_label = self.fresh_label();
        let end_label = self.fresh_label();
        let prev_break = self.break_label.take();
        let prev_next = self.next_label.take();
        self.break_label = Some(end_label);
        self.next_label = Some(top_label);

        self.emit(IrInstr::Label { label: top_label });
        let cond_reg = self.fresh_reg();
        self.lower_expr(condition, cond_reg);
        self.emit(IrInstr::JumpIfFalse {
            src: cond_reg,
            target: end_label,
        });
        self.lower_stmt(body);
        self.emit(IrInstr::Jump { target: top_label });
        self.emit(IrInstr::Label { label: end_label });

        self.break_label = prev_break;
        self.next_label = prev_next;
    }

    /// Lower a for loop.
    fn lower_for(&mut self, variable: &super::token::Token, iterable: &Expr, body: &Stmt) {
        // Desugar: for x in expr { body }
        // Becomes:
        //   let __iter = expr.iter()
        //   while (__iter.hasNext()) {
        //     let x = __iter.next()
        //     body
        //   }

        let top_label = self.fresh_label();
        let end_label = self.fresh_label();
        let prev_break = self.break_label.take();
        let prev_next = self.next_label.take();
        self.break_label = Some(end_label);
        self.next_label = Some(top_label);

        // Evaluate iterable and call .iter()
        let iterable_reg = self.fresh_reg();
        self.lower_expr(iterable, iterable_reg);

        let iter_reg = self.fresh_reg();
        self.emit(IrInstr::GetField {
            dst: iter_reg,
            obj: iterable_reg,
            name: "iter".to_string(),
        });
        self.emit(IrInstr::Call {
            dst: iter_reg,
            func: iter_reg,
            args: vec![],
        });
        self.locals.insert("__iter".to_string(), iter_reg);

        self.emit(IrInstr::Label { label: top_label });
        let cond_reg = self.fresh_reg();
        self.emit(IrInstr::GetField {
            dst: cond_reg,
            obj: iter_reg,
            name: "hasNext".to_string(),
        });
        self.emit(IrInstr::Call {
            dst: cond_reg,
            func: cond_reg,
            args: vec![],
        });
        self.emit(IrInstr::JumpIfFalse {
            src: cond_reg,
            target: end_label,
        });

        let value_reg = self.fresh_reg();
        self.emit(IrInstr::GetField {
            dst: value_reg,
            obj: iter_reg,
            name: "next".to_string(),
        });
        self.emit(IrInstr::Call {
            dst: value_reg,
            func: value_reg,
            args: vec![],
        });
        self.locals.insert(variable.lexeme.clone(), value_reg);

        self.lower_stmt(body);

        self.emit(IrInstr::Jump { target: top_label });
        self.emit(IrInstr::Label { label: end_label });

        self.locals.remove("__iter");
        self.locals.remove(&variable.lexeme);

        self.break_label = prev_break;
        self.next_label = prev_next;
    }

    /// Lower a return statement.
    fn lower_return(&mut self, value: Option<&Expr>) {
        if let Some(expr) = value {
            let dst = self.fresh_reg();
            self.lower_expr(expr, dst);
            self.emit(IrInstr::Return { src: dst });
        } else {
            let dst = self.fresh_reg();
            let index = self.add_constant(Value::Null);
            self.emit(IrInstr::LoadImm { dst, index });
            self.emit(IrInstr::Return { src: dst });
        }
    }

    /// Lower a function declaration.
    fn lower_func(
        &mut self,
        name: &super::token::Token,
        params: &[Param],
        body: &Stmt,
        is_async: bool,
    ) {
        let mut lowerer = AstLowerer::new();
        for (i, param) in params.iter().enumerate() {
            lowerer.locals.insert(param.name.lexeme.clone(), i);
        }
        lowerer.reg_counter = Cell::new(params.len());
        let result = lowerer.lower(&[body.clone()]);

        // Lower default value expressions
        let default_values: Vec<Option<DefaultValueInfo>> = params
            .iter()
            .map(|param| {
                param.default.as_ref().map(|default_value| {
                    let mut default_lowerer = AstLowerer::new();
                    let default_dst = default_lowerer.fresh_reg();
                    default_lowerer.lower_expr(default_value, default_dst);
                    let default_result = default_lowerer.lower(&[]);
                    DefaultValueInfo {
                        instrs: default_result.instrs,
                        constants: default_result.constants,
                    }
                })
            })
            .collect();

        let dst = self.fresh_reg();
        self.locals.insert(name.lexeme.clone(), dst);
        if is_async {
            self.async_functions.insert(name.lexeme.clone());
        }
        self.emit(IrInstr::LoadFunc {
            dst,
            name: name.lexeme.clone(),
            arity: params.len(),
            instrs: result.instrs,
            constants: result.constants,
            default_values,
            captured_vars: vec![],
            upvalue_regs: vec![],
        });
        self.emit(IrInstr::StoreGlobal {
            name: name.lexeme.clone(),
            src: dst,
        });
    }

    /// Lower a class declaration.
    fn lower_class(
        &mut self,
        name: &super::token::Token,
        superclass: Option<&super::token::Token>,
        body: &Stmt,
    ) {
        let mut methods = HashMap::new();
        let mut field_names = HashSet::new();
        let mut field_declarations = vec![];

        // First pass: collect fields and method signatures
        if let Stmt::Block(stmts) = body {
            for member in stmts {
                if let Stmt::Fn { name: method_name, params, body: method_body, .. } = member {
                    // Count params including implicit self
                    let arity = params.len() + 1;
                    field_names.insert(method_name.lexeme.clone());

                    // Create a lowerer for this method
                    let mut method_lowerer = AstLowerer::new();
                    method_lowerer.field_names = field_names.clone();
                    method_lowerer.locals.insert("self".to_string(), 0);
                    for (i, param) in params.iter().enumerate() {
                        method_lowerer.locals.insert(param.name.lexeme.clone(), i + 1);
                    }
                    method_lowerer.reg_counter = Cell::new(params.len() + 1);
                    let result = method_lowerer.lower(&[method_body.as_ref().clone()]);

                    // Lower default values
                    let default_values: Vec<Option<DefaultValueInfo>> = params
                        .iter()
                        .map(|param| {
                            param.default.as_ref().map(|default_value| {
                                let mut default_lowerer = AstLowerer::new();
                                let default_dst = default_lowerer.fresh_reg();
                                default_lowerer.lower_expr(default_value, default_dst);
                                let default_result = default_lowerer.lower(&[]);
                                DefaultValueInfo {
                                    instrs: default_result.instrs,
                                    constants: default_result.constants,
                                }
                            })
                        })
                        .collect();

                    methods.insert(
                        method_name.lexeme.clone(),
                        MethodInfo {
                            arity,
                            instrs: result.instrs,
                            constants: result.constants,
                            default_values,
                        },
                    );
                } else if let Stmt::Let { name: field_name, .. } = member {
                    field_declarations.push(member.clone());
                    field_names.insert(field_name.lexeme.clone());
                }
            }
        }

        // Create implicit init method if there are fields
        if !field_declarations.is_empty() {
            let mut init_lowerer = AstLowerer::new();
            init_lowerer.field_names = field_names.clone();
            init_lowerer.locals.insert("self".to_string(), 0);
            init_lowerer.reg_counter = Cell::new(1);

            for field in &field_declarations {
                if let Stmt::Let { name: field_name, value, .. } = field {
                    let field_name_str = field_name.lexeme.clone();
                    let value_expr = value.clone();
                    let value_dst = init_lowerer.fresh_reg();
                    init_lowerer.lower_expr(&value_expr, value_dst);
                    init_lowerer.emit(IrInstr::SetField {
                        obj: 0,
                        name: field_name_str,
                        src: value_dst,
                    });
                }
            }

            let init_result = init_lowerer.lower(&[]);
            methods.insert(
                "init".to_string(),
                MethodInfo {
                    arity: 1,
                    instrs: init_result.instrs,
                    constants: init_result.constants,
                    default_values: vec![],
                },
            );
        }

        let dst = self.fresh_reg();
        self.locals.insert(name.lexeme.clone(), dst);
        self.emit(IrInstr::LoadClass {
            dst,
            name: name.lexeme.clone(),
            super_class: superclass.map(|t| t.lexeme.clone()),
            methods,
        });
        self.emit(IrInstr::StoreGlobal {
            name: name.lexeme.clone(),
            src: dst,
        });
    }

    /// Lower an enum declaration.
    fn lower_enum(&mut self, name: &super::token::Token, variants: &[super::ast::EnumVariant]) {
        let ns_class_reg = self.fresh_reg();
        self.emit(IrInstr::LoadGlobal {
            dst: ns_class_reg,
            name: "EnumNamespace".to_string(),
        });
        let ns_reg = self.fresh_reg();
        self.emit(IrInstr::NewInstance {
            dst: ns_reg,
            class_reg: ns_class_reg,
            args: vec![],
        });

        let ev_class_reg = self.fresh_reg();
        self.emit(IrInstr::LoadGlobal {
            dst: ev_class_reg,
            name: "EnumValue".to_string(),
        });

        for (ordinal, variant) in variants.iter().enumerate() {
            let val_reg = self.fresh_reg();
            self.emit(IrInstr::NewInstance {
                dst: val_reg,
                class_reg: ev_class_reg,
                args: vec![],
            });
            let name_reg = self.fresh_reg();
            let name_idx = self.add_constant(Value::String(variant.name.lexeme.clone()));
            self.emit(IrInstr::LoadImm { dst: name_reg, index: name_idx });
            self.emit(IrInstr::SetField {
                obj: val_reg,
                name: "name".to_string(),
                src: name_reg,
            });
            let ord_reg = self.fresh_reg();
            let ord_idx = self.add_constant(Value::Int(ordinal as i64));
            self.emit(IrInstr::LoadImm { dst: ord_reg, index: ord_idx });
            self.emit(IrInstr::SetField {
                obj: val_reg,
                name: "ordinal".to_string(),
                src: ord_reg,
            });
            self.emit(IrInstr::SetField {
                obj: ns_reg,
                name: variant.name.lexeme.clone(),
                src: val_reg,
            });
        }

        self.locals.insert(name.lexeme.clone(), ns_reg);
        self.emit(IrInstr::StoreGlobal {
            name: name.lexeme.clone(),
            src: ns_reg,
        });
    }

    /// Lower an import statement.
    fn lower_import(&mut self, path: &[String]) {
        let name = path.join(".");
        let dst = self.fresh_reg();
        self.locals.insert(name.clone(), dst);

        // For stdlib imports, load the actual instance from globals
        if name == "math" || name == "random" || name == "io" || name == "json" {
            self.emit(IrInstr::LoadGlobal { dst, name: name.clone() });
        } else {
            let marker_idx = self.add_constant(Value::String(format!("__import__{}", name)));
            self.emit(IrInstr::LoadImm { dst, index: marker_idx });
        }
        self.emit(IrInstr::StoreGlobal { name, src: dst });
    }

    /// Lower an import from statement.
    fn lower_import_from(&mut self, path: &[String], items: &[String]) {
        let namespace = path.join(".");
        for tok in items {
            let dst = self.fresh_reg();
            self.locals.insert(tok.clone(), dst);
            let marker_idx = self.add_constant(Value::String(format!("__import_from__{}__{}", namespace, tok)));
            self.emit(IrInstr::LoadImm { dst, index: marker_idx });
            self.emit(IrInstr::StoreGlobal { name: tok.clone(), src: dst });
        }
    }

    /// Lower an event declaration.
    fn lower_event_decl(&mut self, name: &super::token::Token, params: &[super::ast::EventParam]) {
        let event_info = Value::EventInfo {
            name: name.lexeme.clone(),
            params: params.iter().map(|p| (p.name.lexeme.clone(), p.type_.lexeme.clone())).collect(),
        };
        self.add_constant(event_info);
    }

    /// Lower an on statement (event handler).
    fn lower_on(&mut self, event: &super::token::Token, handler: &Stmt) {
        let event_name = &event.lexeme;

        // Extract handler parameters and body
        let (params, body) = if let Stmt::Fn { params, body, .. } = handler {
            (params.clone(), body.as_ref().clone())
        } else if let Stmt::Block(stmts) = handler {
            // Anonymous handler - extract first function if present
            for stmt in stmts {
                if let Stmt::Fn { params, body, .. } = stmt {
                    let (event_param, data_params) = if params.len() >= 1 {
                        (params[0].clone(), params[1..].to_vec())
                    } else {
                        (
                            Param {
                                annotations: vec![],
                                name: super::token::Token {
                                    typ: super::token::TokenType::Identifier,
                                    lexeme: "event".to_string(),
                                    line: 0,
                                    column: 0,
                                },
                                type_annot: None,
                                default: None,
                            },
                            vec![],
                        )
                    };

                    let mut handler_lowerer = AstLowerer::new();
                    handler_lowerer.locals.insert(event_param.name.lexeme.clone(), 0);
                    for (i, param) in data_params.iter().enumerate() {
                        handler_lowerer.locals.insert(param.name.lexeme.clone(), i + 1);
                    }
                    handler_lowerer.reg_counter = Cell::new(data_params.len() + 1);
                    let result = handler_lowerer.lower(&[body.as_ref().clone()]);

                    let handler_func_index = self.functions.len();
                    self.functions.push(LoweredResult {
                        instrs: result.instrs,
                        constants: result.constants,
                        functions: vec![],
                        arity: data_params.len() + 1, // +1 for implicit event param
                    });

                    self.emit(IrInstr::RegisterEventHandler {
                        event_name: event_name.clone(),
                        handler_func_index,
                        event_param_name: event_param.name.lexeme,
                        data_param_names: data_params.iter().map(|p| p.name.lexeme.clone()).collect(),
                    });
                    return;
                }
            }
            return; // No function found in block
        } else {
            return;
        };

        // Split params into event param and data params
        let (event_param, data_params) = if params.len() >= 1 {
            (params[0].clone(), params[1..].to_vec())
        } else {
            (
                Param {
                    annotations: vec![],
                    name: super::token::Token {
                        typ: super::token::TokenType::Identifier,
                        lexeme: "event".to_string(),
                        line: 0,
                        column: 0,
                    },
                    type_annot: None,
                    default: None,
                },
                vec![],
            )
        };

        let mut handler_lowerer = AstLowerer::new();
        handler_lowerer.locals.insert(event_param.name.lexeme.clone(), 0);
        for (i, param) in data_params.iter().enumerate() {
            handler_lowerer.locals.insert(param.name.lexeme.clone(), i + 1);
        }
        handler_lowerer.reg_counter = Cell::new(data_params.len() + 1);
        let result = handler_lowerer.lower(&[body.clone()]);

        let handler_func_index = self.functions.len();
        self.functions.push(LoweredResult {
            instrs: result.instrs,
            constants: result.constants,
            functions: vec![],
            arity: data_params.len() + 1, // +1 for implicit event param
        });

        self.emit(IrInstr::RegisterEventHandler {
            event_name: event_name.clone(),
            handler_func_index,
            event_param_name: event_param.name.lexeme,
            data_param_names: data_params.iter().map(|p| p.name.lexeme.clone()).collect(),
        });
    }

    /// Lower a table declaration.
    fn lower_table(&mut self, name: &super::token::Token, fields: &[super::ast::TableField]) {
        let table_name = name.lexeme.clone();
        let field_names_list: Vec<String> = fields.iter().map(|f| f.name.lexeme.clone()).collect();
        let key_field_idx = fields.iter().position(|f| f.is_key).unwrap_or(0);

        // Step 1: db.registerTable("TableName", ["field1", "field2"], keyIndex)
        let db_reg = self.fresh_reg();
        self.emit(IrInstr::LoadGlobal {
            dst: db_reg,
            name: "db".to_string(),
        });

        let table_name_idx = self.add_constant(Value::String(table_name.clone()));
        let table_name_reg = self.fresh_reg();
        self.emit(IrInstr::LoadImm { dst: table_name_reg, index: table_name_idx });

        // Build array of field name strings
        let field_regs: Vec<usize> = field_names_list
            .iter()
            .map(|fname| {
                let r = self.fresh_reg();
                let idx = self.add_constant(Value::String(fname.clone()));
                self.emit(IrInstr::LoadImm { dst: r, index: idx });
                r
            })
            .collect();
        let fields_reg = self.fresh_reg();
        self.emit(IrInstr::NewArray {
            dst: fields_reg,
            elements: field_regs,
        });

        let key_idx_idx = self.add_constant(Value::Int(key_field_idx as i64));
        let key_idx_reg = self.fresh_reg();
        self.emit(IrInstr::LoadImm { dst: key_idx_reg, index: key_idx_idx });

        let register_table_method_reg = self.fresh_reg();
        self.emit(IrInstr::GetField {
            dst: register_table_method_reg,
            obj: db_reg,
            name: "registerTable".to_string(),
        });

        let call_dst = self.fresh_reg();
        self.emit(IrInstr::Call {
            dst: call_dst,
            func: register_table_method_reg,
            args: vec![table_name_reg, fields_reg, key_idx_reg],
        });

        // Step 2: TableName = db.from("TableName")
        let db_reg2 = self.fresh_reg();
        self.emit(IrInstr::LoadGlobal {
            dst: db_reg2,
            name: "db".to_string(),
        });
        let table_name_reg2 = self.fresh_reg();
        self.emit(IrInstr::LoadImm {
            dst: table_name_reg2,
            index: table_name_idx,
        });

        let from_method_reg = self.fresh_reg();
        self.emit(IrInstr::GetField {
            dst: from_method_reg,
            obj: db_reg2,
            name: "from".to_string(),
        });

        let table_reg = self.fresh_reg();
        self.emit(IrInstr::Call {
            dst: table_reg,
            func: from_method_reg,
            args: vec![table_name_reg2],
        });

        self.locals.insert(table_name.clone(), table_reg);
        self.emit(IrInstr::StoreGlobal {
            name: table_name,
            src: table_reg,
        });
    }

    /// Lower a try-catch-finally statement.
    fn lower_try(
        &mut self,
        body: &Stmt,
        _catch_var: Option<&super::token::Token>,
        catch_body: Option<&Stmt>,
        finally_body: Option<&Stmt>,
    ) {
        // Simple lowering: just execute body, catch, then finally
        // Full implementation would require exception handling infrastructure
        self.lower_stmt(body);

        if let Some(cb) = catch_body {
            self.lower_stmt(cb);
        }
        if let Some(fb) = finally_body {
            self.lower_stmt(fb);
        }
    }

    /// Lower an expression to a destination register.
    /// Returns the register containing the result.
    fn lower_expr(&mut self, expr: &Expr, dst: usize) -> usize {
        match expr {
            Expr::Literal(value) => {
                let index = self.add_constant(value.clone());
                self.emit(IrInstr::LoadImm { dst, index });
                dst
            }
            Expr::Variable(name) => {
                let name_str = &name.lexeme;
                if let Some(reg) = self.locals.get(name_str) {
                    if *reg != dst {
                        self.emit(IrInstr::Move { dst, src: *reg });
                    }
                    dst
                } else if self.field_names.contains(name_str) {
                    self.emit(IrInstr::GetField {
                        dst,
                        obj: 0,
                        name: name_str.clone(),
                    });
                    dst
                } else if self.enclosing_locals.contains_key(name_str) {
                    let upvalue_index = self.captured_vars.get(name_str).copied().unwrap_or_else(|| {
                        let idx = self.captured_vars.len();
                        self.captured_vars.insert(name_str.to_string(), idx);
                        idx
                    });
                    self.emit(IrInstr::GetUpvalue {
                        dst,
                        upvalue_index,
                    });
                    dst
                } else {
                    self.emit(IrInstr::LoadGlobal {
                        dst,
                        name: name_str.clone(),
                    });
                    dst
                }
            }
            Expr::Binary { left, op, right } => {
                match op.typ {
                    TokenType::KwAnd => {
                        // Short-circuit AND
                        let short_circuit = self.fresh_label();
                        let end = self.fresh_label();
                        let a_reg = self.fresh_reg();
                        self.lower_expr(left, a_reg);
                        self.emit(IrInstr::JumpIfFalse {
                            src: a_reg,
                            target: short_circuit,
                        });
                        self.lower_expr(right, dst);
                        self.emit(IrInstr::Jump { target: end });
                        self.emit(IrInstr::Label { label: short_circuit });
                        self.emit(IrInstr::Move { dst, src: a_reg });
                        self.emit(IrInstr::Label { label: end });
                        dst
                    }
                    TokenType::KwOr => {
                        // Short-circuit OR
                        let or_false = self.fresh_label();
                        let end = self.fresh_label();
                        let a_reg = self.fresh_reg();
                        self.lower_expr(left, a_reg);
                        self.emit(IrInstr::JumpIfFalse {
                            src: a_reg,
                            target: or_false,
                        });
                        self.emit(IrInstr::Move { dst, src: a_reg });
                        self.emit(IrInstr::Jump { target: end });
                        self.emit(IrInstr::Label { label: or_false });
                        self.lower_expr(right, dst);
                        self.emit(IrInstr::Label { label: end });
                        dst
                    }
                    _ => {
                        let src1 = self.fresh_reg();
                        let src2 = self.fresh_reg();
                        self.lower_expr(left, src1);
                        self.lower_expr(right, src2);
                        self.emit(IrInstr::BinaryOp {
                            dst,
                            op: op.typ,
                            src1,
                            src2,
                        });
                        dst
                    }
                }
            }
            Expr::Unary { op, right } => {
                if op.typ == TokenType::Increment || op.typ == TokenType::Decrement {
                    // Prefix ++/--: mutate the variable and return the new value
                    let target = if let Expr::Variable(var) = right.as_ref() {
                        var
                    } else {
                        panic!("++/-- can only be applied to simple variables")
                    };
                    let delta = if op.typ == TokenType::Increment {
                        TokenType::Plus
                    } else {
                        TokenType::Minus
                    };
                    let one_idx = self.add_constant(Value::Int(1));
                    let one_reg = self.fresh_reg();
                    self.emit(IrInstr::LoadImm { dst: one_reg, index: one_idx });
                    let src_reg = self.fresh_reg();
                    self.lower_expr(right, src_reg);
                    self.emit(IrInstr::BinaryOp {
                        dst,
                        op: delta,
                        src1: src_reg,
                        src2: one_reg,
                    });
                    // Write back
                    let reg = self.locals.get(&target.lexeme);
                    if let Some(r) = reg {
                        self.emit(IrInstr::Move { dst: *r, src: dst });
                    } else {
                        self.emit(IrInstr::StoreGlobal {
                            name: target.lexeme.clone(),
                            src: dst,
                        });
                    }
                } else {
                    let src = self.fresh_reg();
                    self.lower_expr(right, src);
                    self.emit(IrInstr::UnaryOp { dst, op: op.typ, src });
                }
                dst
            }
            Expr::Assign { target, op, value } => {
                if op.typ != TokenType::Assign {
                    // Compound assignment: desugar target op= value to target = target op value
                    let binary_op = match op.typ {
                        TokenType::AddEquals => TokenType::Plus,
                        TokenType::SubEquals => TokenType::Minus,
                        TokenType::MulEquals => TokenType::Star,
                        TokenType::DivEquals => TokenType::Slash,
                        TokenType::ModEquals => TokenType::Percent,
                        _ => panic!("Unknown compound operator: {:?}", op.typ),
                    };

                    match target.as_ref() {
                        Expr::Variable(var) => {
                            if self.const_locals.contains(&var.lexeme) {
                                panic!("Cannot reassign const '{}'", var.lexeme);
                            }
                            let reg = self.locals.get(&var.lexeme).copied();
                            if let Some(r) = reg {
                                let value_reg = self.fresh_reg();
                                self.lower_expr(value, value_reg);
                                self.emit(IrInstr::BinaryOp {
                                    dst: r,
                                    op: binary_op,
                                    src1: r,
                                    src2: value_reg,
                                });
                                r
                            } else if self.field_names.contains(&var.lexeme) {
                                let current_reg = self.fresh_reg();
                                self.emit(IrInstr::GetField {
                                    dst: current_reg,
                                    obj: 0,
                                    name: var.lexeme.clone(),
                                });
                                let value_reg = self.fresh_reg();
                                self.lower_expr(value, value_reg);
                                self.emit(IrInstr::BinaryOp {
                                    dst: current_reg,
                                    op: binary_op,
                                    src1: current_reg,
                                    src2: value_reg,
                                });
                                self.emit(IrInstr::SetField {
                                    obj: 0,
                                    name: var.lexeme.clone(),
                                    src: current_reg,
                                });
                                current_reg
                            } else {
                                let tmp_reg = self.fresh_reg();
                                self.emit(IrInstr::LoadGlobal {
                                    dst: tmp_reg,
                                    name: var.lexeme.clone(),
                                });
                                let value_reg = self.fresh_reg();
                                self.lower_expr(value, value_reg);
                                self.emit(IrInstr::BinaryOp {
                                    dst: tmp_reg,
                                    op: binary_op,
                                    src1: tmp_reg,
                                    src2: value_reg,
                                });
                                self.emit(IrInstr::StoreGlobal {
                                    name: var.lexeme.clone(),
                                    src: tmp_reg,
                                });
                                tmp_reg
                            }
                        }
                        Expr::Index { obj, index } => {
                            let obj_reg = self.fresh_reg();
                            let index_reg = self.fresh_reg();
                            self.lower_expr(obj, obj_reg);
                            self.lower_expr(index, index_reg);
                            let current_reg = self.fresh_reg();
                            self.emit(IrInstr::GetIndex {
                                dst: current_reg,
                                obj: obj_reg,
                                index: index_reg,
                            });
                            let value_reg = self.fresh_reg();
                            self.lower_expr(value, value_reg);
                            self.emit(IrInstr::BinaryOp {
                                dst: current_reg,
                                op: binary_op,
                                src1: current_reg,
                                src2: value_reg,
                            });
                            self.emit(IrInstr::SetIndex {
                                obj: obj_reg,
                                index: index_reg,
                                src: current_reg,
                            });
                            current_reg
                        }
                        _ => panic!("Invalid compound assignment target"),
                    }
                } else {
                    // Simple assignment
                    match target.as_ref() {
                        Expr::Get { obj, name } => {
                            let obj_reg = self.fresh_reg();
                            let src_reg = self.fresh_reg();
                            self.lower_expr(obj, obj_reg);
                            self.lower_expr(value, src_reg);
                            self.emit(IrInstr::SetField {
                                obj: obj_reg,
                                name: name.lexeme.clone(),
                                src: src_reg,
                            });
                            src_reg
                        }
                        Expr::Index { obj, index } => {
                            let obj_reg = self.fresh_reg();
                            let index_reg = self.fresh_reg();
                            let src_reg = self.fresh_reg();
                            self.lower_expr(obj, obj_reg);
                            self.lower_expr(index, index_reg);
                            self.lower_expr(value, src_reg);
                            self.emit(IrInstr::SetIndex {
                                obj: obj_reg,
                                index: index_reg,
                                src: src_reg,
                            });
                            src_reg
                        }
                        Expr::Variable(var) => {
                            if self.const_locals.contains(&var.lexeme) {
                                panic!("Cannot reassign const '{}'", var.lexeme);
                            }
                            let reg = self.locals.get(&var.lexeme).copied();
                            if let Some(r) = reg {
                                self.lower_expr(value, r);
                                r
                            } else if self.field_names.contains(&var.lexeme) {
                                let src = self.fresh_reg();
                                self.lower_expr(value, src);
                                self.emit(IrInstr::SetField {
                                    obj: 0,
                                    name: var.lexeme.clone(),
                                    src,
                                });
                                src
                            } else {
                                let src = self.fresh_reg();
                                self.lower_expr(value, src);
                                self.emit(IrInstr::StoreGlobal {
                                    name: var.lexeme.clone(),
                                    src,
                                });
                                src
                            }
                        }
                        _ => panic!("Invalid assignment target"),
                    }
                }
            }
            Expr::Call { callee, arguments, .. } => {
                let is_async_call = if let Expr::Variable(var) = callee.as_ref() {
                    self.async_functions.contains(&var.lexeme)
                } else {
                    false
                };
                let func_reg = self.fresh_reg();
                self.lower_expr(callee, func_reg);
                let arg_regs: Vec<usize> = arguments
                    .iter()
                    .map(|arg| {
                        let r = self.fresh_reg();
                        self.lower_expr(arg, r);
                        r
                    })
                    .collect();
                if is_async_call {
                    self.emit(IrInstr::AsyncCallInstr {
                        dst,
                        func: func_reg,
                        args: arg_regs,
                    });
                } else {
                    self.emit(IrInstr::Call {
                        dst,
                        func: func_reg,
                        args: arg_regs,
                    });
                }
                dst
            }
            Expr::Group(inner) => {
                self.lower_expr(inner, dst)
            }
            Expr::Get { obj, name } => {
                let obj_reg = self.fresh_reg();
                self.lower_expr(obj, obj_reg);
                self.emit(IrInstr::GetField {
                    dst,
                    obj: obj_reg,
                    name: name.lexeme.clone(),
                });
                dst
            }
            Expr::Index { obj, index } => {
                let obj_reg = self.fresh_reg();
                let index_reg = self.fresh_reg();
                self.lower_expr(obj, obj_reg);
                self.lower_expr(index, index_reg);
                self.emit(IrInstr::GetIndex {
                    dst,
                    obj: obj_reg,
                    index: index_reg,
                });
                dst
            }
            Expr::Is { expr, type_ } => {
                let src_reg = self.fresh_reg();
                self.lower_expr(expr, src_reg);
                self.emit(IrInstr::IsType {
                    dst,
                    src: src_reg,
                    type_name: type_.lexeme.clone(),
                });
                dst
            }
            Expr::Has { target, field } => {
                let obj_reg = self.fresh_reg();
                self.lower_expr(target, obj_reg);
                let field_name = if let Expr::Literal(Value::String(s)) = field.as_ref() {
                    s.clone()
                } else {
                    panic!("has: field name must be a string literal");
                };
                self.emit(IrInstr::HasCheck {
                    dst,
                    obj: obj_reg,
                    field_name,
                });
                dst
            }
            Expr::Elvis { left, right } => {
                // left ?? right desugars to:
                // temp = left
                // if temp != null goto use_left
                // temp = right
                // use_left:
                // result = temp
                let use_left_label = self.fresh_label();
                let end_label = self.fresh_label();
                let temp_reg = self.fresh_reg();
                self.lower_expr(left, temp_reg);
                let null_const_idx = self.add_constant(Value::Null);
                let null_temp_reg = self.fresh_reg();
                self.emit(IrInstr::LoadImm {
                    dst: null_temp_reg,
                    index: null_const_idx,
                });
                let cmp_reg = self.fresh_reg();
                self.emit(IrInstr::BinaryOp {
                    dst: cmp_reg,
                    op: TokenType::EqEq,
                    src1: temp_reg,
                    src2: null_temp_reg,
                });
                self.emit(IrInstr::JumpIfFalse {
                    src: cmp_reg,
                    target: use_left_label,
                });
                self.lower_expr(right, temp_reg);
                self.emit(IrInstr::Jump { target: end_label });
                self.emit(IrInstr::Label { label: use_left_label });
                self.emit(IrInstr::Label { label: end_label });
                self.emit(IrInstr::Move { dst, src: temp_reg });
                dst
            }
            Expr::SafeCall { obj, name } => {
                // obj?.name desugars to:
                // temp = obj
                // if (temp == null) goto null_label
                // result = temp.name
                // goto end_label
                // null_label:
                // result = null
                // end_label:
                let null_label = self.fresh_label();
                let end_label = self.fresh_label();
                let obj_reg = self.fresh_reg();
                self.lower_expr(obj, obj_reg);
                let null_const_idx = self.add_constant(Value::Null);
                let null_temp_reg = self.fresh_reg();
                self.emit(IrInstr::LoadImm {
                    dst: null_temp_reg,
                    index: null_const_idx,
                });
                let cmp_reg = self.fresh_reg();
                self.emit(IrInstr::BinaryOp {
                    dst: cmp_reg,
                    op: TokenType::BangEq,
                    src1: obj_reg,
                    src2: null_temp_reg,
                });
                self.emit(IrInstr::JumpIfFalse {
                    src: cmp_reg,
                    target: null_label,
                });
                self.emit(IrInstr::GetField {
                    dst,
                    obj: obj_reg,
                    name: name.lexeme.clone(),
                });
                self.emit(IrInstr::Jump { target: end_label });
                self.emit(IrInstr::Label { label: null_label });
                self.emit(IrInstr::LoadImm { dst, index: null_const_idx });
                self.emit(IrInstr::Label { label: end_label });
                dst
            }
            Expr::Lambda { params, body, .. } => {
                let lambda_name = format!("__lambda_{}", self.lambda_counter.get());
                self.lambda_counter.set(self.lambda_counter.get() + 1);

                let enclosing = self.locals.clone();
                let mut captured = HashMap::new();

                let mut lowerer = AstLowerer::new();
                lowerer.enclosing_locals = enclosing.clone();
                lowerer.captured_vars = captured.clone();
                lowerer.field_names = self.field_names.clone();

                for (i, param) in params.iter().enumerate() {
                    lowerer.locals.insert(param.name.lexeme.clone(), i);
                }
                lowerer.reg_counter = Cell::new(params.len());

                let result = lowerer.lower(&[body.as_ref().clone()]);

                let captured_names: Vec<String> = captured.keys().cloned().collect();
                let upvalue_src_regs: Vec<usize> = captured.values().cloned().collect();

                let default_values: Vec<Option<DefaultValueInfo>> = params
                    .iter()
                    .map(|param| {
                        param.default.as_ref().map(|default_value| {
                            let mut default_lowerer = AstLowerer::new();
                            let default_dst = default_lowerer.fresh_reg();
                            default_lowerer.lower_expr(default_value, default_dst);
                            let default_result = default_lowerer.lower(&[]);
                            DefaultValueInfo {
                                instrs: default_result.instrs,
                                constants: default_result.constants,
                            }
                        })
                    })
                    .collect();

                self.emit(IrInstr::LoadFunc {
                    dst,
                    name: lambda_name,
                    arity: params.len(),
                    instrs: result.instrs,
                    constants: result.constants,
                    default_values,
                    captured_vars: captured_names,
                    upvalue_regs: upvalue_src_regs,
                });
                dst
            }
            Expr::List(elements) => {
                let element_regs: Vec<usize> = elements
                    .iter()
                    .map(|elem| {
                        let r = self.fresh_reg();
                        self.lower_expr(elem, r);
                        r
                    })
                    .collect();
                self.emit(IrInstr::NewArray {
                    dst,
                    elements: element_regs,
                });
                dst
            }
            Expr::Set(elements) => {
                // Desugar to Set(element1, element2, ...)
                let element_regs: Vec<usize> = elements
                    .iter()
                    .map(|elem| {
                        let r = self.fresh_reg();
                        self.lower_expr(elem, r);
                        r
                    })
                    .collect();
                let set_class_reg = self.fresh_reg();
                self.emit(IrInstr::LoadGlobal {
                    dst: set_class_reg,
                    name: "Set".to_string(),
                });
                self.emit(IrInstr::NewInstance {
                    dst,
                    class_reg: set_class_reg,
                    args: element_regs,
                });
                dst
            }
            Expr::Tuple(elements) => {
                // Desugar to Tuple(element1, element2, ...)
                let element_regs: Vec<usize> = elements
                    .iter()
                    .map(|elem| {
                        let r = self.fresh_reg();
                        self.lower_expr(elem, r);
                        r
                    })
                    .collect();
                let tuple_class_reg = self.fresh_reg();
                self.emit(IrInstr::LoadGlobal {
                    dst: tuple_class_reg,
                    name: "Tuple".to_string(),
                });
                self.emit(IrInstr::NewInstance {
                    dst,
                    class_reg: tuple_class_reg,
                    args: element_regs,
                });
                dst
            }
            Expr::Map(entries) => {
                let map_class_reg = self.fresh_reg();
                self.emit(IrInstr::LoadGlobal {
                    dst: map_class_reg,
                    name: "Map".to_string(),
                });
                self.emit(IrInstr::NewInstance {
                    dst,
                    class_reg: map_class_reg,
                    args: vec![],
                });
                for (key, value) in entries {
                    let key_reg = self.fresh_reg();
                    let value_reg = self.fresh_reg();
                    self.lower_expr(key, key_reg);
                    self.lower_expr(value, value_reg);
                    let set_method_reg = self.fresh_reg();
                    self.emit(IrInstr::GetField {
                        dst: set_method_reg,
                        obj: dst,
                        name: "set".to_string(),
                    });
                    let call_dst = self.fresh_reg();
                    self.emit(IrInstr::Call {
                        dst: call_dst,
                        func: set_method_reg,
                        args: vec![key_reg, value_reg],
                    });
                }
                dst
            }
            Expr::Ternary {
                condition,
                then_branch,
                else_branch,
            } => {
                let else_label = self.fresh_label();
                let end_label = self.fresh_label();
                let cond_reg = self.fresh_reg();
                self.lower_expr(condition, cond_reg);
                self.emit(IrInstr::JumpIfFalse {
                    src: cond_reg,
                    target: else_label,
                });
                self.lower_expr(then_branch, dst);
                self.emit(IrInstr::Jump { target: end_label });
                self.emit(IrInstr::Label { label: else_label });
                self.lower_expr(else_branch, dst);
                self.emit(IrInstr::Label { label: end_label });
                dst
            }
            Expr::Await(inner) => {
                let task_reg = self.fresh_reg();
                self.lower_expr(inner, task_reg);
                self.emit(IrInstr::AwaitInstr { dst, task: task_reg });
                dst
            }
            Expr::Spawn { expr, virtual_ } => {
                let call_expr = if let Expr::Call { .. } = expr.as_ref() {
                    expr.as_ref()
                } else {
                    panic!("spawn requires a call expression");
                };
                if let Expr::Call { callee, arguments, .. } = call_expr {
                    let func_reg = self.fresh_reg();
                    self.lower_expr(callee, func_reg);
                    let arg_regs: Vec<usize> = arguments
                        .iter()
                        .map(|arg| {
                            let r = self.fresh_reg();
                            self.lower_expr(arg, r);
                            r
                        })
                        .collect();
                    self.emit(IrInstr::SpawnInstr {
                        dst,
                        func: func_reg,
                        args: arg_regs,
                        virtual_: *virtual_,
                    });
                }
                dst
            }
            Expr::Throw(inner) => {
                let value_reg = self.fresh_reg();
                self.lower_expr(inner, value_reg);
                self.emit(IrInstr::Throw { src: value_reg });
                dst
            }
            Expr::Annotation { .. } => {
                panic!("AnnotationExpr should not appear in lowered IR");
            }
            Expr::NamedArg { .. } => {
                panic!("NamedArgExpr should not appear in lowered IR");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inklang::token::TokenType;

    fn make_token(typ: TokenType, lexeme: &str) -> super::super::token::Token {
        super::super::token::Token {
            typ,
            lexeme: lexeme.to_string(),
            line: 1,
            column: 0,
        }
    }

    #[test]
    fn test_lower_literal() {
        let mut lowerer = AstLowerer::new();
        let result = lowerer.lower(&[Stmt::Return(Some(Expr::Literal(Value::Int(42))))]);
        assert!(!result.instrs.is_empty());
        // Should produce LoadImm instruction
        let has_load_imm = result.instrs.iter().any(|i| matches!(i, IrInstr::LoadImm { .. }));
        assert!(has_load_imm);
    }

    #[test]
    fn test_lower_variable() {
        let mut lowerer = AstLowerer::new();
        lowerer.locals.insert("x".to_string(), 1); // x is in register 1
        let result = lowerer.lower(&[Stmt::Return(Some(Expr::Variable(make_token(TokenType::Identifier, "x"))))]);
        assert!(!result.instrs.is_empty());
        // Should produce Move instruction (x is in reg 1, return uses reg 0)
        let has_move = result.instrs.iter().any(|i| matches!(i, IrInstr::Move { .. }));
        assert!(has_move);
    }

    #[test]
    fn test_lower_binary_op() {
        let mut lowerer = AstLowerer::new();
        let expr = Expr::Binary {
            left: Box::new(Expr::Literal(Value::Int(1))),
            op: make_token(TokenType::Plus, "+"),
            right: Box::new(Expr::Literal(Value::Int(2))),
        };
        let _dst = lowerer.lower_expr(&expr, 0);
        let has_binary_op = lowerer.instrs.iter().any(|i| matches!(i, IrInstr::BinaryOp { .. }));
        assert!(has_binary_op);
    }

    #[test]
    fn test_lower_let() {
        let mut lowerer = AstLowerer::new();
        let stmt = Stmt::Let {
            annotations: vec![],
            name: make_token(TokenType::Identifier, "x"),
            type_annot: None,
            value: Expr::Literal(Value::Int(5)),
        };
        let result = lowerer.lower(&[stmt]);
        assert!(!result.instrs.is_empty());
    }

    #[test]
    fn test_lower_return() {
        let mut lowerer = AstLowerer::new();
        let result = lowerer.lower(&[Stmt::Return(Some(Expr::Literal(Value::Int(42))))]);
        assert!(!result.instrs.is_empty());
        let has_return = result.instrs.iter().any(|i| matches!(i, IrInstr::Return { .. }));
        assert!(has_return);
    }

    #[test]
    fn test_fresh_labels_unique() {
        let lowerer = AstLowerer::new();
        let label1 = lowerer.fresh_label();
        let label2 = lowerer.fresh_label();
        assert_ne!(label1, label2);
    }

    #[test]
    fn test_fresh_regs_unique() {
        let lowerer = AstLowerer::new();
        let reg1 = lowerer.fresh_reg();
        let reg2 = lowerer.fresh_reg();
        assert_ne!(reg1, reg2);
    }

    #[test]
    fn test_lower_block_scope() {
        let mut lowerer = AstLowerer::new();
        // Inner block with local x
        let block = Stmt::Block(vec![
            Stmt::Let {
                annotations: vec![],
                name: make_token(TokenType::Identifier, "x"),
                type_annot: None,
                value: Expr::Literal(Value::Int(1)),
            },
        ]);
        lowerer.lower_block(&[block]);
        // After block, x should be removed from locals
        assert!(!lowerer.locals.contains_key("x"));
    }

    #[test]
    fn test_lower_if() {
        let mut lowerer = AstLowerer::new();
        let stmt = Stmt::If {
            condition: Expr::Literal(Value::Boolean(true)),
            then_branch: Box::new(Stmt::Return(Some(Expr::Literal(Value::Int(1))))),
            else_branch: Some(Box::new(Stmt::Return(Some(Expr::Literal(Value::Int(2)))))),
        };
        let result = lowerer.lower(&[stmt]);
        // Should have JumpIfFalse, Label instructions
        let has_jump_if_false = result.instrs.iter().any(|i| matches!(i, IrInstr::JumpIfFalse { .. }));
        let has_label = result.instrs.iter().any(|i| matches!(i, IrInstr::Label { .. }));
        assert!(has_jump_if_false);
        assert!(has_label);
    }

    #[test]
    fn test_lower_while() {
        let mut lowerer = AstLowerer::new();
        let stmt = Stmt::While {
            condition: Expr::Literal(Value::Boolean(true)),
            body: Box::new(Stmt::Block(vec![])),
        };
        let result = lowerer.lower(&[stmt]);
        // Should have Label, JumpIfFalse, Jump
        let labels = result.instrs.iter().filter(|i| matches!(i, IrInstr::Label { .. })).count();
        let jumps = result.instrs.iter().filter(|i| matches!(i, IrInstr::Jump { .. })).count();
        assert!(labels >= 2); // top and end labels
        assert!(jumps >= 1); // jump to top (condition always true, so no jump to end needed)
    }

    #[test]
    fn test_lower_for() {
        let mut lowerer = AstLowerer::new();
        lowerer.locals.insert("items".to_string(), 0); // mock iterable
        let stmt = Stmt::For {
            variable: make_token(TokenType::Identifier, "i"),
            iterable: Expr::Variable(make_token(TokenType::Identifier, "items")),
            body: Box::new(Stmt::Block(vec![])),
        };
        let result = lowerer.lower(&[stmt]);
        // Should have while loop structure with labels and jumps
        let labels = result.instrs.iter().filter(|i| matches!(i, IrInstr::Label { .. })).count();
        let jumps = result.instrs.iter().filter(|i| matches!(i, IrInstr::Jump { .. })).count();
        // Note: __iter is cleaned up after the loop body by lower_block scope restoration
        assert!(labels >= 2); // top and end labels
        assert!(jumps >= 1); // jump to continue loop
    }

    #[test]
    fn test_lower_call() {
        let mut lowerer = AstLowerer::new();
        lowerer.locals.insert("print".to_string(), 0); // mock print function
        let expr = Expr::Call {
            callee: Box::new(Expr::Variable(make_token(TokenType::Identifier, "print"))),
            paren: make_token(TokenType::LParen, "("),
            arguments: vec![Expr::Literal(Value::Int(1))],
        };
        let _dst = lowerer.lower_expr(&expr, 0);
        let has_call = lowerer.instrs.iter().any(|i| matches!(i, IrInstr::Call { .. }));
        assert!(has_call);
    }

    #[test]
    fn test_lower_get_field() {
        let mut lowerer = AstLowerer::new();
        let expr = Expr::Get {
            obj: Box::new(Expr::Variable(make_token(TokenType::Identifier, "obj"))),
            name: make_token(TokenType::Identifier, "field"),
        };
        let _dst = lowerer.lower_expr(&expr, 0);
        let has_get_field = lowerer.instrs.iter().any(|i| matches!(i, IrInstr::GetField { .. }));
        assert!(has_get_field);
    }

    #[test]
    fn test_lower_has_check() {
        let mut lowerer = AstLowerer::new();
        let expr = Expr::Has {
            target: Box::new(Expr::Variable(make_token(TokenType::Identifier, "obj"))),
            field: Box::new(Expr::Literal(Value::String("field".to_string()))),
        };
        let _dst = lowerer.lower_expr(&expr, 0);
        let has_has_check = lowerer.instrs.iter().any(|i| matches!(i, IrInstr::HasCheck { .. }));
        assert!(has_has_check);
    }

    #[test]
    fn test_lower_elvis() {
        let mut lowerer = AstLowerer::new();
        let expr = Expr::Elvis {
            left: Box::new(Expr::Variable(make_token(TokenType::Identifier, "x"))),
            right: Box::new(Expr::Literal(Value::Int(0))),
        };
        let _dst = lowerer.lower_expr(&expr, 0);
        // Should have JumpIfFalse for null check
        let has_jump = lowerer.instrs.iter().any(|i| matches!(i, IrInstr::JumpIfFalse { .. }));
        assert!(has_jump);
    }

    #[test]
    fn test_lower_safe_call() {
        let mut lowerer = AstLowerer::new();
        let expr = Expr::SafeCall {
            obj: Box::new(Expr::Variable(make_token(TokenType::Identifier, "obj"))),
            name: make_token(TokenType::Identifier, "method"),
        };
        let _dst = lowerer.lower_expr(&expr, 0);
        // Should have null check and GetField
        let has_get_field = lowerer.instrs.iter().any(|i| matches!(i, IrInstr::GetField { .. }));
        assert!(has_get_field);
    }

    #[test]
    fn test_lower_list() {
        let mut lowerer = AstLowerer::new();
        let expr = Expr::List(vec![
            Expr::Literal(Value::Int(1)),
            Expr::Literal(Value::Int(2)),
        ]);
        let _dst = lowerer.lower_expr(&expr, 0);
        let has_new_array = lowerer.instrs.iter().any(|i| matches!(i, IrInstr::NewArray { .. }));
        assert!(has_new_array);
    }

    #[test]
    fn test_add_constant_dedup_first_constant() {
        let mut lowerer = AstLowerer::new();
        let idx = lowerer.add_constant(Value::Int(1));
        assert_eq!(idx, 0);
        assert_eq!(lowerer.constants, vec![Value::Int(1)]);
    }

    #[test]
    fn test_add_constant_dedup_duplicate_returns_existing() {
        let mut lowerer = AstLowerer::new();
        let idx0 = lowerer.add_constant(Value::Int(1));
        let idx1 = lowerer.add_constant(Value::Int(1));
        assert_eq!(idx0, 0);
        assert_eq!(idx1, 0);
        assert_eq!(lowerer.constants.len(), 1);
    }

    #[test]
    fn test_add_constant_dedup_different_values() {
        let mut lowerer = AstLowerer::new();
        let a = lowerer.add_constant(Value::Int(1));
        let b = lowerer.add_constant(Value::Int(2));
        assert_eq!(a, 0);
        assert_eq!(b, 1);
        assert_eq!(lowerer.constants.len(), 2);
    }

    #[test]
    fn test_add_constant_dedup_string() {
        let mut lowerer = AstLowerer::new();
        let a = lowerer.add_constant(Value::String("foo".to_string()));
        let b = lowerer.add_constant(Value::String("foo".to_string()));
        assert_eq!(a, b);
        assert_eq!(lowerer.constants.len(), 1);
    }

    #[test]
    fn test_add_constant_dedup_boolean() {
        let mut lowerer = AstLowerer::new();
        let a = lowerer.add_constant(Value::Boolean(true));
        let b = lowerer.add_constant(Value::Boolean(true));
        assert_eq!(a, b);
        assert_eq!(lowerer.constants.len(), 1);
    }

    #[test]
    fn test_add_constant_dedup_null() {
        let mut lowerer = AstLowerer::new();
        let a = lowerer.add_constant(Value::Null);
        let b = lowerer.add_constant(Value::Null);
        assert_eq!(a, b);
        assert_eq!(lowerer.constants.len(), 1);
    }

    #[test]
    fn test_add_constant_dedup_mixed_unique() {
        let mut lowerer = AstLowerer::new();
        let a = lowerer.add_constant(Value::Int(0));
        let b = lowerer.add_constant(Value::Int(1));
        let c = lowerer.add_constant(Value::Int(0)); // duplicate
        let d = lowerer.add_constant(Value::Int(2));
        assert_eq!(a, 0);
        assert_eq!(b, 1);
        assert_eq!(c, 0); // returns existing
        assert_eq!(d, 2);
        assert_eq!(lowerer.constants.len(), 3);
    }

    #[test]
    fn test_add_constant_dedup_float() {
        let mut lowerer = AstLowerer::new();
        let a = lowerer.add_constant(Value::Float(1.0));
        let b = lowerer.add_constant(Value::Float(1.0));
        assert_eq!(a, b);
        assert_eq!(lowerer.constants.len(), 1);
    }

    #[test]
    fn test_add_constant_dedup_distinct_floats() {
        let mut lowerer = AstLowerer::new();
        let a = lowerer.add_constant(Value::Float(1.0));
        let b = lowerer.add_constant(Value::Float(2.0));
        assert_eq!(a, 0);
        assert_eq!(b, 1);
    }

    #[test]
    fn test_add_constant_dedup_nan_not_deduplicated() {
        // f32::NAN != f32::NAN under IEEE 754, so two NaN entries are expected
        let mut lowerer = AstLowerer::new();
        let a = lowerer.add_constant(Value::Float(f32::NAN));
        let b = lowerer.add_constant(Value::Float(f32::NAN));
        assert_ne!(a, b);
        assert_eq!(lowerer.constants.len(), 2);
    }
}
