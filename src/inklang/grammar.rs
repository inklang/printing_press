//! Grammar IR types and loading for Inklang grammars.
//!
//! This module provides types for deserializing grammar IR files (JSON format)
//! produced by quill's grammar serializer.

use std::collections::HashMap;
use std::io::Read;
use serde::{Deserialize, Serialize};
use super::CompileError;

/// A rule in the grammar, represented as a tagged enum.
/// Corresponds to the TypeScript `Rule` type.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Rule {
    #[serde(rename = "seq")]
    Seq { items: Vec<Rule> },
    #[serde(rename = "choice")]
    Choice { items: Vec<Rule> },
    #[serde(rename = "many")]
    Many { item: Box<Rule> },
    #[serde(rename = "many1")]
    Many1 { item: Box<Rule> },
    #[serde(rename = "optional")]
    Optional { item: Box<Rule> },
    #[serde(rename = "ref")]
    Ref { rule: String },
    #[serde(rename = "keyword")]
    Keyword { value: String },
    #[serde(rename = "literal")]
    Literal { value: String },
    #[serde(rename = "identifier")]
    Identifier,
    #[serde(rename = "int")]
    Int,
    #[serde(rename = "float")]
    Float,
    #[serde(rename = "string")]
    String,
    #[serde(rename = "block")]
    Block { scope: Option<String> },
}

/// A rule entry with an optional handler.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleEntry {
    pub rule: Rule,
    #[serde(default)]
    pub handler: Option<String>,
}

/// Declaration definition in a grammar package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeclarationDef {
    #[serde(rename = "keyword")]
    pub keyword: String,
    #[serde(rename = "nameRule")]
    pub name_rule: Rule,
    #[serde(rename = "scopeRules")]
    pub scope_rules: Vec<String>,
    #[serde(rename = "inheritsBase")]
    pub inherits_base: bool,
    #[serde(default)]
    pub handler: Option<String>,
}

/// GrammarPackage: top-level grammar IR.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrammarPackage {
    pub version: u32,
    pub package: String,
    pub keywords: Vec<String>,
    pub rules: HashMap<String, RuleEntry>,
    pub declarations: Vec<DeclarationDef>,
}

/// MergedGrammar: combined grammar from multiple packages.
#[derive(Debug, Clone)]
pub struct MergedGrammar {
    pub keywords: Vec<String>,
    pub rules: HashMap<String, RuleEntry>,
    pub declarations: Vec<DeclarationDef>,
}

/// Load a grammar package from a JSON file.
pub fn load_grammar(path: &str) -> Result<GrammarPackage, CompileError> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| CompileError::Other(format!("Failed to open grammar file '{}': {}", path, e)))?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .map_err(|e| CompileError::Other(format!("Failed to read grammar file '{}': {}", path, e)))?;
    serde_json::from_str(&contents)
        .map_err(|e| CompileError::Other(format!("Failed to parse grammar JSON from '{}': {}", path, e)))
}

/// Auto-discover grammars by scanning:
/// 1. `dist/grammar.ir.json` (project's own grammar)
/// 2. `packages/*/dist/grammar.ir.json` (installed packages)
pub fn discover_grammars() -> Option<MergedGrammar> {
    let mut packages = Vec::new();

    // Scan dist/grammar.ir.json
    if let Ok(pkg) = load_grammar("dist/grammar.ir.json") {
        packages.push(pkg);
    }

    // Scan packages/*/dist/grammar.ir.json
    if let Ok(entries) = std::fs::read_dir("packages") {
        for entry in entries.filter_map(|e| e.ok()) {
            let pkg_path = entry.path();
            if pkg_path.is_dir() {
                let grammar_path = pkg_path.join("dist/grammar.ir.json");
                if grammar_path.exists() {
                    if let Ok(pkg) = load_grammar(grammar_path.to_str().unwrap_or("")) {
                        packages.push(pkg);
                    }
                }
            }
        }
    }

    if packages.is_empty() {
        None
    } else {
        Some(merge_grammars(packages))
    }
}

/// Merge multiple grammar packages into a single MergedGrammar.
/// Keywords are deduplicated via sort+dedup.
pub fn merge_grammars(packages: Vec<GrammarPackage>) -> MergedGrammar {
    let mut all_keywords: Vec<String> = Vec::new();
    let mut all_rules: HashMap<String, RuleEntry> = HashMap::new();
    let mut all_declarations: Vec<DeclarationDef> = Vec::new();

    for pkg in packages {
        all_keywords.extend(pkg.keywords);
        all_rules.extend(pkg.rules);
        all_declarations.extend(pkg.declarations);
    }

    // Deduplicate keywords via sort + dedup
    all_keywords.sort();
    all_keywords.dedup();

    MergedGrammar {
        keywords: all_keywords,
        rules: all_rules,
        declarations: all_declarations,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_grammar() {
        let json = r#"{
            "version": 1,
            "package": "test.grammar",
            "keywords": ["if", "else", "while"],
            "rules": {
                "expression": {
                    "rule": {"type": "choice", "items": [
                        {"type": "identifier"},
                        {"type": "int"}
                    ]},
                    "handler": "handleExpr"
                }
            },
            "declarations": []
        }"#;

        let pkg: GrammarPackage = serde_json::from_str(json).unwrap();
        assert_eq!(pkg.version, 1);
        assert_eq!(pkg.package, "test.grammar");
        assert_eq!(pkg.keywords.len(), 3);
        assert_eq!(pkg.rules.len(), 1);
        assert!(pkg.declarations.is_empty());
    }

    #[test]
    fn test_merge_grammars_deduplicates_keywords() {
        let json1 = r#"{
            "version": 1,
            "package": "pkg1",
            "keywords": ["if", "else"],
            "rules": {},
            "declarations": []
        }"#;
        let json2 = r#"{
            "version": 1,
            "package": "pkg2",
            "keywords": ["else", "while", "for"],
            "rules": {},
            "declarations": []
        }"#;

        let pkg1: GrammarPackage = serde_json::from_str(json1).unwrap();
        let pkg2: GrammarPackage = serde_json::from_str(json2).unwrap();

        let merged = merge_grammars(vec![pkg1, pkg2]);

        // Should have 4 unique keywords: if, else, while, for
        assert_eq!(merged.keywords.len(), 4);
        assert!(merged.keywords.contains(&"if".to_string()));
        assert!(merged.keywords.contains(&"else".to_string()));
        assert!(merged.keywords.contains(&"while".to_string()));
        assert!(merged.keywords.contains(&"for".to_string()));
    }

    #[test]
    fn test_merge_grammars_combines_rules() {
        let json1 = r#"{
            "version": 1,
            "package": "pkg1",
            "keywords": [],
            "rules": {
                "rule1": {"rule": {"type": "identifier"}}
            },
            "declarations": []
        }"#;
        let json2 = r#"{
            "version": 1,
            "package": "pkg2",
            "keywords": [],
            "rules": {
                "rule2": {"rule": {"type": "int"}}
            },
            "declarations": []
        }"#;

        let pkg1: GrammarPackage = serde_json::from_str(json1).unwrap();
        let pkg2: GrammarPackage = serde_json::from_str(json2).unwrap();

        let merged = merge_grammars(vec![pkg1, pkg2]);

        assert_eq!(merged.rules.len(), 2);
        assert!(merged.rules.contains_key("rule1"));
        assert!(merged.rules.contains_key("rule2"));
    }

    #[test]
    fn test_rule_variants() {
        // Test Seq
        let json = r#"{"type": "seq", "items": [{"type": "int"}, {"type": "identifier"}]}"#;
        let rule: Rule = serde_json::from_str(json).unwrap();
        assert!(matches!(rule, Rule::Seq { .. }));

        // Test Choice
        let json = r#"{"type": "choice", "items": [{"type": "int"}, {"type": "identifier"}]}"#;
        let rule: Rule = serde_json::from_str(json).unwrap();
        assert!(matches!(rule, Rule::Choice { .. }));

        // Test Many
        let json = r#"{"type": "many", "item": {"type": "identifier"}}"#;
        let rule: Rule = serde_json::from_str(json).unwrap();
        assert!(matches!(rule, Rule::Many { .. }));

        // Test Many1
        let json = r#"{"type": "many1", "item": {"type": "identifier"}}"#;
        let rule: Rule = serde_json::from_str(json).unwrap();
        assert!(matches!(rule, Rule::Many1 { .. }));

        // Test Optional
        let json = r#"{"type": "optional", "item": {"type": "identifier"}}"#;
        let rule: Rule = serde_json::from_str(json).unwrap();
        assert!(matches!(rule, Rule::Optional { .. }));

        // Test Ref
        let json = r#"{"type": "ref", "rule": "myRule"}"#;
        let rule: Rule = serde_json::from_str(json).unwrap();
        assert!(matches!(rule, Rule::Ref { rule: r } if r == "myRule"));

        // Test Keyword
        let json = r#"{"type": "keyword", "value": "if"}"#;
        let rule: Rule = serde_json::from_str(json).unwrap();
        assert!(matches!(rule, Rule::Keyword { value: v } if v == "if"));

        // Test Literal
        let json = r#"{"type": "literal", "value": "hello"}"#;
        let rule: Rule = serde_json::from_str(json).unwrap();
        assert!(matches!(rule, Rule::Literal { value: v } if v == "hello"));

        // Test Identifier
        let json = r#"{"type": "identifier"}"#;
        let rule: Rule = serde_json::from_str(json).unwrap();
        assert!(matches!(rule, Rule::Identifier));

        // Test Int
        let json = r#"{"type": "int"}"#;
        let rule: Rule = serde_json::from_str(json).unwrap();
        assert!(matches!(rule, Rule::Int));

        // Test Float
        let json = r#"{"type": "float"}"#;
        let rule: Rule = serde_json::from_str(json).unwrap();
        assert!(matches!(rule, Rule::Float));

        // Test String
        let json = r#"{"type": "string"}"#;
        let rule: Rule = serde_json::from_str(json).unwrap();
        assert!(matches!(rule, Rule::String));

        // Test Block with null scope
        let json = r#"{"type": "block", "scope": null}"#;
        let rule: Rule = serde_json::from_str(json).unwrap();
        assert!(matches!(rule, Rule::Block { scope: None }));

        // Test Block with scope
        let json = r#"{"type": "block", "scope": "myScope"}"#;
        let rule: Rule = serde_json::from_str(json).unwrap();
        assert!(matches!(rule, Rule::Block { scope: Some(s) } if s == "myScope"));
    }

    #[test]
    fn test_declaration_def() {
        let json = r#"{
            "keyword": "event",
            "nameRule": {"type": "identifier"},
            "scopeRules": ["global", "entity"],
            "inheritsBase": true,
            "handler": "handleEvent"
        }"#;

        let decl: DeclarationDef = serde_json::from_str(json).unwrap();
        assert_eq!(decl.keyword, "event");
        assert!(matches!(decl.name_rule, Rule::Identifier));
        assert_eq!(decl.scope_rules.len(), 2);
        assert!(decl.inherits_base);
        assert_eq!(decl.handler, Some("handleEvent".to_string()));
    }

    #[test]
    fn test_rule_entry_with_handler() {
        let json = r#"{
            "rule": {"type": "identifier"},
            "handler": "handleId"
        }"#;
        let entry: RuleEntry = serde_json::from_str(json).unwrap();
        assert!(matches!(entry.rule, Rule::Identifier));
        assert_eq!(entry.handler, Some("handleId".to_string()));
    }

    #[test]
    fn test_rule_entry_without_handler() {
        let json = r#"{"rule": {"type": "int"}}"#;
        let entry: RuleEntry = serde_json::from_str(json).unwrap();
        assert!(matches!(entry.rule, Rule::Int));
        assert_eq!(entry.handler, None);
    }
}
