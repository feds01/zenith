//! Tests of the Haskell parser.

use std::{ops::Range, path::Path};

use zenith_utils::location::ByteRange;

use super::*;

/// Parse `source`, plain Haskell, with a new parser.
fn parse(source: &str) -> HaskellFile {
    let file = Parser::new().parse(source, Dialect::PLAIN);
    assert_ranges_valid(source, &file);
    file
}

/// The text of `range` in `source`.
fn text(source: &str, range: ByteRange) -> &str {
    &source[Range::from(range)]
}

/// The modules of the imports of `file`.
fn modules(file: &HaskellFile) -> Vec<&str> {
    file.imports.iter().map(|import| import.module.as_str()).collect()
}

/// The names of the import list of `import`, and whether it hides them.
fn list(import: &Import) -> (bool, Vec<&str>) {
    let list = import.list.as_ref().expect("an import list");
    (list.hiding, list.names.iter().map(|name| name.value.as_str()).collect())
}

/// The text of the comments of `file`.
fn comments<'a>(source: &'a str, file: &HaskellFile) -> Vec<&'a str> {
    file.comments.iter().map(|comment| text(source, comment.range)).collect()
}

/// The errors of `file`, with the text that each is at.
fn errors<'a>(source: &'a str, file: &HaskellFile) -> Vec<(HaskellSyntaxError, &'a str)> {
    file.errors.iter().map(|error| (error.value.clone(), text(source, error.range))).collect()
}

/// Check that every range of `file` is in `source`, on character boundaries,
/// and that the imports and the comments are in source order.
fn assert_ranges_valid(source: &str, file: &HaskellFile) {
    let check = |range: ByteRange| {
        assert!(range.start() <= range.end(), "{range} is backwards");
        assert!(source.get(Range::from(range)).is_some(), "{range} is not in {source:?}");
    };
    file.module.iter().for_each(|module| check(module.range));
    file.errors.iter().for_each(|error| check(error.range));
    for import in &file.imports {
        check(import.range);
        check(import.module_range);
        assert!(import.range.start() <= import.module_range.start());
        assert!(import.module_range.end() <= import.range.end());
        let names = import.list.iter().flat_map(|list| &list.names);
        names.for_each(|name| check(name.range));
    }
    for pair in file.imports.windows(2) {
        assert!(pair[0].range.end() <= pair[1].range.start(), "imports overlap in {source:?}");
    }
    for pair in file.comments.windows(2) {
        assert!(pair[0].range.end() <= pair[1].range.start(), "comments overlap in {source:?}");
    }
    file.comments.iter().for_each(|comment| check(comment.range));
}

#[test]
fn reads_the_module_header() {
    let source = "module Acme.Api.Handlers (serve, Handler (..)) where\n";
    let file = parse(source);
    let module = file.module.as_ref().unwrap();
    assert_eq!(module.value, "Acme.Api.Handlers");
    assert_eq!(text(source, module.range), "Acme.Api.Handlers");
    assert!(file.imports.is_empty());
    assert!(file.errors.is_empty());
}

#[test]
fn a_file_without_a_header_is_read_from_its_first_lexeme() {
    let source = "import Data.List (sort)\nimport System.IO\n\nmain = print (sort [3, 1, 2])\n";
    let file = parse(source);
    assert_eq!(file.module, None);
    assert_eq!(modules(&file), ["Data.List", "System.IO"]);
    assert!(file.errors.is_empty());
}

#[test]
fn reads_a_plain_import() {
    let source = "module Main where\n\nimport Data.Map\n\nmain = pure ()\n";
    let file = parse(source);
    let [import] = &file.imports[..] else { panic!("{file:?}") };
    assert_eq!(import.module, "Data.Map");
    assert_eq!(text(source, import.range), "import Data.Map");
    assert_eq!(text(source, import.module_range), "Data.Map");
    assert_eq!(import.package, None);
    assert_eq!(import.alias, None);
    assert_eq!(import.list, None);
    assert!(!import.qualified && !import.safe && !import.source);
}

#[test]
fn reads_qualified_imports_before_and_after_the_module() {
    let source = "import qualified Data.Map as Map\nimport Data.Set qualified as Set\n";
    let file = parse(source);
    let [map, set] = &file.imports[..] else { panic!("{file:?}") };
    assert!(map.qualified && set.qualified);
    assert_eq!(map.alias.as_deref(), Some("Map"));
    assert_eq!(set.alias.as_deref(), Some("Set"));
    assert_eq!(text(source, set.range), "import Data.Set qualified as Set");
}

#[test]
fn reads_package_safe_and_source_imports() {
    let source = "import \"aeson\" Data.Aeson\n\
                  import safe Data.List\n\
                  import {-# SOURCE #-} Acme.Types (Config)\n\
                  import {-# source #-} safe qualified \"acme-core\" Acme.Core as Core\n";
    let file = parse(source);
    assert!(file.errors.is_empty(), "{:?}", file.errors);
    let [aeson, list, types, core] = &file.imports[..] else { panic!("{file:?}") };
    assert_eq!(aeson.package.as_deref(), Some("aeson"));
    assert!(list.safe && !list.source);
    assert!(types.source && !types.safe);
    assert!(core.source && core.safe && core.qualified);
    assert_eq!(core.package.as_deref(), Some("acme-core"));
    assert_eq!(core.alias.as_deref(), Some("Core"));
}

#[test]
fn reads_the_names_of_import_lists() {
    let source = "import Data.Map (Map, lookup, (!), Maybe (..), Either (Left, Right))\n\
                  import Prelude hiding (head, tail)\n\
                  import GHC.TypeLits (type (+), pattern KnownNat, data Proxy, type .., data ..)\n\
                  import Control.Applicative (type Applicative (data .., type ..))\n\
                  import Data.List ()\n\
                  import Data.Char (ord, chr,)\n";
    let file = parse(source);
    assert!(file.errors.is_empty(), "{:?}", file.errors);
    let lists: Vec<_> = file.imports.iter().map(list).collect();
    assert_eq!(
        lists,
        [
            (false, vec!["Map", "lookup", "!", "Maybe", "Either"]),
            (true, vec!["head", "tail"]),
            (false, vec!["+", "KnownNat", "Proxy", "..", ".."]),
            (false, vec!["Applicative"]),
            (false, vec![]),
            (false, vec!["ord", "chr"]),
        ]
    );
    let names = &file.imports[0].list.as_ref().unwrap().names;
    assert_eq!(text(source, names[2].range), "!");
}

#[test]
fn pattern_is_a_name_of_its_own_outside_a_namespace() {
    let source = "import Acme.Matching (pattern, pattern Match)\n";
    let file = parse(source);
    assert_eq!(list(&file.imports[0]), (false, vec!["pattern", "Match"]));
}

#[test]
fn reads_magic_hash_and_unicode_names() {
    let source = "import GHC.Exts (Int#, (+#), plusWord#)\nimport Ünicode.Modul ((∘), naïve)\n";
    let file = parse(source);
    assert!(file.errors.is_empty(), "{:?}", file.errors);
    assert_eq!(modules(&file), ["GHC.Exts", "Ünicode.Modul"]);
    assert_eq!(list(&file.imports[0]).1, ["Int#", "+#", "plusWord#"]);
    assert_eq!(list(&file.imports[1]).1, ["∘", "naïve"]);
}

#[test]
fn reads_every_item_of_an_export_list() {
    let source = "module Acme.Api\n  ( module Acme.Types\n  , Handler (.., Extra)\n  , \
                  Map.lookup\n  , (Map.!)\n  , (-->)\n  , type (++)\n  , pattern Ok\n  , default Num\n  \
                  , data ..\n  ,\n  ) where\n\nimport Acme.Types\n";
    let file = parse(source);
    assert!(file.errors.is_empty(), "{:?}", file.errors);
    assert_eq!(file.module.as_ref().unwrap().value, "Acme.Api");
    assert_eq!(modules(&file), ["Acme.Types"]);
}

#[test]
fn an_import_list_may_continue_on_the_lines_after_it() {
    let source = "module M where\n\nimport Acme.Types\n  ( Request (..)\n  , Response (..)\n  \
                  )\nimport Data.Text\n";
    let file = parse(source);
    assert_eq!(modules(&file), ["Acme.Types", "Data.Text"]);
    assert_eq!(list(&file.imports[0]).1, ["Request", "Response"]);
    assert!(text(source, file.imports[0].range).ends_with(")"));
}

#[test]
fn stops_at_the_first_declaration_after_the_imports() {
    let source = "import Data.List\n\nmain = pure ()\n\nimport Data.Map\nx = \"unterminated\n";
    let file = parse(source);
    assert_eq!(modules(&file), ["Data.List"]);
    // Nothing after the first declaration is lexed, so its errors aren't found.
    assert!(file.errors.is_empty(), "{:?}", file.errors);
}

#[test]
fn reads_a_body_in_braces() {
    let source =
        "module M where { import A ; import qualified B as C\n; ; import D; main = pure () }";
    let file = parse(source);
    assert!(file.errors.is_empty(), "{:?}", file.errors);
    assert_eq!(modules(&file), ["A", "B", "D"]);
}

#[test]
fn semicolons_end_declarations_in_a_laid_out_body() {
    let source = "import A; import B\nimport C;\n";
    let file = parse(source);
    assert_eq!(modules(&file), ["A", "B", "C"]);
}

#[test]
fn a_body_is_laid_out_at_the_column_of_its_first_lexeme() {
    let source = "module M where\n  import A\n  import B\n    (b)\nimport C\n";
    let file = parse(source);
    // `import C`, to the left of the body, ends it, as GHC would stop there.
    assert_eq!(modules(&file), ["A", "B"]);
}

#[test]
fn a_body_on_the_line_of_the_header_is_laid_out_at_its_column() {
    let source = "module M where import A\n               import B\n";
    let file = parse(source);
    assert_eq!(modules(&file), ["A", "B"]);
}

#[test]
fn tab_stops_are_8_apart() {
    let source = "module M where\n\timport A\n        import B\n\t  (b)\n";
    let file = parse(source);
    assert!(file.errors.is_empty(), "{:?}", file.errors);
    assert_eq!(modules(&file), ["A", "B"]);
    assert_eq!(list(&file.imports[1]).1, ["b"]);
}

#[test]
fn a_lexeme_after_a_comment_is_at_its_own_column() {
    // `import B` starts the line but for the comment, so it is at column 8 and
    // continues `import A`, as the layout rule has it.
    let source = "import A\n{- c -} import B\n";
    let file = parse(source);
    assert!(file.imports.is_empty());
    assert_eq!(
        errors(source, &file),
        [(HaskellSyntaxError::Unexpected("import".into()), "import")]
    );
}

#[test]
fn keeps_the_comments_up_to_the_end_of_the_imports() {
    let source = "-- | The API.\n{- A {- nested -} comment -}\nmodule M where\n\n\
                  -- zenith-ignore: legacy\nimport A -- trailing\nimport B {- block -} -- last\n\
                  -- after\n\nmain = pure () -- in the body\n";
    let file = parse(source);
    assert_eq!(
        comments(source, &file),
        [
            "-- | The API.",
            "{- A {- nested -} comment -}",
            "-- zenith-ignore: legacy",
            "-- trailing",
            "{- block -}",
            "-- last",
        ]
    );
    assert!(file.comments[1].block && !file.comments[0].block);
}

#[test]
fn dashes_start_a_comment_only_as_a_lexeme_of_their_own() {
    let source =
        "module M ((-->), (|--)) where\nimport A --- a comment\nimport B --| an operator\n";
    let file = parse(source);
    assert_eq!(modules(&file), ["A"]);
    assert_eq!(comments(source, &file), ["--- a comment"]);
    assert_eq!(errors(source, &file), [(HaskellSyntaxError::Unexpected("--|".into()), "--|")]);
}

#[test]
fn skips_pragmas_but_source() {
    let source = "{-# LANGUAGE OverloadedStrings #-}\n{-# OPTIONS_GHC -Wall #-}\n\
                  module M {-# DEPRECATED \"use N\" #-}\n  ( {-# WARNING \"old\" #-} f\n  ) \
                  where\n\nimport A\n";
    let file = parse(source);
    assert!(file.errors.is_empty(), "{:?}", file.errors);
    assert_eq!(file.module.as_ref().unwrap().value, "M");
    assert_eq!(modules(&file), ["A"]);
    // Pragmas are not comments, so no suppression is read from them.
    assert!(file.comments.is_empty());
}

#[test]
fn reads_the_imports_of_every_branch_of_the_preprocessor() {
    let source = "#!/usr/bin/env runghc\n{-# LANGUAGE CPP #-}\nmodule M where\n\n\
                  #include \"config.h\"\n#if defined(mingw32_HOST_OS) && \\\n    \
                  defined(USE_WIN32)\nimport System.Win32\n#else\nimport System.Posix\n  #endif\n\
                  import Data.List\n";
    let file = parse(source);
    assert!(file.errors.is_empty(), "{:?}", file.errors);
    assert_eq!(modules(&file), ["System.Win32", "System.Posix", "Data.List"]);
}

#[test]
fn reads_a_header_that_each_branch_writes_its_own_way_once() {
    let source = "{-# LANGUAGE CPP #-}\n#ifdef STRICT\nmodule Data.Strict\n#else\nmodule \
                  Data.Lazy\n#endif\n    ( readInt\n#if WITH_WORDS\n    , readWord\n#endif\n    ) \
                  where\n\nimport Data.Int\n";
    let file = parse(source);
    assert!(file.errors.is_empty(), "{:?}", file.errors);
    // The first branch is read, as the others are its alternatives.
    assert_eq!(file.module.as_ref().unwrap().value, "Data.Strict");
    assert_eq!(modules(&file), ["Data.Int"]);
}

#[test]
fn skips_the_headers_of_other_branches_but_reads_their_imports() {
    let source = "#if defined(SHORT)\nmodule Properties.Short (tests) where\nimport \
                  Data.ByteString.Short\n#elif !defined(LAZY)\nmodule Properties.Strict\n  \
                  (tests) where\nimport Data.ByteString\n#else\nmodule Properties.Lazy (tests) \
                  where\n#endif\nimport Data.Char\n\ntests = []\n";
    let file = parse(source);
    assert!(file.errors.is_empty(), "{:?}", file.errors);
    assert_eq!(file.module.as_ref().unwrap().value, "Properties.Short");
    assert_eq!(modules(&file), ["Data.ByteString.Short", "Data.ByteString", "Data.Char"]);
}

#[test]
fn skips_the_rest_of_a_header_that_each_branch_ends_its_own_way() {
    let source = "module GHC.Windows\n#if defined(javascript_HOST_ARCH)\n    ( ) where\n\n#else\n    \
                  ( BOOL\n    , HANDLE\n    ) where\n\nimport GHC.Internal.Windows\n#if C\nimport \
                  C1\n#else\nimport C2\n#endif\n#endif\nimport Data.Word\n";
    let file = parse(source);
    assert!(file.errors.is_empty(), "{:?}", file.errors);
    assert_eq!(file.module.as_ref().unwrap().value, "GHC.Windows");
    assert_eq!(modules(&file), ["GHC.Internal.Windows", "C1", "C2", "Data.Word"]);
}

#[test]
fn a_branch_without_the_rest_of_the_header_ends_with_its_conditional() {
    let source = "module Platform\n#if defined(mingw32_HOST_OS)\n  (platform) where\nimport \
                  Windows.Stuff\n#elif defined(linux_HOST_OS)\n  (platform) where\nimport \
                  Linux.Stuff\n#else\n# error \"unsupported\"\n#endif\nimport Data.Text\n\nplatform \
                  = x\n  where x = 1\n";
    let file = parse(source);
    assert!(file.errors.is_empty(), "{:?}", file.errors);
    assert_eq!(modules(&file), ["Windows.Stuff", "Linux.Stuff", "Data.Text"]);
}

#[test]
fn reads_an_import_from_the_branch_that_it_starts_in() {
    let source = "import Data.Map\n#if MIN_VERSION_containers(0,6,0)\n  (Map, lookup)\n#else\n  \
                  (Map)\n#endif\n#if X\nimport A\n#else\nimport B\n  ( b\n#if Y\n  , c\n#endif\n  \
                  )\n#endif\n";
    let file = parse(source);
    assert!(file.errors.is_empty(), "{:?}", file.errors);
    assert_eq!(modules(&file), ["Data.Map", "A", "B"]);
    assert_eq!(list(&file.imports[0]).1, ["Map", "lookup"]);
    assert_eq!(list(&file.imports[2]).1, ["b", "c"]);
}

#[test]
fn skips_what_if_0_holds() {
    let source = "import A\n#if 0 /* old */\nimport Old -- zenith-ignore: never read\n#elif X\nimport \
                  B\n#else\nimport C\n#endif\n#if 0\n#if X\nimport Older\n#endif\n#endif\n";
    let file = parse(source);
    assert_eq!(modules(&file), ["A", "B", "C"]);
    assert!(file.comments.is_empty());
}

#[test]
fn reads_a_signature() {
    let source = "signature Acme.Store (Store, get) where\n\nimport Data.Map (Map)\n";
    let file = Parser::new().parse(source, Dialect::of(Path::new("src/Acme/Store.hsig")));
    assert!(file.errors.is_empty(), "{:?}", file.errors);
    assert_eq!(file.module.as_ref().unwrap().value, "Acme.Store");
    assert_eq!(modules(&file), ["Data.Map"]);
}

#[test]
fn signature_starts_a_header_only_in_a_signature() {
    let source = "signature :: Int\nsignature = 1\n";
    let file = parse(source);
    assert!(file.errors.is_empty(), "{:?}", file.errors);
    assert_eq!(file.module, None);
}

#[test]
fn reads_the_levels_of_template_haskell() {
    let source = "import splice Language.Haskell.TH\nimport Acme.Quotes quote qualified as Q\n\
                  import Data.List\n";
    let file = parse(source);
    assert!(file.errors.is_empty(), "{:?}", file.errors);
    let levels: Vec<_> = file.imports.iter().map(|import| import.level).collect();
    assert_eq!(levels, [Some(Level::Splice), Some(Level::Quote), None]);
    assert!(file.imports[1].qualified);
}

#[test]
fn reads_crlf_line_endings_and_a_byte_order_mark() {
    let source = "\u{FEFF}module M where\r\n\r\nimport A -- a comment\r\nimport B\r\n";
    let file = parse(source);
    assert_eq!(modules(&file), ["A", "B"]);
    assert_eq!(comments(source, &file), ["-- a comment"]);
}

#[test]
fn a_byte_order_mark_takes_no_column() {
    let source = "\u{FEFF}import A\nimport B\n";
    assert_eq!(modules(&parse(source)), ["A", "B"]);
    let source = "\u{FEFF}#!/usr/bin/env runghc\nimport A\n";
    assert_eq!(modules(&parse(source)), ["A"]);
}

/// Parse `source`, literate Haskell, with a new parser.
fn parse_literate(source: &str) -> HaskellFile {
    let file = Parser::new().parse(source, Dialect::LITERATE);
    assert_ranges_valid(source, &file);
    file
}

#[test]
fn reads_the_bird_tracks_of_a_literate_file() {
    let source = "The handlers of the API, which import nothing else:\n\n\
                  > module Acme.Api where\n>\n> import Data.Map (Map)\n\n\
                  Some text, which does not import Not.This.\n\n> import Data.Tëxt\n";
    let file = parse_literate(source);
    assert!(file.errors.is_empty(), "{:?}", file.errors);
    assert_eq!(file.module.as_ref().unwrap().value, "Acme.Api");
    assert_eq!(modules(&file), ["Data.Map", "Data.Tëxt"]);
    // The ranges are those of the literate source.
    assert_eq!(text(source, file.imports[0].module_range), "Data.Map");
}

#[test]
fn reads_the_code_blocks_of_a_literate_file() {
    let source = "\\section{The API}\n\n\\begin{code}\nmodule Acme.Api where\n\nimport \
                  Data.Map\n\\end{code}\n\nSome text, which does not import \
                  Not.This.\n\n\\begin{code}\nimport Data.Text\n\\end{code}\n";
    let file = parse_literate(source);
    assert!(file.errors.is_empty(), "{:?}", file.errors);
    assert_eq!(modules(&file), ["Data.Map", "Data.Text"]);
    assert_eq!(text(source, file.imports[1].range), "import Data.Text");
}

#[test]
fn keeps_the_lines_of_the_preprocessor_in_a_literate_file() {
    let source = "> import Data.Map\n#if MIN_VERSION_containers(0,6,0)\n>   (Map, lookup)\n#else\n>   \
                  (Map)\n#endif\n> import Data.Text\n";
    let file = parse_literate(source);
    assert!(file.errors.is_empty(), "{:?}", file.errors);
    assert_eq!(modules(&file), ["Data.Map", "Data.Text"]);
    assert_eq!(list(&file.imports[0]).1, ["Map", "lookup"]);
}

#[test]
fn a_literate_parser_is_reused_between_files() {
    let mut parser = Parser::new();
    let long = parser.parse("> import Data.Map\n> import Data.Set\n", Dialect::LITERATE);
    let short = parser.parse("> import A\n", Dialect::LITERATE);
    assert_eq!(modules(&long), ["Data.Map", "Data.Set"]);
    assert_eq!(modules(&short), ["A"]);
}

#[test]
fn dialects_are_told_by_their_extension() {
    let signature = Dialect { literate: false, signature: true };
    let literate_signature = Dialect { literate: true, signature: true };
    for (path, dialect) in [
        ("src/Acme/Api.hs", Dialect::PLAIN),
        ("src/Acme/Api.hs-boot", Dialect::PLAIN),
        ("src/Acme/Api.hsc", Dialect::PLAIN),
        ("src/Acme/Api.lhs", Dialect::LITERATE),
        ("src/Acme/Api.lhs-boot", Dialect::LITERATE),
        ("src/Acme/Store.hsig", signature),
        ("src/Acme/Store.lhsig", literate_signature),
        ("Setup", Dialect::PLAIN),
    ] {
        assert_eq!(Dialect::of(Path::new(path)), dialect, "{path}");
    }
}

#[test]
fn reports_what_the_grammar_expected() {
    let cases = [
        ("import\n", HaskellSyntaxError::Expected(Expected::ModuleName), ""),
        ("import data.map\n", HaskellSyntaxError::Expected(Expected::ModuleName), "data"),
        ("import M as\n", HaskellSyntaxError::Expected(Expected::ModuleName), ""),
        ("import M (a b)\n", HaskellSyntaxError::Expected(Expected::Token(")")), "b"),
        ("import M (type)\n", HaskellSyntaxError::Expected(Expected::Token(")")), "type"),
        ("import M hiding x\n", HaskellSyntaxError::Expected(Expected::Token("(")), "x"),
        ("import M (a) b\n", HaskellSyntaxError::Unexpected("b".into()), "b"),
        ("import M (default C)\n", HaskellSyntaxError::Expected(Expected::Token(")")), "default"),
        ("module M (f)\nf = 1\n", HaskellSyntaxError::Expected(Expected::Token("where")), "f"),
    ];
    for (source, error, at) in cases {
        let file = parse(source);
        assert!(file.imports.is_empty(), "{source:?}");
        assert_eq!(errors(source, &file), [(error, at)], "{source:?}");
    }
}

#[test]
fn an_error_at_the_end_of_a_declaration_is_where_it_ends() {
    let source = "import qualified\nimport B\n";
    let file = parse(source);
    let [error] = &file.errors[..] else { panic!("{file:?}") };
    assert_eq!(error.range, ByteRange::new(16, 16));
    assert_eq!(modules(&file), ["B"]);
}

#[test]
fn reads_the_imports_around_an_error() {
    let source = "module M where\nimport A\nimport B (\nimport C\n";
    let file = parse(source);
    assert_eq!(modules(&file), ["A", "C"]);
    assert_eq!(errors(source, &file), [(HaskellSyntaxError::Expected(Expected::Token(")")), "")]);
}

#[test]
fn a_header_without_where_ends_before_the_imports() {
    let source = "module M (f)\n\nimport A\nimport B\n";
    let file = parse(source);
    assert_eq!(file.module, None);
    assert_eq!(modules(&file), ["A", "B"]);
    let expected = HaskellSyntaxError::Expected(Expected::Token("where"));
    assert_eq!(errors(source, &file), [(expected, "")]);
}

#[test]
fn reports_text_that_is_no_lexeme() {
    let source = "import \"acme Acme.Core\nimport A\nimport B\u{7}\n";
    let file = parse(source);
    assert_eq!(modules(&file), ["A"]);
    assert_eq!(
        errors(source, &file),
        [
            (HaskellSyntaxError::UnterminatedString, "\"acme Acme.Core"),
            (HaskellSyntaxError::InvalidCharacter, "\u{7}"),
        ]
    );
}

#[test]
fn reports_an_unterminated_comment_after_the_imports() {
    let source = "import A\n{- import B\nimport C\n";
    let file = parse(source);
    assert_eq!(modules(&file), ["A"]);
    assert_eq!(
        errors(source, &file),
        [(HaskellSyntaxError::UnterminatedComment, "{- import B\nimport C\n")]
    );
}

#[test]
fn deeply_nested_comments_do_not_overflow_the_stack() {
    let source = "{-".repeat(100_000);
    let file = parse(&source);
    assert_eq!(file.errors[0].value, HaskellSyntaxError::UnterminatedComment);
}

#[test]
fn package_names_look_like_the_names_of_packages() {
    let source = "import \"ac\\\"me\" A\nimport \"acme-\\\n  \\core\" B\nimport \"\" C\nimport \
                  \"acme-2\" D\nimport \"base64-bytestring\" E\nimport \"b\\x61s\\&\\101\" F\n";
    let file = parse(source);
    assert_eq!(modules(&file), ["B", "E", "F"]);
    // The name is what the string's escapes and gaps stand for.
    let packages: Vec<_> = file.imports.iter().map(|import| import.package.as_deref()).collect();
    assert_eq!(packages, [Some("acme-core"), Some("base64-bytestring"), Some("base")]);
    let expected = HaskellSyntaxError::Expected(Expected::PackageName);
    assert_eq!(
        errors(source, &file),
        [(expected.clone(), "\"ac\\\"me\""), (expected.clone(), "\"\""), (expected, "\"acme-2\"")]
    );
}

#[test]
fn a_reserved_word_is_never_qualified() {
    // `M.where` is `M`, `.` and `where`, so the header has no export list.
    let source = "module Main.where\n";
    let file = parse(source);
    assert_eq!(
        errors(source, &file),
        [(HaskellSyntaxError::Expected(Expected::Token("where")), ".")]
    );
}

#[test]
fn never_panics_on_a_prefix_of_a_source() {
    let source = "\u{FEFF}{-# LANGUAGE CPP #-}\n-- | Docs.\nmodule Acme.Ünicode\n  ( (∘)\n  , \
                  T (.., C)\n  ) where\n#if X\nimport {-# SOURCE #-} safe qualified \"pkg\" \
                  A.B qualified as C hiding (type (+), pattern P)\n#endif\nimport D; import \
                  E {- {- -} -}\n  (f, (Map.!), g#)\nx = 'a' : \"s\\\"\\  \\t\"\n";
    let mut parser = Parser::new();
    for (end, _) in source.char_indices() {
        let prefix = &source[..end];
        for dialect in [Dialect::PLAIN, Dialect::LITERATE] {
            assert_ranges_valid(prefix, &parser.parse(prefix, dialect));
        }
    }
}

#[test]
fn every_error_has_a_message() {
    let errors = [
        HaskellSyntaxError::SourceTooLarge,
        HaskellSyntaxError::UnterminatedComment,
        HaskellSyntaxError::UnterminatedString,
        HaskellSyntaxError::InvalidCharacter,
        HaskellSyntaxError::Expected(Expected::ModuleName),
        HaskellSyntaxError::Expected(Expected::PackageName),
        HaskellSyntaxError::Expected(Expected::Name),
        HaskellSyntaxError::Expected(Expected::Token("where")),
        HaskellSyntaxError::Unexpected("import".into()),
        HaskellSyntaxError::UnexpectedEnd,
    ];
    let messages: Vec<_> = errors.iter().map(ToString::to_string).collect();
    assert_eq!(
        messages,
        [
            "the source is too large to read",
            "unterminated block comment",
            "unterminated string",
            "invalid character",
            "expected a module name",
            "expected a package name, of letters, digits and `-`",
            "expected a name, or an operator in parentheses",
            "expected `where`",
            "unexpected `import`",
            "unexpected end of the declaration",
        ]
    );
}
