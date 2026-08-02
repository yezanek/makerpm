use std::path::Path;

use makerpm::parse::parse_rpmspec;
use makerpm::spec_gen;

fn load_fixture(name: &str) -> makerpm::model::PkgSpecFile {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let toml_str = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {name}: {e}"));
    parse_rpmspec(&toml_str).unwrap_or_else(|e| panic!("failed to parse fixture {name}: {e}"))
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
    let spec = parse_rpmspec(toml_str).unwrap();
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
    let spec = parse_rpmspec(toml_str).unwrap();
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
    let spec = parse_rpmspec(toml_str).unwrap();
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
    let spec = parse_rpmspec(toml_str).unwrap();
    let rendered = spec_gen::render(&spec, &[]).unwrap();
    assert!(rendered.contains("%pretrans -n interp-pretrans-sub -p /usr/bin/perl\n"));
}

#[test]
fn every_subpackage_scriptlet_keeps_name_with_interpreter() {
    let toml_str = r##"
[package]
name = "scriptlet-target"
version = "1.0"
summary = "Scriptlet targets"
license = "MIT"
description = "Scriptlet target test."
[[subpackage]]
suffix = "sub"
summary = "Subpackage"
description = "Scriptlet subpackage."
files.paths = ["/usr/share/scriptlet-target"]
[subpackage.scriptlets]
interpreter = "/bin/bash"
pretrans = "echo pretrans"
pre = "echo pre"
post = "echo post"
preun = "echo preun"
postun = "echo postun"
posttrans = "echo posttrans"
"##;
    let spec = parse_rpmspec(toml_str).unwrap();
    let rendered = spec_gen::render(&spec, &[]).unwrap();
    for directive in ["pretrans", "pre", "post", "preun", "postun", "posttrans"] {
        assert!(
            rendered.contains(&format!(
                "%{directive} -n scriptlet-target-sub -p /bin/bash"
            )),
            "missing correctly targeted %{directive}"
        );
    }
}

#[test]
fn render_applies_declared_patches_by_default() {
    let toml_str = r#"
[package]
name = "patched"
version = "1.0"
summary = "Patched package"
license = "MIT"
description = "A patched package."
sources = ["source.tar.gz"]
patches = ["fix.patch"]
[package.files]
paths = ["/usr/bin/patched"]
"#;
    let spec = parse_rpmspec(toml_str).unwrap();
    let rendered = spec_gen::render(&spec, &[]).unwrap();
    assert!(rendered.contains("Patch0:       fix.patch"));
    assert!(rendered.contains("%autosetup -p1"));
}

#[test]
fn manual_patch_step_disables_automatic_patch_application() {
    let toml_str = r#"
[package]
name = "manually-patched"
version = "1.0"
summary = "Manually patched package"
license = "MIT"
description = "A package with manual patch handling."
sources = ["source.tar.gz"]
patches = ["fix.patch"]
[package.build.steps]
prep = "%patch 0 -p1"
[package.files]
paths = ["/usr/bin/manually-patched"]
"#;
    let spec = parse_rpmspec(toml_str).unwrap();
    let rendered = spec_gen::render(&spec, &[]).unwrap();
    assert!(rendered.contains("%autosetup -p1 -N"));
    assert_eq!(rendered.matches("%patch 0 -p1").count(), 1);
}

#[test]
fn autopatch_step_disables_automatic_patch_application() {
    let spec = spec_with_prep("%autopatch -p1");
    let rendered = spec_gen::render(&spec, &[]).unwrap();
    assert!(rendered.contains("%autosetup -p1 -N"));
    assert_eq!(rendered.matches("%autopatch").count(), 1);
}

#[test]
fn escaped_patch_literal_keeps_automatic_patch_application() {
    let spec = spec_with_prep("echo %%patch");
    let rendered = spec_gen::render(&spec, &[]).unwrap();
    assert!(rendered.contains("%autosetup -p1\n"));
    assert!(!rendered.contains("%autosetup -p1 -N"));
}

#[test]
fn numbered_patch_directive_disables_automatic_patch_application() {
    let spec = spec_with_prep("%patch12 -p1");
    let rendered = spec_gen::render(&spec, &[]).unwrap();
    assert!(rendered.contains("%autosetup -p1 -N"));
}

#[test]
fn patch_like_suffix_does_not_disable_automatic_patch_application() {
    let spec = spec_with_prep("%patches");
    let rendered = spec_gen::render(&spec, &[]).unwrap();
    assert!(rendered.contains("%autosetup -p1\n"));
    assert!(!rendered.contains("%autosetup -p1 -N"));
}

fn spec_with_prep(prep: &str) -> makerpm::model::PkgSpecFile {
    parse_rpmspec(&format!(
        r#"
[package]
name = "prep-test"
version = "1.0"
summary = "Prep test"
license = "MIT"
description = "Tests prep directives."
sources = ["source.tar.gz"]
patches = ["fix.patch"]
[package.build.steps]
prep = {prep:?}
[package.files]
paths = ["/usr/bin/prep-test"]
"#
    ))
    .unwrap()
}

#[test]
fn subpackage_noarch_inherits_base_value() {
    let toml_str = r#"
[package]
name = "docs"
version = "1.0"
summary = "Docs"
license = "MIT"
description = "Documentation."
noarch = true
[[subpackage]]
suffix = "extra"
summary = "Extra docs"
description = "Extra documentation."
files.paths = ["/usr/share/docs/extra"]
"#;
    let spec = parse_rpmspec(toml_str).unwrap();
    let rendered = spec_gen::render(&spec, &[]).unwrap();
    let subpackage = rendered
        .split("%package        extra")
        .nth(1)
        .expect("subpackage section should be rendered");
    assert!(subpackage.starts_with("\nSummary:        Extra docs\nBuildArch:      noarch"));
}

#[test]
fn changelog_version_is_part_of_header() {
    let spec = load_fixture("spec_plain_no_sub.toml");
    let rendered = spec_gen::render(&spec, &[]).unwrap();
    assert!(rendered.contains("* Mon Jun 29 2026 Jane Doe <jane@example.org> - 1.0-1"));
    assert!(!rendered.contains("\n- 1.0-1\n"));
}
