//! Writing TOML by hand, for the files and snippets that zenith writes for
//! people to read and edit: strings, and arrays of them on one line or a
//! value per line.
//!
//! ```
//! # use zenith_utils::toml_writing;
//! let values = ["web".to_string(), "api".to_string()];
//! assert_eq!(toml_writing::array("modules", &values), "modules = [\"web\", \"api\"]\n");
//! ```

/// The longest line that [`array()`] writes an array on; a longer array is
/// written a value per line.
pub const LINE_WIDTH: usize = 80;

/// `value` as a TOML string, quoted and escaped.
pub fn string(value: &str) -> String {
    toml::Value::String(value.to_string()).to_string()
}

/// `key = [...]`, a line of the quoted `values`, or a value per line if they
/// don't fit on one.
pub fn array(key: &str, values: &[String]) -> String {
    let quoted: Vec<_> = values.iter().map(|value| string(value)).collect();
    let line = format!("{key} = [{}]\n", quoted.join(", "));
    match line.len() <= LINE_WIDTH + 1 {
        true => line,
        false => array_per_line(key, values),
    }
}

/// `key = [...]` with a quoted value per line, so that adding or removing one
/// changes only its line.
pub fn array_per_line(key: &str, values: &[String]) -> String {
    let mut lines = format!("{key} = [\n");
    for value in values {
        lines += &format!("    {},\n", string(value));
    }
    lines + "]\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_arrays_are_written_a_value_per_line() {
        let values: Vec<_> = (1..=8).map(|n| format!("packages/module-{n}")).collect();
        let written = array("modules", &values);
        assert!(written.starts_with("modules = [\n    \"packages/module-1\",\n"), "{written}");
        assert!(written.ends_with("    \"packages/module-8\",\n]\n"), "{written}");
        assert_eq!(array("modules", &values[..2]).lines().count(), 1);
    }

    #[test]
    fn values_are_quoted_and_escaped() {
        let values = ["a \"b\"".to_string()];
        assert_eq!(array("k", &values), "k = ['a \"b\"']\n");
        assert_eq!(array_per_line("k", &[]), "k = [\n]\n");
    }
}
