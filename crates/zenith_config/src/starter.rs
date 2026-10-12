//! The starter configuration file that `zenith init` writes.

use crate::Config;

/// Where the JSON Schema of the configuration file is published, which the
/// `#:schema` comment points editors at.
pub const SCHEMA_URL: &str =
    "https://raw.githubusercontent.com/feds01/zenith/main/zenith.schema.json";

/// The fewest workspace packages that the starter keeps free of cycles.
const MIN_PACKAGES: usize = 2;

impl Config {
    /// A starter configuration file for a project whose workspace packages,
    /// other than the root package, are `packages`, as `(name, directory)`
    /// pairs.
    ///
    /// With two or more packages, its one contract keeps them free of
    /// dependency cycles. Otherwise, it has an example contract, commented
    /// out.
    pub fn starter<'p>(packages: impl IntoIterator<Item = (&'p str, &'p str)>) -> String {
        let packages: Vec<_> = packages.into_iter().collect();
        let mut starter = format!(
            "#:schema {SCHEMA_URL}
#
# The architecture contracts of this project, which `zenith check` checks.

[analysis]
# Uncomment to leave type-only imports, which are erased at runtime, out of
# every contract.
# exclude_kinds = [\"type\"]
"
        );
        if packages.len() >= MIN_PACKAGES {
            starter += "\n# The workspace packages:\n";
            for (name, dir) in &packages {
                starter += &format!("#   {name} ({dir})\n");
            }
            starter += "
# The workspace packages must not depend on each other in cycles.
[contracts.packages-acyclic]
type = \"acyclic\"
modules = [\"pkg:*\"]
";
        } else {
            starter += "
# For example, nothing under `src/lib` may import `src/app`:
# [contracts.lib-below-app]
# type = \"forbidden\"
# from = [\"src/lib\"]
# to = [\"src/app\"]
";
        }
        starter
    }
}

#[cfg(test)]
mod tests {
    use crate::{Config, ContractKind};

    #[test]
    fn a_starter_keeps_two_or_more_packages_free_of_cycles() {
        let starter = Config::starter([("@acme/api", "packages/api"), ("@acme/web", "apps/web")]);
        assert!(starter.starts_with("#:schema https://"));
        assert!(starter.contains("#   @acme/api (packages/api)\n#   @acme/web (apps/web)\n"));

        let config = Config::parse(&starter).unwrap();
        let [contract] = config.contracts.as_slice() else {
            panic!("expected one contract, got {:?}", config.contracts);
        };
        assert_eq!(contract.id, "packages-acyclic");
        assert!(matches!(contract.kind, ContractKind::Acyclic(_)));
    }

    #[test]
    fn a_starter_without_packages_has_an_example_contract() {
        for packages in [&[][..], &[("@acme/api", "packages/api")]] {
            let starter = Config::starter(packages.iter().copied());
            assert!(starter.contains("# [contracts.lib-below-app]"));
            assert!(Config::parse(&starter).unwrap().contracts.is_empty());

            // Uncommented, the example is valid.
            let uncommented = starter.replace("# [contracts", "[contracts");
            let uncommented = uncommented.replace("# type", "type");
            let uncommented = uncommented.replace("# from", "from").replace("# to", "to");
            assert_eq!(Config::parse(&uncommented).unwrap().contracts.len(), 1);
        }
    }
}
