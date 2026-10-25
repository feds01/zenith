mod clock;
mod store;

use std::collections::HashMap;

pub fn tally(words: &[&str]) -> HashMap<&str, usize> {
    let mut counts = HashMap::new();
    for word in words {
        *counts.entry(*word).or_default() += 1;
    }
    counts
}
