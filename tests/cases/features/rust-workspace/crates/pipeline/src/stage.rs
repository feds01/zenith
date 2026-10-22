use lang_js::parse::Parser;

pub fn name(parser: &Parser) -> &'static str {
    parser.language()
}
