//! A dense bit set of graph nodes.

use crate::ModuleId;

/// A set of [ModuleId]s, stored as a bit set. Iteration is in ascending id
/// order, which is also the alphabetical order of node names.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct NodeSet {
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

    /// Add a node to the set. Returns `true` if it was not already present.
    pub fn insert(&mut self, id: ModuleId) -> bool {
        let (word, bit) = Self::position(id);
        if word >= self.words.len() {
            self.words.resize(word + 1, 0);
        }
        let was_present = self.words[word] & bit != 0;
        self.words[word] |= bit;
        !was_present
    }

    /// Remove a node from the set.
    pub fn remove(&mut self, id: ModuleId) {
        let (word, bit) = Self::position(id);
        if let Some(word) = self.words.get_mut(word) {
            *word &= !bit;
        }
    }

    /// Whether the set contains a node.
    pub fn contains(&self, id: ModuleId) -> bool {
        let (word, bit) = Self::position(id);
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
                let bit = word.trailing_zeros();
                word &= word - 1;
                Some(ModuleId::from_index(index * 64 + bit as usize))
            })
        })
    }

    fn position(id: ModuleId) -> (usize, u64) {
        let index = id.index();
        (index / 64, 1 << (index % 64))
    }
}

impl FromIterator<ModuleId> for NodeSet {
    fn from_iter<T: IntoIterator<Item = ModuleId>>(iter: T) -> Self {
        let mut set = NodeSet::new();
        for id in iter {
            set.insert(id);
        }
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
    use super::*;

    #[test]
    fn set_operations() {
        let ids = |xs: &[usize]| xs.iter().map(|x| ModuleId::from_index(*x)).collect::<NodeSet>();
        let mut a = ids(&[1, 64, 200]);
        assert!(a.contains(ModuleId::from_index(64)));
        assert!(!a.contains(ModuleId::from_index(65)));
        assert_eq!(a.len(), 3);
        assert!(a.intersects(&ids(&[200])));
        assert!(!a.intersects(&ids(&[2])));
        a.union_with(&ids(&[2]));
        a.difference_with(&ids(&[1]));
        assert_eq!(a.iter().map(|id| id.index()).collect::<Vec<_>>(), vec![2, 64, 200]);
        assert!(NodeSet::new().is_empty());
    }
}
