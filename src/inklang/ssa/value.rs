//! SSA Value types.
//!
//! In SSA form, each register has a base number and a version.
//! For example, r0.0 is the first definition of r0, r0.1 is the second, etc.

use std::fmt;

/// Represents a versioned SSA register.
/// In SSA form, each register has a base number and a version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SsaValue {
    /// The base register number (e.g., 0 for r0).
    pub base_reg: usize,
    /// The version number (e.g., 0 for r0.0).
    pub version: usize,
}

impl SsaValue {
    /// Create a new SSA value with the given base register and version.
    pub fn new(base_reg: usize, version: usize) -> Self {
        SsaValue { base_reg, version }
    }

    /// Represents an undefined/placeholder SSA value.
    /// Used during phi placement before renaming.
    pub const UNDEFINED: SsaValue = SsaValue { base_reg: 0, version: 0 };

    /// Check if this value is the undefined value.
    pub fn is_undefined(&self) -> bool {
        self.base_reg == 0 && self.version == 0
    }
}

impl fmt::Display for SsaValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r{}.{}", self.base_reg, self.version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ssa_value_new() {
        let val = SsaValue::new(5, 3);
        assert_eq!(val.base_reg, 5);
        assert_eq!(val.version, 3);
    }

    #[test]
    fn test_ssa_value_display() {
        let val = SsaValue::new(5, 3);
        assert_eq!(format!("{}", val), "r5.3");
    }

    #[test]
    fn test_ssa_value_equality() {
        let a = SsaValue::new(1, 2);
        let b = SsaValue::new(1, 2);
        let c = SsaValue::new(1, 3);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_ssa_value_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(SsaValue::new(1, 2));
        set.insert(SsaValue::new(1, 2)); // duplicate
        set.insert(SsaValue::new(3, 4));
        assert_eq!(set.len(), 2);
    }
}
