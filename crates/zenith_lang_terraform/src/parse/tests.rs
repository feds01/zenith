//! Tests of the Terraform and Terragrunt reference parser.

use std::ops::Range;

use zenith_utils::location::ByteRange;

use super::*;

/// Parse `source` as a Terraform file with a new parser.
fn parse(source: &str) -> TerraformFile {
    Parser::new().parse(source, Dialect::Terraform)
}

/// Parse `source` as a Terragrunt file with a new parser.
fn parse_terragrunt(source: &str) -> TerraformFile {
    Parser::new().parse(source, Dialect::Terragrunt)
}

/// The text of `range` in `source`.
fn text(source: &str, range: ByteRange) -> &str {
    &source[Range::from(range)]
}

/// The values of the references of `file`.
fn values(file: &TerraformFile) -> Vec<&str> {
    file.references.iter().map(|reference| reference.value.as_str()).collect()
}

/// The references of `file`, as (kind, value) pairs.
fn kinds(file: &TerraformFile) -> Vec<(ReferenceKind, &str)> {
    file.references.iter().map(|reference| (reference.kind, reference.value.as_str())).collect()
}

/// The text of the values of `file` that are not literals.
fn non_literal<'a>(source: &'a str, file: &TerraformFile) -> Vec<&'a str> {
    file.non_literal.iter().map(|value| text(source, value.range)).collect()
}

/// The text of the comments of `file`.
fn comments<'a>(source: &'a str, file: &TerraformFile) -> Vec<&'a str> {
    file.comments.iter().map(|comment| text(source, comment.range)).collect()
}

/// The errors of `file`, with the text that each is at.
fn errors<'a>(source: &'a str, file: &TerraformFile) -> Vec<(TerraformSyntaxError, &'a str)> {
    file.errors.iter().map(|error| (error.value.clone(), text(source, error.range))).collect()
}

/// Check that every range of `file` is in `source`, on character boundaries,
/// and that the comments and the errors are in source order.
fn assert_ranges_valid(source: &str, file: &TerraformFile) {
    let check = |range: ByteRange| {
        assert!(range.start() <= range.end(), "{range} is backwards");
        assert!(source.get(Range::from(range)).is_some(), "{range} is not in {source:?}");
    };
    for reference in &file.references {
        check(reference.range);
        check(reference.attribute);
        check(reference.block);
        assert!(reference.block.start() <= reference.attribute.start());
        assert!(reference.attribute.start() <= reference.range.start());
        assert!(reference.range.end() <= reference.attribute.end());
        assert!(reference.attribute.end() <= reference.block.end());
    }
    file.non_literal.iter().for_each(|value| check(value.range));
    file.comments.iter().for_each(|comment| check(comment.range));
    file.errors.iter().for_each(|error| check(error.range));
    for pair in file.comments.windows(2) {
        assert!(pair[0].range.end() <= pair[1].range.start(), "comments overlap in {source:?}");
    }
    for pair in file.errors.windows(2) {
        assert!(pair[0].range.start() <= pair[1].range.start(), "errors unsorted in {source:?}");
    }
}

#[test]
fn reads_the_source_of_a_module() {
    let source = "module \"vpc\" {\n  source = \"../modules/vpc\"\n}\n";
    let file = parse(source);
    let [reference] = &file.references[..] else {
        panic!("expected one reference: {file:?}");
    };
    assert_eq!(reference.kind, ReferenceKind::Module);
    assert_eq!(reference.value, "../modules/vpc");
    assert_eq!(text(source, reference.range), "\"../modules/vpc\"");
    assert_eq!(text(source, reference.attribute), "source = \"../modules/vpc\"");
    assert_eq!(text(source, reference.block), source.trim_end());
    assert!(file.errors.is_empty() && file.non_literal.is_empty());
}

#[test]
fn reads_every_module_in_source_order() {
    let source = "module \"a\" { source = \"./a\" }\n\
                  resource \"aws_s3_bucket\" \"b\" {}\n\
                  module \"c\" {\n  version = \"1.0.0\"\n  source  = \"acme/c/aws\"\n  count = 2\n}\n\
                  module d { source = \"git::https://example.com/d.git?ref=v1\" }\n";
    assert_eq!(
        values(&parse(source)),
        ["./a", "acme/c/aws", "git::https://example.com/d.git?ref=v1"]
    );
}

#[test]
fn reads_sources_only_from_module_blocks() {
    // A provider's source, and the attributes of other blocks, name no module.
    let source = r#"
terraform {
  required_providers {
    aws = { source = "hashicorp/aws", version = "~> 5.0" }
  }
}

data "http" "a" {
  source = "url"
}

locals {
  source = "../modules/x"
}

source = "../modules/y"

module "a" {
  sources = "../modules/z"
}
"#;
    let file = parse(source);
    assert!(file.references.is_empty(), "{:?}", file.references);
    assert!(file.non_literal.is_empty() && file.errors.is_empty());
}

#[test]
fn reads_only_top_level_modules() {
    // Terraform has no nested `module` blocks, and nor do the blocks around them
    // count.
    let source = "x {\n  module \"a\" { source = \"./a\" }\n}\n\
                  module \"b\" {\n  module \"c\" { source = \"./c\" }\n  source = \"./b\"\n}\n";
    assert_eq!(values(&parse(source)), ["./b"]);
}

#[test]
fn reads_the_blocks_of_its_dialect() {
    let source = "module \"a\" { source = \"./a\" }\n\
                  terraform { source = \"./b\" }\n\
                  dependency \"c\" { config_path = \"../c\" }\n";
    assert_eq!(kinds(&parse(source)), [(ReferenceKind::Module, "./a")]);
    assert_eq!(
        kinds(&parse_terragrunt(source)),
        [(ReferenceKind::TerragruntSource, "./b"), (ReferenceKind::Dependency, "../c")]
    );
}

#[test]
fn reads_terragrunt_sources_and_dependencies() {
    let source = r#"
include "root" {
  path = find_in_parent_folders("root.hcl")
}

terraform {
  source = "git::git@github.com:acme/modules.git//vpc?ref=v1.2.0"

  extra_arguments "retry" {
    commands = ["apply"]
    source   = "not a reference"
  }
}

dependency "network" {
  config_path = "../network"

  mock_outputs = {
    vpc_id = "vpc-1"
  }
}

dependencies {
  paths = ["../dns", "../iam",]
}
"#;
    let file = parse_terragrunt(source);
    assert!(file.errors.is_empty(), "{:?}", file.errors);
    assert_eq!(
        kinds(&file),
        [
            (
                ReferenceKind::TerragruntSource,
                "git::git@github.com:acme/modules.git//vpc?ref=v1.2.0"
            ),
            (ReferenceKind::Dependency, "../network"),
            (ReferenceKind::Dependency, "../dns"),
            (ReferenceKind::Dependency, "../iam"),
        ]
    );
    // The paths of `dependencies` are each in the one attribute of the one block.
    let [dns, iam] = &file.references[2..] else {
        panic!("expected two paths: {:?}", file.references);
    };
    assert_eq!((dns.attribute, dns.block), (iam.attribute, iam.block));
    assert_eq!(text(source, dns.attribute), "paths = [\"../dns\", \"../iam\",]");
    assert!(text(source, dns.block).starts_with("dependencies {"));
    // The `include` path is a function call, but it is not a reference.
    assert!(file.non_literal.is_empty());
}

#[test]
fn locates_values_that_are_not_literals() {
    let source = r#"
module "a" { source = "${path.module}/a" }
module "b" { source = local.b }
module "c" {
  source = <<EOT
./c
EOT
}
module "d" { source = "./d"[0] }
module "e" { source = ("./e") }
module "f" { source = 5 }
"#;
    let file = parse(source);
    assert!(file.references.is_empty(), "{:?}", file.references);
    assert_eq!(
        non_literal(source, &file),
        ["\"${path.module}/a\"", "local.b", "<<EOT\n./c\nEOT", "\"./d\"[0]", "(\"./e\")", "5"]
    );
    assert!(file.non_literal.iter().all(|value| value.value == ReferenceKind::Module));
}

#[test]
fn locates_terragrunt_values_that_are_not_literals() {
    let source = r#"
terraform {
  source = "${get_repo_root()}/modules/vpc"
}
dependency "a" {
  config_path = "${get_terragrunt_dir()}/../a"
}
dependencies {
  paths = concat(["../b"], local.more)
}
dependencies {
  paths = ["../c", local.d]
}
"#;
    let file = parse_terragrunt(source);
    assert_eq!(values(&file), ["../c"]);
    assert_eq!(
        non_literal(source, &file),
        [
            "\"${get_repo_root()}/modules/vpc\"",
            "\"${get_terragrunt_dir()}/../a\"",
            "concat([\"../b\"], local.more)",
            "local.d",
        ]
    );
    let kinds: Vec<_> = file.non_literal.iter().map(|value| value.value).collect();
    assert_eq!(
        kinds,
        [
            ReferenceKind::TerragruntSource,
            ReferenceKind::Dependency,
            ReferenceKind::Dependency,
            ReferenceKind::Dependency
        ]
    );
}

#[test]
fn reports_escaped_references() {
    for value in [r#""a\"b""#, r#""a\\b""#, "\"\\u0041\"", r#""$${a}""#, r#""%%{a}""#] {
        let source = format!("module \"a\" {{ source = {value} }}\n");
        let file = parse(&source);
        assert!(file.references.is_empty(), "{value}");
        assert_eq!(
            errors(&source, &file),
            [(TerraformSyntaxError::EscapedReference(ReferenceKind::Module), value)]
        );
    }
    // A lone `$` or `%` is not an escape.
    assert_eq!(values(&parse("module \"a\" { source = \"./$a%b\" }")), ["./$a%b"]);
}

#[test]
fn reports_empty_references() {
    let source = "dependency \"a\" { config_path = \"\" }\n";
    let file = parse_terragrunt(source);
    assert!(file.references.is_empty());
    assert_eq!(
        errors(source, &file),
        [(TerraformSyntaxError::EmptyReference(ReferenceKind::Dependency), "\"\"")]
    );
}

#[test]
fn collects_comments_at_the_top_level_and_in_read_blocks() {
    let source = r#"# The network.
module "vpc" { // inline
  # above
  source = /* before */ "../vpc" # trailing
  /* last */
} # after

resource "x" "y" {
  # inside a resource
}

/*
 * end
 */
"#;
    let file = parse(source);
    assert_eq!(values(&file), ["../vpc"]);
    assert_eq!(
        comments(source, &file),
        [
            "# The network.",
            "// inline",
            "# above",
            "/* before */",
            "# trailing",
            "/* last */",
            "# after",
            "/*\n * end\n */",
        ]
    );
    let blocks: Vec<_> = file.comments.iter().map(|comment| comment.block).collect();
    assert_eq!(blocks, [false, false, false, true, false, true, false, true]);
    assert_ranges_valid(source, &file);
}

#[test]
fn collects_comments_between_dependency_paths() {
    let source = "dependencies {\n  paths = [\n    \"../a\", # first\n    // second\n    \"../b\",\n  ]\n}\n";
    let file = parse_terragrunt(source);
    assert_eq!(values(&file), ["../a", "../b"]);
    assert_eq!(comments(source, &file), ["# first", "// second"]);
}

#[test]
fn comments_hide_reference_like_text() {
    let source = "# module \"a\" { source = \"./a\" }\n/* module \"b\" { source = \"./b\" } */\n";
    let file = parse(source);
    assert!(file.references.is_empty());
    assert_eq!(comments(source, &file).len(), 2);
}

#[test]
fn reads_crlf_files() {
    let source = "# a\r\nmodule \"a\" {\r\n  source = \"./a\" // b\r\n}\r\n";
    let file = parse(source);
    assert_eq!(values(&file), ["./a"]);
    assert_eq!(comments(source, &file), ["# a", "// b"]);
    assert!(file.errors.is_empty());
}

#[test]
fn skips_a_byte_order_mark() {
    let source = "\u{feff}module \"a\" { source = \"./a\" }\n";
    let file = parse(source);
    assert_eq!(values(&file), ["./a"]);
    assert!(file.errors.is_empty());
}

#[test]
fn reads_unicode() {
    let source = "module \"сеть\" { source = \"../модули/сеть\" } # 日本\n";
    let file = parse(source);
    assert_eq!(values(&file), ["../модули/сеть"]);
    assert_eq!(comments(source, &file), ["# 日本"]);
    assert_ranges_valid(source, &file);
}

#[test]
fn reads_an_empty_file() {
    for source in ["", "\n", "# only a comment\n"] {
        let file = parse(source);
        assert!(file.references.is_empty() && file.errors.is_empty(), "{source:?}");
    }
}

#[test]
fn reports_a_missing_brace_and_keeps_what_comes_before() {
    let source = "module \"a\" { source = \"./a\" }\nmodule \"b\" {\n  source = \"./b\"\n";
    let file = parse(source);
    assert_eq!(values(&file), ["./a", "./b"]);
    assert_eq!(errors(source, &file), [(TerraformSyntaxError::Missing("}".into()), "")]);
    assert_eq!(file.errors[0].range.start() as usize, source.len());
}

#[test]
fn reports_an_unterminated_string() {
    let source = "module \"a\" {\n  source = \"./a\n}\n";
    let file = parse(source);
    assert!(file.references.is_empty());
    let missing: Vec<_> = file.errors.iter().map(|error| error.value.to_string()).collect();
    assert_eq!(missing, ["expected `\"`", "expected `}`"]);
    assert_ranges_valid(source, &file);
}

#[test]
fn reports_errors_anywhere_in_the_file() {
    // An error in a block that holds no references is still a syntax error.
    let source = "module \"a\" { source = \"./a\" }\nresource \"x\" \"y\" {\n  a = = 1\n}\n";
    let file = parse(source);
    assert_eq!(values(&file), ["./a"]);
    assert!(!file.errors.is_empty());
    assert_ranges_valid(source, &file);
}

#[test]
fn leaves_out_an_attribute_with_an_error() {
    let source =
        "module \"a\" {\n  source = \"./a\" \"./x\"\n}\nmodule \"b\" { source = \"./b\" }\n";
    let file = parse(source);
    assert_eq!(values(&file), ["./b"]);
    assert_eq!(errors(source, &file), [(TerraformSyntaxError::Invalid, "\"./a\"")]);
}

#[test]
fn keeps_an_attribute_next_to_an_error() {
    // The stray `+` is an error of its own, after the attribute.
    let source = "module \"a\" {\n  source = \"./a\" +\n}\nmodule \"b\" { source = \"./b\" }\n";
    let file = parse(source);
    assert_eq!(values(&file), ["./a", "./b"]);
    assert_eq!(errors(source, &file), [(TerraformSyntaxError::Invalid, "+")]);
}

#[test]
fn reports_an_unterminated_comment() {
    let source = "module \"a\" { source = \"./a\" }\n/* never closed\n";
    let file = parse(source);
    assert!(!file.errors.is_empty());
    assert_ranges_valid(source, &file);
}

#[test]
fn tolerates_garbage() {
    for source in [
        "\0\0\0",
        "{{{{{",
        "}}}}}",
        "module",
        "module \"a\"",
        "module \"a\" {",
        "module \"a\" { source",
        "module \"a\" { source =",
        "module \"a\" { source = }",
        "module \"a\" { source = \"",
        "module \"a\" { source = \"${",
        "module \"a\" { source = [\"./a\"] }",
        "module \"a\" { source = \"./a\" source = \"./b\" }",
        "dependencies { paths = [",
        "dependencies { paths = [\"./a\" \"./b\"] }",
        "dependencies { paths = \"./a\" }",
        "/",
        "#",
        "//",
        "/*",
        "/**/",
        "\u{feff}",
        "\u{feff}\u{feff}module \"a\" {}",
        "module \u{200b} \"a\" { source = \"./a\" }",
    ] {
        for file in [parse(source), parse_terragrunt(source)] {
            assert_ranges_valid(source, &file);
        }
    }
}

#[test]
fn displays_errors() {
    assert_eq!(TerraformSyntaxError::Invalid.to_string(), "invalid syntax");
    assert_eq!(TerraformSyntaxError::Missing("}".to_owned()).to_string(), "expected `}`");
    assert_eq!(
        TerraformSyntaxError::Missing("block_end".to_owned()).to_string(),
        "expected block end"
    );
    assert_eq!(
        TerraformSyntaxError::EscapedReference(ReferenceKind::Module).to_string(),
        "escapes in a module source are not supported"
    );
    assert_eq!(
        TerraformSyntaxError::EmptyReference(ReferenceKind::Dependency).to_string(),
        "the dependency path is empty"
    );
    assert_eq!(ReferenceKind::TerragruntSource.to_string(), "Terraform source");
}

/// A realistic Terraform file that uses most of what the parser reads.
const REALISTIC: &str = "\u{feff}# The production stack of the EU region.

terraform {
  required_version = \">= 1.6\"
  required_providers {
    aws = {
      source  = \"hashicorp/aws\"
      version = \"~> 5.0\"
    }
  }
  backend \"s3\" {}
}

locals {
  name = \"prod-${var.region}\"
}

// zenith-ignore stacks-independent: shared until the split
module \"vpc\" {
  source = \"../../../modules/vpc\" # the network

  cidr = \"10.0.0.0/16\"
  tags = { Name = local.name }
}

module \"consul\" {
  source  = \"hashicorp/consul/aws\"
  version = \"0.1.0\"
  for_each = toset([\"a\", \"b\"])
}

/* Old services. */
module \"legacy\" {
  source = \"git::https://example.com/legacy.git//services?ref=v1.2.0\"
  depends_on = [module.vpc]
}

module \"app\" {
  source = \"${path.module}/app\"
}

resource \"aws_s3_bucket\" \"logs\" {
  bucket = \"logs-${local.name}\" # not a reference
  lifecycle {
    prevent_destroy = true
  }
}

output \"vpc_id\" {
  value = module.vpc.id
}
";

#[test]
fn reads_a_realistic_file() {
    let file = parse(REALISTIC);
    assert!(file.errors.is_empty(), "{:?}", file.errors);
    assert_eq!(
        values(&file),
        [
            "../../../modules/vpc",
            "hashicorp/consul/aws",
            "git::https://example.com/legacy.git//services?ref=v1.2.0",
        ]
    );
    assert_eq!(non_literal(REALISTIC, &file), ["\"${path.module}/app\""]);
    assert_eq!(
        comments(REALISTIC, &file),
        [
            "# The production stack of the EU region.",
            "// zenith-ignore stacks-independent: shared until the split",
            "# the network",
            "/* Old services. */",
        ]
    );
    assert_ranges_valid(REALISTIC, &file);
}

#[test]
fn never_panics_on_a_truncated_file() {
    // Cut the file at every character boundary, which is what an interrupted
    // read, or a file that is being typed, looks like.
    for end in (0..=REALISTIC.len()).filter(|&end| REALISTIC.is_char_boundary(end)) {
        let source = &REALISTIC[..end];
        assert_ranges_valid(source, &parse(source));
    }
}

#[test]
fn never_panics_on_random_input() {
    // Strings of the pieces that are special in HCL syntax, from a fixed
    // pseudo-random sequence.
    const PIECES: &[&str] = &[
        "module",
        "terraform",
        "dependency",
        "dependencies",
        "source",
        "config_path",
        "paths",
        "\"a\"",
        "{",
        "}",
        "[",
        "]",
        "(",
        ")",
        "=",
        ",",
        "\"",
        "${",
        "$${",
        "%{",
        "\\",
        "<<EOT\n",
        "EOT",
        "#",
        "//",
        "/*",
        "*/",
        "\n",
        "\r\n",
        " ",
        "a",
        "é",
        "日",
        "\u{feff}",
        "0",
        "\0",
        "\u{1F600}",
    ];
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..3000 {
        let length = next() % 24;
        let mut source = String::new();
        for _ in 0..length {
            source.push_str(PIECES[(next() % PIECES.len() as u64) as usize]);
        }
        for file in [parse(&source), parse_terragrunt(&source)] {
            assert_ranges_valid(&source, &file);
        }
    }
}
