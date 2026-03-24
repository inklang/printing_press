//! Compile-time constant value types.

/// Compile-time constant values used in AST literals.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Boolean(bool),
    Int(i64),
    Float(f32),
    Double(f64),
    String(String),
    EventInfo {
        name: String,
        params: Vec<(String, String)>, // (param_name, param_type)
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_null() {
        let val = Value::Null;
        assert!(matches!(val, Value::Null));
    }

    #[test]
    fn test_value_boolean() {
        let true_val = Value::Boolean(true);
        let false_val = Value::Boolean(false);
        assert!(matches!(true_val, Value::Boolean(true)));
        assert!(matches!(false_val, Value::Boolean(false)));
    }

    #[test]
    fn test_value_int() {
        let val = Value::Int(42);
        assert!(matches!(val, Value::Int(42)));
    }

    #[test]
    fn test_value_float() {
        let val = Value::Float(3.14);
        assert!(matches!(val, Value::Float(_)));
    }

    #[test]
    fn test_value_double() {
        let val = Value::Double(3.14159265358979);
        assert!(matches!(val, Value::Double(_)));
    }

    #[test]
    fn test_value_string() {
        let val = Value::String("hello".to_string());
        assert!(matches!(val, Value::String(s) if s == "hello"));
    }

    #[test]
    fn test_value_event_info() {
        let val = Value::EventInfo {
            name: "player_join".to_string(),
            params: vec![("player".to_string(), "Player".to_string())],
        };
        match val {
            Value::EventInfo { name, params } => {
                assert_eq!(name, "player_join");
                assert_eq!(params.len(), 1);
                assert_eq!(params[0], ("player".to_string(), "Player".to_string()));
            }
            _ => panic!("Expected EventInfo"),
        }
    }

    #[test]
    fn test_value_clone() {
        let val = Value::Int(100);
        let cloned = val.clone();
        assert_eq!(val, cloned);
    }

    #[test]
    fn test_value_partial_eq() {
        let a = Value::Int(42);
        let b = Value::Int(42);
        let c = Value::Int(100);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
