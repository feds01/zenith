mod stage;

use lang_js::parse::Parser;

pub fn run() -> &'static str {
    stage::name(&Parser)
}
