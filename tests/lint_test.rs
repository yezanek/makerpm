use std::path::Path;

use makerpm::lint::{lint, LintResult, Severity};
use makerpm::parse::parse_pkgspec;

fn load_fixture(fixture: &str) -> String {
    let path = format!("tests/fixtures/{fixture}");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"))
}

fn load_and_lint(fixture: &str) -> LintResult {
    let toml_str = load_fixture(fixture);
    let spec = parse_pkgspec(&toml_str).expect("fixture should parse");
    lint(&spec, Path::new("."), &toml_str)
}

fn has_finding(result: &LintResult, severity: Severity, substring: &str) -> bool {
    result
        .findings
        .iter()
        .any(|finding| finding.severity == severity && finding.message.contains(substring))
}

fn has_error(result: &LintResult, substring: &str) -> bool {
    has_finding(result, Severity::Error, substring)
}

fn has_warning(result: &LintResult, substring: &str) -> bool {
    has_finding(result, Severity::Warning, substring)
}

#[test]
fn version_hyphen_triggers_error() {
    let result = load_and_lint("err_version_hyphen.toml");
    assert!(result.has_errors());
    assert!(has_error(&result, "must not contain a literal '-'"));
}

#[test]
fn license_unknown_triggers_warning() {
    let result = load_and_lint("err_license_unknown.toml");
    assert!(!result.has_errors());
    assert!(has_warning(&result, "not a valid SPDX expression"));
}

#[test]
fn local_source_missing_triggers_error() {
    let result = load_and_lint("err_local_source_missing.toml");
    assert!(result.has_errors());
    assert!(has_error(&result, "local source file not found"));
}

#[test]
fn sha256_length_mismatch_triggers_error() {
    let result = load_and_lint("err_sha256_length_mismatch.toml");
    assert!(result.has_errors());
    assert!(has_error(
        &result,
        "sha256sums has 2 entries but sources has 1"
    ));
}

#[test]
fn unverified_source_triggers_warning() {
    let result = load_and_lint("warn_unverified_source.toml");
    assert!(!result.has_errors());
    assert!(has_warning(&result, "has no declared sha256sums entry"));
}

#[test]
fn subpackage_empty_fields_triggers_error() {
    let result = load_and_lint("err_subpackage_empty_fields.toml");
    assert!(result.has_errors());
    assert!(has_error(&result, "empty summary"));
    assert!(has_error(&result, "empty description"));
    assert!(has_error(&result, "has no files declared"));
}

#[test]
fn suffix_not_unique_triggers_error() {
    let result = load_and_lint("err_suffix_not_unique.toml");
    assert!(result.has_errors());
    assert!(has_error(&result, "duplicate subpackage suffix"));
}

#[test]
fn changelog_empty_triggers_warning() {
    let result = load_and_lint("warn_changelog_empty.toml");
    assert!(has_warning(&result, "changelog is empty"));
}

#[test]
fn duplicate_subpackage_summary_triggers_warning() {
    let result = load_and_lint("warn_duplicate_subpackage_summary.toml");
    assert!(has_warning(&result, "summary is identical"));
}

#[test]
fn release_without_dist_triggers_warning() {
    let result = load_and_lint("warn_release_without_dist.toml");
    assert!(has_warning(&result, "does not end in %{?dist}"));
}

#[test]
fn short_package_and_subpackage_descriptions_trigger_warnings() {
    let result = load_and_lint("warn_short_descriptions.toml");
    assert_eq!(
        result
            .findings
            .iter()
            .filter(|finding| finding.severity == Severity::Warning
                && finding.message.contains("shorter than 10 characters"))
            .count(),
        2
    );
}

#[test]
fn todo_comment_triggers_warning() {
    let result = load_and_lint("warn_todo_comment.toml");
    let finding = result
        .findings
        .iter()
        .find(|finding| finding.message.contains("unresolved # TODO"))
        .expect("TODO comment should produce a warning");
    assert_eq!(finding.severity, Severity::Warning);
    assert!(finding.field_path.starts_with("line "));
    assert!(finding.suggestion.is_some());
}

#[test]
fn file_overlap_triggers_error() {
    let result = load_and_lint("err_file_overlap.toml");
    assert!(result.has_errors());
    assert!(has_error(&result, "is claimed by both"));
}

#[test]
fn run_tests_false_with_check_triggers_warning() {
    let result = load_and_lint("warn_run_tests_false_with_check.toml");
    assert!(has_warning(&result, "run_tests is explicitly false"));
}

#[test]
fn valid_minimal_has_no_errors() {
    let result = load_and_lint("valid_minimal.toml");
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
    let result = lint(&spec, Path::new("."), toml_str);
    assert!(result.injected_build_deps.contains(&"cmake".to_string()));
    assert!(result.injected_build_deps.contains(&"gcc-c++".to_string()));
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
    let result = lint(&spec, Path::new("."), toml_str);
    assert!(result.has_errors());
    assert!(has_error(
        &result,
        "patch_sha256sums has 2 entries but patches has 1"
    ));
}

#[test]
fn subpackage_build_deps_detected_during_lint() {
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
    let result = lint(&spec, Path::new("."), toml_str);
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
    let result = lint(&spec, Path::new("."), toml_str);
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
    let result = lint(&spec, Path::new("."), toml_str);
    assert!(!result.has_unverified_sources);
}

#[test]
fn duplicate_resolved_source_names_are_rejected() {
    let toml_str = r#"
[package]
name = "duplicates"
version = "1.0"
summary = "Duplicate sources"
license = "MIT"
description = "Duplicate resolved source names."
sources = ["https://one.example/data.tar.gz", "https://two.example/data.tar.gz"]
sha256sums = ["SKIP", "SKIP"]
"#;
    let spec = parse_pkgspec(toml_str).unwrap();
    let result = lint(&spec, Path::new("."), toml_str);
    assert!(result.has_errors());
    assert!(has_error(&result, "resolved filename"));
}

#[test]
fn unsafe_resolved_source_name_is_rejected_during_lint() {
    let toml_str = r#"
[package]
name = "unsafe-source"
version = "1.0"
summary = "Unsafe source"
license = "MIT"
description = "Unsafe resolved source name."
sources = ["../data::https://example.org/data"]
sha256sums = ["SKIP"]
"#;
    let spec = parse_pkgspec(toml_str).unwrap();
    let result = lint(&spec, Path::new("."), toml_str);
    assert!(result.has_errors());
    assert!(has_error(&result, "unsafe filename"));
}

#[test]
fn trailing_separator_resolved_source_name_is_rejected() {
    let toml_str = r#"
[package]
name = "trailing-separator"
version = "1.0"
summary = "Trailing separator"
license = "MIT"
description = "Rejects normalized filenames."
sources = ["data/::https://example.org/data"]
sha256sums = ["SKIP"]
"#;
    let spec = parse_pkgspec(toml_str).unwrap();
    let result = lint(&spec, Path::new("."), toml_str);
    assert!(result.has_errors());
    assert!(has_error(&result, "unsafe filename"));
}
