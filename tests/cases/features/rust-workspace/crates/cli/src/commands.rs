use std::fs;

use lang_js::parse::Parser;

use crate::Config;

pub fn run(config: Config) {
    let _ = fs::read_to_string("zenith.toml");
    let _ = (Parser.language(), pipeline::run(), config.language);
}
