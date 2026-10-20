//! `zenith schema`.

use std::io::{self, Write};

use anyhow::Result;
use zenith_pipeline::LANGUAGES;

use crate::ExitStatus;

/// Print the JSON Schema of `zenith.toml`.
pub fn schema() -> Result<ExitStatus> {
    io::stdout().lock().write_all(render()?.as_bytes())?;
    Ok(ExitStatus::Success)
}

/// The JSON Schema of `zenith.toml`, pretty-printed, as committed in
/// `zenith.schema.json`.
fn render() -> serde_json::Result<String> {
    Ok(serde_json::to_string_pretty(&zenith_config::json_schema(LANGUAGES))? + "\n")
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use super::*;

    #[test]
    fn the_committed_schema_is_up_to_date() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../zenith.schema.json");
        // Git may check the file out with Windows line endings.
        let committed = fs::read_to_string(path).unwrap_or_default().replace("\r\n", "\n");
        assert!(
            committed == render().unwrap(),
            "`zenith.schema.json` is out of date; run `just schema` to regenerate it"
        );
    }
}
