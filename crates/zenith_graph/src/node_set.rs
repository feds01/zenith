//! A dense bit set of graph nodes.

use std::{
    fmt,
    hash::{Hash, Hasher},
};

use fixedbitset::{Block, FixedBitSet};

use crate::ModuleId;

/// A set of [`ModuleId`]s, stored as a bit set indexed by id.
///
/// Iteration is in ascending id order, which is also the alphabetical order
/// of node names, so anything derived from a set is deterministic.
#[derive(Clone, Default)]
pub struct NodeSet {
    /// One bit per node id. The bit set grows to fit the largest id inserted,
    /// so two equal sets can differ in length: equality and hashing ignore
    /// trailing zero blocks, where [`FixedBitSet`]'s own compare the length.
    bits: FixedBitSet,
}

impl NodeSet {
    /// Create an empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a set containing a single node.
    pub fn single(id: ModuleId) -> Self {
        let mut set = Self::new();
        set.insert(id);
        set
    }

    /// Add a node to the set. Returns whether it was newly added.
    pub fn insert(&mut self, id: ModuleId) -> bool {
        let added = !self.bits.contains(id.index());
        self.bits.grow_and_insert(id.index());
        added
    }

    /// Whether the set contains a node.
    pub fn contains(&self, id: ModuleId) -> bool {
        self.bits.contains(id.index())
    }

    /// Whether the set is empty.
    pub fn is_empty(&self) -> bool {
        self.bits.is_clear()
    }

    /// The number of nodes in the set.
    pub fn len(&self) -> usize {
        self.bits.count_ones(..)
    }

    /// Add every node of `other` to this set.
    pub fn union_with(&mut self, other: &NodeSet) {
        self.bits.union_with(&other.bits);
    }

    /// Remove every node of `other` from this set.
    pub fn difference_with(&mut self, other: &NodeSet) {
        self.bits.difference_with(&other.bits);
    }

    /// Whether this set shares any node with `other`.
    pub fn intersects(&self, other: &NodeSet) -> bool {
        !self.bits.is_disjoint(&other.bits)
    }

    /// Iterate over the nodes in ascending id order.
    pub fn iter(&self) -> impl Iterator<Item = ModuleId> + '_ {
        self.bits.ones().map(ModuleId::new)
    }

    /// The blocks up to and including the last non-zero one.
    fn significant_blocks(&self) -> &[Block] {
        let blocks = self.bits.as_slice();
        let len = blocks.iter().rposition(|block| *block != 0).map_or(0, |last| last + 1);
        &blocks[..len]
    }
}

impl PartialEq for NodeSet {
    fn eq(&self, other: &Self) -> bool {
        self.significant_blocks() == other.significant_blocks()
    }
}

impl Eq for NodeSet {}

impl Hash for NodeSet {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.significant_blocks().hash(state);
    }
}

impl fmt::Debug for NodeSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.iter().map(ModuleId::index)).finish()
    }
}

impl FromIterator<ModuleId> for NodeSet {
    fn from_iter<T: IntoIterator<Item = ModuleId>>(iter: T) -> Self {
        let mut set = NodeSet::new();
        set.extend(iter);
        set
    }
}

impl Extend<ModuleId> for NodeSet {
    fn extend<T: IntoIterator<Item = ModuleId>>(&mut self, iter: T) {
        for id in iter {
            self.insert(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::hash::BuildHasher;

    use rustc_hash::FxBuildHasher;

    use super::*;

    fn set(indices: &[usize]) -> NodeSet {
        indices.iter().map(|index| ModuleId::new(*index)).collect()
    }

    fn indices(set: &NodeSet) -> Vec<usize> {
        set.iter().map(ModuleId::index).collect()
    }

    #[test]
    fn inserts_and_finds_nodes_across_blocks() {
        let mut nodes = NodeSet::new();
        assert!(nodes.is_empty());
        assert!(nodes.insert(ModuleId::new(200)));
        assert!(nodes.insert(ModuleId::new(1)));
        assert!(nodes.insert(ModuleId::new(64)));
        assert!(!nodes.insert(ModuleId::new(64)));

        assert!(!nodes.is_empty());
        assert_eq!(nodes.len(), 3);
        assert!(nodes.contains(ModuleId::new(64)));
        assert!(!nodes.contains(ModuleId::new(63)));
        assert!(!nodes.contains(ModuleId::new(10_000)));
        assert_eq!(indices(&nodes), [1, 64, 200]);
        assert_eq!(indices(&NodeSet::single(ModuleId::ROOT)), [0]);
    }

    #[test]
    fn combines_sets() {
        let mut nodes = set(&[1, 64, 200]);
        assert!(nodes.intersects(&set(&[200])));
        assert!(!nodes.intersects(&set(&[2, 300])));
        assert!(!nodes.intersects(&NodeSet::new()));

        nodes.union_with(&set(&[2, 300]));
        assert_eq!(indices(&nodes), [1, 2, 64, 200, 300]);

        nodes.difference_with(&set(&[1, 300, 400]));
        assert_eq!(indices(&nodes), [2, 64, 200]);

        nodes.extend([ModuleId::new(5)]);
        assert_eq!(indices(&nodes), [2, 5, 64, 200]);
    }

    #[test]
    fn equal_sets_compare_and_hash_equal_however_they_were_built() {
        // Removing the only node of the last block leaves a trailing zero
        // block.
        let mut shrunk = set(&[3, 500]);
        shrunk.difference_with(&set(&[500]));
        let direct = set(&[3]);
        assert_eq!(shrunk, direct);
        assert_eq!(FxBuildHasher.hash_one(&shrunk), FxBuildHasher.hash_one(&direct));

        let mut emptied = set(&[700]);
        emptied.difference_with(&set(&[700]));
        assert!(emptied.is_empty());
        assert_eq!(emptied, NodeSet::new());
        assert_ne!(direct, set(&[4]));
    }

    #[test]
    fn debug_lists_the_node_indices() {
        assert_eq!(format!("{:?}", set(&[70, 2])), "{2, 70}");
    }
}
