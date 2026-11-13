//! The languages that zenith reads, which a configuration may enable.

use zenith_config::Languages;

/// Every language, in the order that the docs list them.
pub const LANGUAGES: Languages = Languages::new(&[
    &zenith_lang_js::JS,
    &zenith_lang_go::GO,
    &zenith_lang_haskell::HASKELL,
    &zenith_lang_python::PYTHON,
    &zenith_lang_rust::RUST,
    &zenith_lang_terraform::TERRAFORM,
]);

#[cfg(test)]
mod tests {
    use zenith_config::Config;
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
    fn the_source_extensions_and_file_names_are_what_the_default_include_matches() {
        for language in LANGUAGES.iter() {
            let globs: Vec<_> =
                language.include.iter().map(|glob| Glob::new(glob).unwrap()).collect();
            for extension in language.source_extensions {
                let path = format!("src/a.{extension}");
                assert!(globs.iter().any(|glob| glob.is_match(&path)), "{path}");
            }
            for name in language.source_file_names {
                let path = format!("src/{name}");
                assert!(globs.iter().any(|glob| glob.is_match(&path)), "{path}");
            }
            assert!(!globs.iter().any(|glob| glob.is_match("src/a.txt")));
        }
    }

    #[test]
    fn names_only_apply_to_the_schemes_of_languages_that_record_them() {
        // Python, Go, Haskell and Terraform imports take whole modules.
        let recording: Vec<_> = LANGUAGES
            .iter()
            .filter(|language| language.records_names)
            .map(|language| language.name)
            .collect();
        assert_eq!(recording, ["js", "rust"]);
        for language in LANGUAGES.iter() {
            for scheme in language.schemes {
                let toml =
                    include_str!("fixtures/names-on-a-scheme.toml").replace("SCHEME", scheme);
                let errors = Config::parse(&toml, LANGUAGES).err().unwrap_or_default();
                let errors: Vec<_> = errors.iter().map(ToString::to_string).collect();
                match language.records_names {
                    true => assert_eq!(errors, Vec::<String>::new(), "{scheme}"),
                    false => assert_eq!(
                        errors,
                        [format!(
                            "in contract `c`: invalid selector `{scheme}:x`: `names` doesn't apply \
                             to `{scheme}:` selectors, since the `{}` language doesn't record the \
                             names that imports take",
                            language.name
                        )],
                        "{scheme}"
                    ),
                }
            }
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
