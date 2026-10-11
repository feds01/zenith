//! Helpers for printing human-readable text.

/// `count` followed by `noun`, in the plural unless `count` is 1, e.g. `3
/// imports`. The plural adds an `s`.
pub fn plural(count: usize, noun: &str) -> String {
    if count == 1 { format!("1 {noun}") } else { format!("{count} {noun}s") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_one_is_singular() {
        assert_eq!(plural(0, "import"), "0 imports");
        assert_eq!(plural(1, "import"), "1 import");
        assert_eq!(plural(2, "source file"), "2 source files");
    }
}
