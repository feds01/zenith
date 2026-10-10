//! A dense bit set of graph nodes.

use std::{
    fmt,
    hash::{Hash, Hasher},
};

use crate::ModuleId;

/// The number of nodes one word of a [`NodeSet`] holds.
const WORD_BITS: usize = u64::BITS as usize;

/// A set of [`ModuleId`]s, stored as a bit set indexed by id.
///
/// Iteration is in ascending id order, which is also the alphabetical order
/// of node names, so anything derived from a set is deterministic.
#[derive(Clone, Default)]
pub struct NodeSet {
    /// One bit per node id, least significant bit first. Trailing words may
    /// be zero, so equality and hashing ignore them.
    words: Vec<u64>,
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
        let (word, bit) = position(id);
        if word >= self.words.len() {
            self.words.resize(word + 1, 0);
        }
        let added = self.words[word] & bit == 0;
        self.words[word] |= bit;
        added
    }

    /// Whether the set contains a node.
    pub fn contains(&self, id: ModuleId) -> bool {
        let (word, bit) = position(id);
        self.words.get(word).is_some_and(|word| word & bit != 0)
    }

    /// Whether the set is empty.
    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|word| *word == 0)
    }

    /// The number of nodes in the set.
    pub fn len(&self) -> usize {
        self.words.iter().map(|word| word.count_ones() as usize).sum()
    }

    /// Add every node of `other` to this set.
    pub fn union_with(&mut self, other: &NodeSet) {
        if other.words.len() > self.words.len() {
            self.words.resize(other.words.len(), 0);
        }
        for (word, other) in self.words.iter_mut().zip(&other.words) {
            *word |= other;
        }
    }

    /// Remove every node of `other` from this set.
    pub fn difference_with(&mut self, other: &NodeSet) {
        for (word, other) in self.words.iter_mut().zip(&other.words) {
            *word &= !other;
        }
    }

    /// Whether this set shares any node with `other`.
    pub fn intersects(&self, other: &NodeSet) -> bool {
        self.words.iter().zip(&other.words).any(|(a, b)| a & b != 0)
    }

    /// Iterate over the nodes in ascending id order.
    pub fn iter(&self) -> impl Iterator<Item = ModuleId> + '_ {
        self.words.iter().enumerate().flat_map(|(index, word)| {
            let mut word = *word;
            std::iter::from_fn(move || {
                if word == 0 {
                    return None;
                }
                let bit = word.trailing_zeros() as usize;
                // Clear the lowest set bit.
                word &= word - 1;
                Some(ModuleId::from_index(index * WORD_BITS + bit))
            })
        })
    }

    /// The words up to and including the last non-zero one.
    fn significant_words(&self) -> &[u64] {
        let len = self.words.iter().rposition(|word| *word != 0).map_or(0, |last| last + 1);
        &self.words[..len]
    }
}

/// The word that holds `id`, and the bit for `id` within it.
fn position(id: ModuleId) -> (usize, u64) {
    let index = id.index();
    (index / WORD_BITS, 1 << (index % WORD_BITS))
}

impl PartialEq for NodeSet {
    fn eq(&self, other: &Self) -> bool {
        self.significant_words() == other.significant_words()
    }
}

impl Eq for NodeSet {}

impl Hash for NodeSet {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.significant_words().hash(state);
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
        indices.iter().map(|index| ModuleId::from_index(*index)).collect()
    }

    fn indices(set: &NodeSet) -> Vec<usize> {
        set.iter().map(ModuleId::index).collect()
    }

    #[test]
    fn inserts_and_finds_nodes_across_words() {
        let mut nodes = NodeSet::new();
        assert!(nodes.is_empty());
        assert!(nodes.insert(ModuleId::from_index(200)));
        assert!(nodes.insert(ModuleId::from_index(1)));
        assert!(nodes.insert(ModuleId::from_index(64)));
        assert!(!nodes.insert(ModuleId::from_index(64)));

        assert!(!nodes.is_empty());
        assert_eq!(nodes.len(), 3);
        assert!(nodes.contains(ModuleId::from_index(64)));
        assert!(!nodes.contains(ModuleId::from_index(63)));
        assert!(!nodes.contains(ModuleId::from_index(10_000)));
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

        nodes.extend([ModuleId::from_index(5)]);
        assert_eq!(indices(&nodes), [2, 5, 64, 200]);
    }

    #[test]
    fn equal_sets_compare_and_hash_equal_however_they_were_built() {
        // Removing the only node of the last word leaves a trailing zero word.
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
