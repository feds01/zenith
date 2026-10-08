//! `zenith schema`: print the JSON Schema of `zenith.toml`.

use anyhow::Result;

use crate::ExitStatus;

pub fn schema() -> Result<ExitStatus> {
    println!("{}", serde_json::to_string_pretty(&zenith_config::json_schema())?);
    Ok(ExitStatus::Success)
}
