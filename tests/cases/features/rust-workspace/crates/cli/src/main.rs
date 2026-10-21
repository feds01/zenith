mod commands;

use serde::Serialize;

#[derive(Serialize)]
pub struct Config {
    pub language: &'static str,
}

fn main() {
    // The language, without going through the pipeline.
    let language = lang_js::JS;
    commands::run(Config { language });
}
