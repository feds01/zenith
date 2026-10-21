//! The languages that zenith reads, which a configuration may enable.

use zenith_config::Languages;

/// Every language, in the order that the docs list them.
pub const LANGUAGES: Languages = Languages::new(&[&zenith_lang_js::JS]);

#[cfg(test)]
mod tests {
    use zenith_utils::glob::Glob;

    use super::*;

    #[test]
    fn every_language_is_named_by_its_first_table_name() {
        for language in LANGUAGES.iter() {
            assert_eq!(language.table_names.first(), Some(&language.name));
            for name in language.table_names {
                assert_eq!(LANGUAGES.by_table_name(name), Some(language), "{name}");
            }
        }
    }

    #[test]
    fn table_names_belong_to_one_language() {
        let mut names: Vec<_> = LANGUAGES.table_names().collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count);
    }

    #[test]
    fn schemes_belong_to_one_owner() {
        let mut schemes: Vec<_> = LANGUAGES.schemes().collect();
        let count = schemes.len();
        schemes.sort_unstable();
        schemes.dedup();
        assert_eq!(schemes.len(), count);
        assert_eq!(LANGUAGES.by_scheme("pkg"), None);
    }

    #[test]
    fn the_source_extensions_are_what_the_default_include_matches() {
        for language in LANGUAGES.iter() {
            let globs: Vec<_> =
                language.include.iter().map(|glob| Glob::new(glob).unwrap()).collect();
            for extension in language.source_extensions {
                let path = format!("src/a.{extension}");
                assert!(globs.iter().any(|glob| glob.is_match(&path)), "{path}");
            }
            assert!(!globs.iter().any(|glob| glob.is_match("src/a.txt")));
        }
    }

    #[test]
    fn selectors_may_leave_off_every_source_extension() {
        for language in LANGUAGES.iter() {
            for extension in language.source_extensions {
                assert!(language.optional_extensions.contains(extension), "{extension}");
            }
        }
    }
}
