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

#[test]
fn render_subpkg_build_deps() {
    let spec = load_fixture("spec_subpkg_build_deps.toml");
    let rendered = spec_gen::render(&spec, &[]).unwrap();
    insta::assert_snapshot!(rendered);
}

#[test]
fn render_supplements_enhances() {
    let toml_str = r#"
[package]
name = "enhanced-pkg"
version = "1.0"
summary = "Enhanced package"
license = "MIT"
description = "Tests suggests/supplements/enhances."

[package.deps]
suggests = ["vim"]
supplements = ["system-release"]
enhances = ["vim-enhanced"]

[package.files]
paths = ["/usr/bin/enhanced-pkg"]

[[package.changelog]]
version = "1.0-1"
date = "2026-07-26"
packager = "Test User <test@example.org>"
entries = ["Initial release"]
"#;
    let spec = parse_pkgspec(toml_str).unwrap();
    let rendered = spec_gen::render(&spec, &[]).unwrap();
    assert!(rendered.contains("Suggests:"));
    assert!(rendered.contains("Supplements:"));
    assert!(rendered.contains("Enhances:"));
}

#[test]
fn render_base_pkg_no_scriptlet_n_flag() {
    let toml_str = r##"
[package]
name = "base-noflags"
version = "1.0"
summary = "Base package"
license = "MIT"
description = "Scriptlets without -n flag."
[package.files]
paths = ["/usr/bin/base-noflags"]

[package.scriptlets]
pre = "#!/bin/sh\necho pre"
post = "#!/bin/sh\necho post"

[[package.changelog]]
version = "1.0-1"
date = "2026-07-26"
packager = "Test User <test@example.org>"
entries = ["Initial release"]
"##;
    let spec = parse_pkgspec(toml_str).unwrap();
    let rendered = spec_gen::render(&spec, &[]).unwrap();
    assert!(rendered.contains("%pre\n"));
    assert!(!rendered.contains("-n base-noflags"));
}

#[test]
fn render_subpkg_pretrans_interpreter() {
    let toml_str = r##"
[package]
name = "interp-pretrans"
version = "1.0"
summary = "Pretrans interpreter test"
license = "MIT"
description = "Tests pretrans with custom interpreter."

[package.files]
paths = ["/usr/bin/interp-pretrans"]

[[subpackage]]
suffix = "sub"
summary = "Sub"
description = "Sub."
files.paths = ["/usr/bin/interp-sub"]

[subpackage.scriptlets]
interpreter = "/usr/bin/perl"
pretrans = "#!/usr/bin/perl\nprint pretrans"

[[package.changelog]]
version = "1.0-1"
date = "2026-07-26"
packager = "Test User <test@example.org>"
entries = ["Initial release"]
"##;
    let spec = parse_pkgspec(toml_str).unwrap();
    let rendered = spec_gen::render(&spec, &[]).unwrap();
    assert!(rendered.contains("%pretrans -p /usr/bin/perl\n"));
}
