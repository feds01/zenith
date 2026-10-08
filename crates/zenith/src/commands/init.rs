//! `zenith init`: write a starter `zenith.toml`.

use std::fs;

use anyhow::{Result, bail};
use zenith_config::CONFIG_FILE_NAME;
use zenith_utils::path::adjust_canonicalisation;
use zenith_workspace::{WalkSettings, discover_packages, walk};

use crate::{
    ExitStatus,
    cli::{GlobalArgs, InitCommand},
};

pub fn init(global: &GlobalArgs, args: InitCommand) -> Result<ExitStatus> {
    let root = match &global.root {
        Some(root) => adjust_canonicalisation(root),
        None => adjust_canonicalisation(std::env::current_dir()?),
    };
    let path = root.join(CONFIG_FILE_NAME);
    if path.exists() && !args.force {
        bail!("`{}` already exists; use `--force` to overwrite it", path.display());
    }

    let settings = WalkSettings {
        include: None,
        exclude: Vec::new(),
        respect_gitignore: true,
        source_extensions: zenith_parse::SOURCE_EXTENSIONS,
    };
    let files = walk(&root, &settings);
    let workspace = discover_packages(&root, &files.package_jsons);

    let mut config = String::from(
        "#:schema https://raw.githubusercontent.com/feds01/zenith/main/zenith.schema.json\n\
         #\n\
         # Zenith architecture contracts: https://github.com/feds01/zenith\n\n\
         [analysis]\n\
         # Uncomment to ignore type-only imports, which are erased at runtime.\n\
         # exclude_kinds = [\"type\"]\n",
    );

    if workspace.packages.len() >= 2 {
        let names: Vec<String> =
            workspace.packages.iter().map(|p| format!("#   {} ({})", p.name, p.dir)).collect();
        config.push_str(&format!(
            "\n# Workspace packages:\n{}\n\n\
             # No dependency cycles between workspace packages.\n\
             [contracts.packages-acyclic]\n\
             type = \"acyclic\"\n\
             modules = [\"pkg:*\"]\n",
            names.join("\n")
        ));
    } else {
        config.push_str(
            "\n# An example: nothing under `src/lib` may import `src/app`.\n\
             # [contracts.lib-is-standalone]\n\
             # type = \"forbidden\"\n\
             # from = [\"src/lib\"]\n\
             # to = [\"src/app\"]\n",
        );
    }

    fs::write(&path, config)?;
    println!(
        "Wrote `{}` ({} source files, {} workspace packages found).",
        path.display(),
        files.source_files.len(),
        workspace.packages.len()
    );
    Ok(ExitStatus::Success)
}
