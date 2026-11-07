//! The grammar of a module's header and of its imports, as GHC parses
//! them: chapter 5 of the Report, with the extensions that change them, which
//! are package names (`PackageImports`), `qualified` after the module
//! (`ImportQualifiedPost`), `safe` imports (Safe Haskell), imports of boot
//! files (`{-# SOURCE #-}`), imports at the levels of Template Haskell
//! (`ExplicitLevelImports`), signatures (Backpack), and the namespaces of the
//! names in a list (`ExplicitNamespaces`, `PatternSynonyms` and
//! `NamedDefaults`).
//!
//! Each function parses the production that its documentation gives, named
//! as GHC's grammar names it, from the lexemes of one declaration, as the
//! layout rule grouped them. A lexeme that is always written the same, such
//! as `import` or `(`, is the parser of itself.

use winnow::{
    combinator::{alt, cut_err, delimited, opt, peek, preceded, separated, terminated},
    error::{ContextError, ErrMode},
    prelude::*,
    stream::{Accumulate, ContainsToken, Location, TokenSlice},
    token::one_of,
};
use zenith_utils::location::{ByteRange, Located};

use super::{
    Expected, Import, ImportList, Level,
    lexer::{Kind, Token},
};

/// The lexemes of a declaration, as the grammar reads them.
pub(super) type Tokens<'t> = TokenSlice<'t, Token<'t>>;

/// Why the grammar could not read a declaration: the innermost context is
/// what it expected instead of what it found.
pub(super) type Error = ErrMode<ContextError<Expected>>;

/// What the grammar reads, or why it can't.
type Result<T> = std::result::Result<T, Error>;

/// A kind of lexeme is the parser of a lexeme of that kind.
impl<'t> Parser<Tokens<'t>, &'t Token<'t>, Error> for Kind {
    fn parse_next(&mut self, input: &mut Tokens<'t>) -> Result<&'t Token<'t>> {
        one_of(*self).parse_next(input)
    }
}

/// A kind of lexeme contains the lexemes of that kind.
impl ContainsToken<&Token<'_>> for Kind {
    fn contains_token(&self, token: &Token<'_>) -> bool {
        token.kind == *self
    }
}

/// `header → module modid [exports] where`, or `signature modid [exports]
/// where` in a signature of Backpack's: what names the module. Which of the
/// two starts a header in a file is for the layout rule to tell, by the
/// file's dialect.
pub(super) fn header(input: &mut Tokens<'_>) -> Result<Located<String>> {
    let rest = (opt(exports), "where".context(Expected::Token("where")));
    preceded(alt(("module", "signature")), cut_err(terminated(modid, rest))).parse_next(input)
}

/// `exports → ( export1 , … , exportn [ , ] )`
fn exports(input: &mut Tokens<'_>) -> Result<()> {
    list(export).parse_next(input)
}

/// `export → import | default qtycon`: an item of an export list, which GHC
/// parses as an item of an import list, but for the class whose defaults a
/// module may export.
fn export(input: &mut Tokens<'_>) -> Result<Located<String>> {
    alt((preceded("default", name), import)).parse_next(input)
}

/// `importdecl → import [{-# SOURCE #-}] [level] [safe] [qualified] [string]
/// modid [level] [qualified] [as modid] [impspec]`
pub(super) fn importdecl(input: &mut Tokens<'_>) -> Result<Import> {
    let start = Location::current_token_start(input);
    "import".parse_next(input)?;
    let source = flag(Kind::SourcePragma).parse_next(input)?;
    let level_before = opt(level).parse_next(input)?;
    let safe = flag("safe").parse_next(input)?;
    let qualified_before = flag("qualified").parse_next(input)?;
    let package = opt(package).parse_next(input)?;
    let module = cut_err(modid).parse_next(input)?;
    let level = opt(level).parse_next(input)?.or(level_before);
    let qualified = flag("qualified").parse_next(input)? || qualified_before;
    let alias = opt(preceded("as", cut_err(modid))).parse_next(input)?;
    let list = opt(impspec).parse_next(input)?;
    Ok(Import {
        module: module.value,
        range: ByteRange::new(start as u32, Location::previous_token_end(input) as u32),
        module_range: module.range,
        package,
        source,
        level,
        safe,
        qualified,
        alias: alias.map(|alias| alias.value),
        list,
    })
}

/// `string`: the package that a package-qualified import names, between its
/// quotes, which must look like a package's name, as GHC requires.
fn package(input: &mut Tokens<'_>) -> Result<String> {
    let name = Kind::String
        .verify_map(|token: &Token<'_>| token.unquoted().filter(|name| is_package_name(name)));
    preceded(peek(Kind::String), cut_err(name.context(Expected::PackageName))).parse_next(input)
}

/// `level → splice | quote`
fn level(input: &mut Tokens<'_>) -> Result<Level> {
    alt(("splice".value(Level::Splice), "quote".value(Level::Quote))).parse_next(input)
}

/// `impspec → ( import1 , … , importn [ , ] ) | hiding ( import1 , … ,
/// importn [ , ] )`
fn impspec(input: &mut Tokens<'_>) -> Result<ImportList> {
    alt((
        preceded("hiding", cut_err(list(import))).map(|names| ImportList { hiding: true, names }),
        list(import).map(|names| ImportList { hiding: false, names }),
    ))
    .parse_next(input)
}

/// `import → qcname_ext [subordinates] | module modid | pattern qcon | type
/// .. | data ..`: an item of an import list, or of an export list.
fn import(input: &mut Tokens<'_>) -> Result<Located<String>> {
    alt((
        preceded("module", cut_err(modid)),
        preceded("pattern", name),
        preceded(namespace, wildcard),
        terminated(qcname_ext, opt(subordinates)),
    ))
    .parse_next(input)
}

/// `qcname_ext → qcname | type oqtycon | data qvarcon`: a name, maybe in the
/// namespace of types or of data.
fn qcname_ext(input: &mut Tokens<'_>) -> Result<Located<String>> {
    alt((preceded(namespace, name), name)).parse_next(input)
}

/// `( qcname_ext_w_wildcard1 , … , qcname_ext_w_wildcardn )`: the names that
/// belong to a type or a class, after it.
fn subordinates(input: &mut Tokens<'_>) -> Result<()> {
    list(subordinate).parse_next(input)
}

/// `qcname_ext_w_wildcard → qcname_ext | [type | data] ..`: a
/// constructor, a field or a method, or `..` for all of them, or all of those
/// in a namespace.
fn subordinate(input: &mut Tokens<'_>) -> Result<()> {
    alt((preceded(opt(namespace), wildcard).void(), qcname_ext.void())).parse_next(input)
}

/// `type | data`: the namespace of types, or of data, that the names after
/// it are in.
fn namespace(input: &mut Tokens<'_>) -> Result<()> {
    alt(("type", "data")).void().parse_next(input)
}

/// `..`, which stands for every name of a type, or of a namespace, and is
/// named as it is written.
fn wildcard(input: &mut Tokens<'_>) -> Result<Located<String>> {
    Kind::Symbol
        .verify(|token: &&Token<'_>| token.text == "..")
        .map(Token::located)
        .parse_next(input)
}

/// A name, maybe qualified, or an operator in parentheses, such as `(<>)`.
///
/// `var → varid | ( varsym )`, `con → conid | ( consym )`
fn name(input: &mut Tokens<'_>) -> Result<Located<String>> {
    alt((Kind::VarId, Kind::ConId, delimited("(", Kind::Symbol, ")")))
        .map(Token::located)
        .context(Expected::Name)
        .parse_next(input)
}

/// `modid → {conid .} conid`: the name of a module, which is one lexeme.
fn modid(input: &mut Tokens<'_>) -> Result<Located<String>> {
    Kind::ConId.map(Token::located).context(Expected::ModuleName).parse_next(input)
}

/// `( item1 , … , itemn [ , ] )`: the items that `item` parses, in
/// parentheses and apart by commas, which may end with one.
fn list<'t, O, C>(item: impl Parser<Tokens<'t>, O, Error>) -> impl Parser<Tokens<'t>, C, Error>
where
    C: Accumulate<O>,
{
    let close = (opt(","), ")".context(Expected::Token(")")));
    preceded(
        "(".context(Expected::Token("(")),
        cut_err(terminated(separated(0.., item, ","), close)),
    )
}

/// Whether `name` looks like the name of a package, as GHC requires of a
/// package-qualified import's: words of letters and digits, apart by `-`, of
/// which none is digits alone.
fn is_package_name(name: &str) -> bool {
    name.split('-').all(|word| {
        !word.is_empty()
            && word.chars().all(char::is_alphanumeric)
            && !word.chars().all(|c| c.is_ascii_digit())
    })
}

/// Whether `parser` reads what comes next, which it may not.
fn flag<'t, O>(parser: impl Parser<Tokens<'t>, O, Error>) -> impl Parser<Tokens<'t>, bool, Error> {
    opt(parser).map(|found| found.is_some())
}
