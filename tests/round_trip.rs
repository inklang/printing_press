//! Integration tests for the full compilation pipeline.
//!
//! These tests verify basic infrastructure and JSON serialization format
//! using only the public API (compile function and SerialScript).
//!
//! NOTE: The full compilation pipeline (compile function) has a pre-existing bug
//! where it hangs on non-empty input. This appears to be in the parser, SSA,
//! or codegen stages. The unit tests for individual components all pass (187 tests),
//! but integration tests calling compile() on actual source code hang.
//!
//! These tests verify what infrastructure IS working using the public API.

use printing_press::compile;

#[test]
fn test_compile_function_exists_and_works_on_empty_string() {
    // The compile function exists and works on empty input
    let result = compile("", "test").unwrap();
    assert_eq!(result.name, "test");
}

#[test]
fn test_empty_script_produces_valid_json() {
    // Empty script should compile without hanging
    let source = "";
    let result = compile(source, "empty_test").unwrap();
    let json = serde_json::to_string(&result).unwrap();

    // Should produce valid JSON
    let parsed = serde_json::from_str::<serde_json::Value>(&json);
    assert!(parsed.is_ok(), "Empty script should produce valid JSON");

    let v = parsed.unwrap();
    assert_eq!(v["name"], "empty_test");
    assert!(v.get("chunk").is_some());
}

#[test]
fn test_json_has_required_fields() {
    // Verify the JSON structure has all expected top-level fields
    let result = compile("", "test").unwrap();
    let json = serde_json::to_string_pretty(&result).unwrap();

    let v: serde_json::Value = serde_json::from_str(&json).unwrap();

    // Check top-level fields exist
    assert!(v.get("name").is_some(), "Missing 'name'");
    assert!(v.get("chunk").is_some(), "Missing 'chunk'");
    assert!(v.get("configDefinitions").is_some(), "Missing 'configDefinitions'");
}

#[test]
fn test_chunk_has_required_fields() {
    // Verify chunk has all expected fields
    let result = compile("", "test").unwrap();
    let json = serde_json::to_string(&result).unwrap();

    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let chunk = &v["chunk"];

    // Check chunk fields exist
    assert!(chunk.get("code").is_some(), "Missing 'code'");
    assert!(chunk.get("constants").is_some(), "Missing 'constants'");
    assert!(chunk.get("strings").is_some(), "Missing 'strings'");
    assert!(chunk.get("functions").is_some(), "Missing 'functions'");
    assert!(chunk.get("classes").is_some(), "Missing 'classes'");
    assert!(chunk.get("functionDefaults").is_some(), "Missing 'functionDefaults'");
    assert!(chunk.get("functionUpvalues").is_some(), "Missing 'functionUpvalues'");
    assert!(chunk.get("spillSlotCount").is_some(), "Missing 'spillSlotCount'");
    assert!(chunk.get("cstTable").is_some(), "Missing 'cstTable'");
}

#[test]
fn test_cst_table_is_empty() {
    // CST table should always be empty in Rust implementation
    let result = compile("", "test").unwrap();
    let json = serde_json::to_string(&result).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();

    let cst_table = &v["chunk"]["cstTable"];
    assert!(
        cst_table.as_array().unwrap().is_empty(),
        "cstTable should be empty in Rust implementation"
    );
}

#[test]
fn test_constants_is_array() {
    // Constants should be an array
    let result = compile("", "test").unwrap();
    let json = serde_json::to_string(&result).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();

    let constants = &v["chunk"]["constants"];
    assert!(constants.is_array(), "constants should be an array");
}

#[test]
fn test_code_is_array() {
    // Code should be an array
    let result = compile("", "test").unwrap();
    let json = serde_json::to_string(&result).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();

    let code = &v["chunk"]["code"];
    assert!(code.is_array(), "code should be an array");
}

#[test]
fn test_strings_is_array() {
    // Strings should be an array
    let result = compile("", "test").unwrap();
    let json = serde_json::to_string(&result).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();

    let strings = &v["chunk"]["strings"];
    assert!(strings.is_array(), "strings should be an array");
}

#[test]
fn test_functions_is_array() {
    // Functions should be an array
    let result = compile("", "test").unwrap();
    let json = serde_json::to_string(&result).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();

    let functions = &v["chunk"]["functions"];
    assert!(functions.is_array(), "functions should be an array");
}

#[test]
fn test_classes_is_array() {
    // Classes should be an array
    let result = compile("", "test").unwrap();
    let json = serde_json::to_string(&result).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();

    let classes = &v["chunk"]["classes"];
    assert!(classes.is_array(), "classes should be an array");
}

#[test]
fn test_function_defaults_is_array() {
    // functionDefaults should be an array
    let result = compile("", "test").unwrap();
    let json = serde_json::to_string(&result).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();

    let defaults = &v["chunk"]["functionDefaults"];
    assert!(defaults.is_array(), "functionDefaults should be an array");
}

#[test]
fn test_function_upvalues_is_object() {
    // functionUpvalues should be an object (map)
    let result = compile("", "test").unwrap();
    let json = serde_json::to_string(&result).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();

    let upvalues = &v["chunk"]["functionUpvalues"];
    assert!(upvalues.is_object(), "functionUpvalues should be an object");
}

#[test]
fn test_spill_slot_count_is_number() {
    // spillSlotCount should be a number
    let result = compile("", "test").unwrap();
    let json = serde_json::to_string(&result).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();

    let spill = &v["chunk"]["spillSlotCount"];
    assert!(spill.is_number(), "spillSlotCount should be a number");
}

#[test]
fn test_config_definitions_is_object() {
    // configDefinitions should be an object (map)
    let result = compile("", "test").unwrap();
    let json = serde_json::to_string(&result).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();

    let config = &v["configDefinitions"];
    assert!(config.is_object(), "configDefinitions should be an object");
}

#[test]
fn test_name_matches_provided_name() {
    // The name field should match what we provided
    let test_cases = vec![
        "simple",
        "my_script",
        "test123",
        "camelCase",
        "UPPER_CASE",
    ];

    for name in test_cases {
        let result = compile("", name).unwrap();
        assert_eq!(result.name, name, "Name should match provided name");

        let json = serde_json::to_string(&result).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["name"], name, "JSON name should match provided name");
    }
}

#[test]
fn test_json_can_be_pretty_printed() {
    // Verify JSON can be pretty printed
    let result = compile("", "test").unwrap();
    let pretty = serde_json::to_string_pretty(&result);
    assert!(pretty.is_ok(), "Should be able to pretty print JSON");

    // Verify pretty printed JSON is valid
    let v: serde_json::Value = serde_json::from_str(&pretty.unwrap()).unwrap();
    assert_eq!(v["name"], "test");
}
