use std::path::Path;

use makerpm::parse::parse_pkgspec;
use makerpm::validate::{validate, ValidationResult};

fn load_fixture(fixture: &str) -> String {
    let path = format!("tests/fixtures/{fixture}");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"))
}

fn load_and_validate(fixture: &str) -> ValidationResult {
    let toml_str = load_fixture(fixture);
    let spec = parse_pkgspec(&toml_str).expect("fixture should parse");
    validate(&spec, Path::new("."))
}

fn has_diagnostic_matching(result: &ValidationResult, substring: &str) -> bool {
    result
        .diagnostics
        .iter()
        .any(|d| format!("{d:?}").contains(substring))
}

fn has_warning_matching(result: &ValidationResult, substring: &str) -> bool {
    result.diagnostics.iter().any(|d| {
        let s = format!("{d:?}");
        s.contains(substring) && d.severity() == Some(miette::Severity::Warning)
    })
}

#[test]
fn version_hyphen_triggers_error() {
    let result = load_and_validate("err_version_hyphen.toml");
    assert!(result.has_errors());
    assert!(has_diagnostic_matching(
        &result,
        "must not contain a literal '-'"
    ));
}

#[test]
fn license_unknown_triggers_warning() {
    let result = load_and_validate("err_license_unknown.toml");
    assert!(has_diagnostic_matching(
        &result,
        "not a valid SPDX expression"
    ));
}

#[test]
fn local_source_missing_triggers_error() {
    let result = load_and_validate("err_local_source_missing.toml");
    assert!(result.has_errors());
    assert!(has_diagnostic_matching(
        &result,
        "local source file not found"
    ));
}

#[test]
fn sha256_length_mismatch_triggers_error() {
    let result = load_and_validate("err_sha256_length_mismatch.toml");
    assert!(result.has_errors());
    assert!(has_diagnostic_matching(
        &result,
        "sha256sums has 2 entries but sources has 1"
    ));
}

#[test]
fn unverified_source_triggers_warning() {
    let result = load_and_validate("warn_unverified_source.toml");
    assert!(!result.has_errors());
    assert!(has_warning_matching(
        &result,
        "has no declared sha256sums entry"
    ));
}

#[test]
fn subpackage_empty_fields_triggers_error() {
    let result = load_and_validate("err_subpackage_empty_fields.toml");
    assert!(result.has_errors());
    assert!(has_diagnostic_matching(&result, "empty summary"));
    assert!(has_diagnostic_matching(&result, "empty description"));
    assert!(has_diagnostic_matching(&result, "has no files declared"));
}

#[test]
fn suffix_not_unique_triggers_error() {
    let result = load_and_validate("err_suffix_not_unique.toml");
    assert!(result.has_errors());
    assert!(has_diagnostic_matching(&result, "duplicate subpackage suffix"));
}

#[test]
fn changelog_empty_triggers_warning() {
    let toml_str = r#"
[package]
name = "no-changelog"
version = "1.0"
summary = "Package with empty changelog"
license = "MIT"
description = "No changelog entries."
"#;
    let spec = parse_pkgspec(toml_str).unwrap();
    let result = validate(&spec, Path::new("."));
    assert!(has_warning_matching(&result, "changelog is empty"));
}

#[test]
fn file_overlap_triggers_error() {
    let result = load_and_validate("err_file_overlap.toml");
    assert!(result.has_errors());
    assert!(has_diagnostic_matching(&result, "is claimed by both"));
}

#[test]
fn run_tests_false_with_check_triggers_warning() {
    let result = load_and_validate("warn_run_tests_false_with_check.toml");
    assert!(has_warning_matching(
        &result,
        "run_tests is explicitly false"
    ));
}

#[test]
fn valid_minimal_has_no_errors() {
    let result = load_and_validate("valid_minimal.toml");
    assert!(!result.has_errors());
}

#[test]
fn auto_inject_build_requires() {
    let toml_str = r#"
[package]
name = "cmake-pkg"
version = "1.0"
summary = "Cmake package"
license = "MIT"
description = "Tests auto-injection."

[package.build]
system = "cmake"
"#;
    let spec = parse_pkgspec(toml_str).unwrap();
    let result = validate(&spec, Path::new("."));
    assert!(result.injected_build_deps.contains(&"cmake".to_string()));
    assert!(result
        .injected_build_deps
        .contains(&"gcc-c++".to_string()));
}

#[test]
fn patch_sha256_length_mismatch() {
    let toml_str = r#"
[package]
name = "patch-mismatch"
version = "1.0"
summary = "Test"
license = "MIT"
description = "Test"
patches = ["fix.patch"]
patch_sha256sums = ["abc", "def"]
"#;
    let spec = parse_pkgspec(toml_str).unwrap();
    let result = validate(&spec, Path::new("."));
    assert!(result.has_errors());
    assert!(has_diagnostic_matching(
        &result,
        "patch_sha256sums has 2 entries but patches has 1"
    ));
}

#[test]
fn subpackage_build_deps_detected_in_validation() {
    let toml_str = r#"
[package]
name = "subpkg-build"
version = "1.0"
summary = "Package with subpackage build deps"
license = "MIT"
description = "Tests subpackage build deps."

[package.files]
paths = ["/usr/bin/subpkg-build"]

[[subpackage]]
suffix = "devel"
summary = "Devel"
description = "Dev files."
files.paths = ["/usr/include/subpkg-build/"]

[subpackage.deps]
build_depends = ["cmake"]
"#;
    let spec = parse_pkgspec(toml_str).unwrap();
    let result = validate(&spec, Path::new("."));
    assert!(!result.has_errors());
}

#[test]
fn has_unverified_sources_flag_set() {
    let toml_str = r#"
[package]
name = "unverified"
version = "1.0"
summary = "Unverified remote source"
license = "MIT"
description = "Remote source without checksum."
sources = ["https://example.org/file.tar.gz"]
"#;
    let spec = parse_pkgspec(toml_str).unwrap();
    let result = validate(&spec, Path::new("."));
    assert!(result.has_unverified_sources);
}

#[test]
fn no_unverified_sources_when_checksums_present() {
    let toml_str = r#"
[package]
name = "verified"
version = "1.0"
summary = "Verified remote source"
license = "MIT"
description = "Remote source with checksum."
sources = ["https://example.org/file.tar.gz"]
sha256sums = ["9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a00"]
"#;
    let spec = parse_pkgspec(toml_str).unwrap();
    let result = validate(&spec, Path::new("."));
    assert!(!result.has_unverified_sources);
}
