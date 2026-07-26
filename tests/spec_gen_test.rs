use std::path::Path;

use makerpm::parse::parse_pkgspec;
use makerpm::spec_gen;

fn load_fixture(name: &str) -> makerpm::model::PkgSpecFile {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let toml_str = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {name}: {e}"));
    parse_pkgspec(&toml_str).unwrap_or_else(|e| panic!("failed to parse fixture {name}: {e}"))
}

#[test]
fn render_plain_no_subpackage() {
    let spec = load_fixture("spec_plain_no_sub.toml");
    let rendered = spec_gen::render(&spec, &[]).unwrap();
    insta::assert_snapshot!(rendered);
}

#[test]
fn render_cmake_package() {
    let spec = load_fixture("spec_cmake.toml");
    let deps = vec!["cmake".into(), "gcc-c++".into()];
    let rendered = spec_gen::render(&spec, &deps).unwrap();
    insta::assert_snapshot!(rendered);
}

#[test]
fn render_subpkgs_shared_deps() {
    let spec = load_fixture("spec_subpkgs_shared_deps.toml");
    let rendered = spec_gen::render(&spec, &[]).unwrap();
    insta::assert_snapshot!(rendered);
}

#[test]
fn render_custom_scriptlets() {
    let spec = load_fixture("spec_custom_scriptlets.toml");
    let rendered = spec_gen::render(&spec, &[]).unwrap();
    insta::assert_snapshot!(rendered);
}

#[test]
fn render_percent_escaping() {
    let toml_str = r#"
[package]
name = "escape-test"
version = "1.0"
summary = "Tests percent escaping in descriptions"
license = "MIT"
description = "This package uses %{_libdir} and %{name} macros in its description."

[package.build]
system = "make"

[package.files]
paths = ["%{_bindir}/escape-test"]

[[package.changelog]]
version = "1.0-1"
date = "2026-07-01"
packager = "Test User <test@example.org>"
entries = ["Initial package with %percent signs"]
"#;
    let spec = parse_pkgspec(toml_str).unwrap();
    let rendered = spec_gen::render(&spec, &[]).unwrap();
    insta::assert_snapshot!(rendered);
}
