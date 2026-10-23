//! The errors that `#[derive(Diagnostic)]` and `message!` give for code that
//! names the messages of the catalogue wrongly: each case in `compile-fail`
//! must fail to compile with the errors in its `.stderr` file. Re-record
//! them with `TRYBUILD=overwrite cargo test -p zenith_diagnostics --test
//! compile_fail`.

#[test]
fn the_macros_reject_what_the_catalogue_cannot_format() {
    trybuild::TestCases::new().compile_fail("tests/compile-fail/*.rs");
}
