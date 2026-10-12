//! Helpers for printing human-readable text.

/// `count` followed by `noun`, in the plural unless `count` is 1, e.g. `3
/// imports`. The plural adds an `s`, or replaces a `y` after a consonant
/// with `ies`, as in `2 entries`.
pub fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        return format!("1 {noun}");
    }
    let mut letters = noun.chars().rev();
    match (letters.next(), letters.next()) {
        (Some('y'), Some(before)) if !"aeiou".contains(before) => {
            format!("{count} {}ies", &noun[..noun.len() - 1])
        }
        _ => format!("{count} {noun}s"),
    }
}

/// `count` with a `,` between each group of three digits, e.g. `1,234`.
pub fn thousands(count: usize) -> String {
    let digits = count.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_one_is_singular() {
        assert_eq!(plural(0, "import"), "0 imports");
        assert_eq!(plural(1, "import"), "1 import");
        assert_eq!(plural(2, "source file"), "2 source files");
        assert_eq!(plural(2, "baseline entry"), "2 baseline entries");
        assert_eq!(plural(1, "entry"), "1 entry");
        assert_eq!(plural(3, "key"), "3 keys");
    }

    #[test]
    fn separates_thousands() {
        for (count, shown) in [(0, "0"), (999, "999"), (1_000, "1,000"), (1_234_567, "1,234,567")] {
            assert_eq!(thousands(count), shown);
        }
    }
}
