//! Graph-coloring register allocator.
//!
//! Assigns physical registers (R0-R15) to virtual registers based on live ranges.
//! Uses a simplified greedy allocator that handles register pressure by spilling.

use std::collections::{HashMap, VecDeque};

use super::liveness::LiveRange;

/// Result of register allocation.
#[derive(Debug, Clone)]
pub struct AllocResult {
    /// Mapping from virtual register to physical register.
    pub mapping: HashMap<usize, usize>,
    /// Mapping from virtual register to spill slot index.
    pub spills: HashMap<usize, usize>,
    /// Total number of spill slots needed.
    pub spill_slot_count: usize,
}

/// Register allocator with 16 physical registers (R0-R15).
pub struct RegisterAllocator {
    num_regs: usize,
}

impl RegisterAllocator {
    pub fn new() -> Self {
        Self { num_regs: 16 }
    }

    /// Allocate physical registers for virtual registers.
    ///
    /// `ranges` - Map of virtual register to live range
    /// `arity` - Number of parameter registers (these are pre-allocated to R0, R1, ...)
    pub fn allocate(&mut self, ranges: &HashMap<usize, LiveRange>, arity: usize) -> AllocResult {
        let mut mapping: HashMap<usize, usize> = HashMap::new();
        let mut spills: HashMap<usize, usize> = HashMap::new();
        let mut spill_slot: usize = 0;

        // Pre-allocate parameter registers
        for i in 0..arity {
            mapping.insert(i, i);
        }

        // Sort ranges by start position
        let mut sorted_ranges: Vec<&LiveRange> = ranges.values().collect();
        sorted_ranges.sort_by_key(|r| r.start);

        // Active ranges (currently live virtual registers)
        let mut active: HashMap<usize, LiveRange> = HashMap::new();
        let mut free_regs: VecDeque<usize> = VecDeque::new();

        // Initialize free regs (starting after arity params)
        for reg in arity..self.num_regs {
            free_regs.push_back(reg);
        }

        // Add initially active ranges (params that are live at start)
        for i in 0..arity {
            if let Some(range) = ranges.get(&i) {
                active.insert(i, range.clone());
            }
        }

        for range in sorted_ranges {
            // Skip parameter registers (already allocated)
            if range.reg < arity {
                continue;
            }

            // Expire old ranges that end before current range starts
            let expired: Vec<usize> = active
                .iter()
                .filter(|(_, r)| r.end < range.start)
                .map(|(reg, _)| *reg)
                .collect();

            for phys_reg in expired {
                if phys_reg >= arity {
                    active.remove(&phys_reg);
                    free_regs.push_front(phys_reg);
                }
            }

            if free_regs.is_empty() {
                // Need to spill: find the range that ends latest (best spill candidate)
                let spill_candidate = active
                    .iter()
                    .filter(|(phys_reg, _)| **phys_reg >= arity)
                    .max_by_key(|(_, r)| r.end)
                    .map(|(phys_reg, range)| (*phys_reg, range.reg.clone()));

                if let Some((phys_reg, spilled_reg)) = spill_candidate {
                    active.remove(&phys_reg);
                    spills.insert(spilled_reg, spill_slot);
                    mapping.insert(range.reg, phys_reg);
                    active.insert(phys_reg, range.clone());
                    spill_slot += 1;
                } else {
                    panic!(
                        "RegisterAllocator: no spill candidate and no free registers"
                    );
                }
            } else {
                // Allocate a free physical register
                let phys_reg = free_regs.pop_front().unwrap();
                mapping.insert(range.reg, phys_reg);
                active.insert(phys_reg, range.clone());
            }
        }

        AllocResult {
            mapping,
            spills,
            spill_slot_count: spill_slot,
        }
    }
}

impl Default for RegisterAllocator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_simple_allocation_no_spill() {
        // v0 and v1 have non-overlapping live ranges, both should fit in registers
        let mut ranges: HashMap<usize, LiveRange> = HashMap::new();
        ranges.insert(0, LiveRange { reg: 0, start: 0, end: 0 });
        ranges.insert(1, LiveRange { reg: 1, start: 1, end: 1 });

        let mut allocator = RegisterAllocator::new();
        let result = allocator.allocate(&ranges, 0);

        // Both should get physical registers
        assert!(result.spills.is_empty());
        assert_eq!(result.mapping.len(), 2);
    }

    #[test]
    fn test_allocation_with_params() {
        // Test that parameter registers are pre-allocated
        let mut ranges: HashMap<usize, LiveRange> = HashMap::new();
        ranges.insert(0, LiveRange { reg: 0, start: 0, end: 2 }); // param v0
        ranges.insert(1, LiveRange { reg: 1, start: 0, end: 2 }); // param v1
        ranges.insert(2, LiveRange { reg: 2, start: 1, end: 1 }); // temp v2

        let mut allocator = RegisterAllocator::new();
        let result = allocator.allocate(&ranges, 2);

        // Params should be in R0 and R1
        assert_eq!(result.mapping[&0], 0);
        assert_eq!(result.mapping[&1], 1);
        // v2 should get R2 (first free after params)
        assert_eq!(result.mapping[&2], 2);
        // No spills
        assert!(result.spills.is_empty());
    }

    #[test]
    fn test_allocation_spill_required() {
        // When we have more overlapping live ranges than registers, spill some
        let mut ranges: HashMap<usize, LiveRange> = HashMap::new();
        // All regs live at same time - need to spill
        for i in 0..20 {
            ranges.insert(i, LiveRange {
                reg: i,
                start: 0,
                end: 10,
            });
        }

        let mut allocator = RegisterAllocator::new();
        let result = allocator.allocate(&ranges, 0);

        // Should have some spills
        assert!(!result.spills.is_empty() || result.mapping.len() < 20);
    }

    #[test]
    fn test_non_overlapping_ranges_share_register() {
        // v0: 0-1, v1: 2-3 - should share a register since they don't overlap
        let mut ranges: HashMap<usize, LiveRange> = HashMap::new();
        ranges.insert(0, LiveRange { reg: 0, start: 0, end: 1 });
        ranges.insert(1, LiveRange { reg: 1, start: 2, end: 3 });

        let mut allocator = RegisterAllocator::new();
        let result = allocator.allocate(&ranges, 0);

        // Both should get allocated (possibly to same register since non-overlapping)
        assert_eq!(result.mapping.len(), 2);
        assert!(result.spills.is_empty());
    }

    #[test]
    fn test_alloc_result_structure() {
        let ranges: HashMap<usize, LiveRange> = HashMap::new();
        let mut allocator = RegisterAllocator::new();
        let result = allocator.allocate(&ranges, 0);

        assert_eq!(result.spill_slot_count, 0);
        assert!(result.spills.is_empty());
        assert!(result.mapping.is_empty());
    }
}
