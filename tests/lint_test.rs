use std::path::Path;

use makerpm::lint::{lint, LintResult, Severity};
use makerpm::parse::parse_rpmspec;

fn load_fixture(fixture: &str) -> String {
    let path = format!("tests/fixtures/{fixture}");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"))
}

fn load_and_lint(fixture: &str) -> LintResult {
    let toml_str = load_fixture(fixture);
    let spec = parse_rpmspec(&toml_str).expect("fixture should parse");
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
    let result = load_and_lint("inject_cmake_build_requires.toml");
    assert!(result.injected_build_deps.contains(&"cmake".to_string()));
    assert!(result.injected_build_deps.contains(&"gcc-c++".to_string()));
}

#[test]
fn patch_sha256_length_mismatch() {
    let result = load_and_lint("err_patch_sha256_length_mismatch.toml");
    assert!(result.has_errors());
    assert!(has_error(
        &result,
        "patch_sha256sums has 2 entries but patches has 1"
    ));
}

#[test]
fn subpackage_build_deps_detected_during_lint() {
    let result = load_and_lint("valid_subpackage_build_deps.toml");
    assert!(!result.has_errors());
}

#[test]
fn has_unverified_sources_flag_set() {
    let result = load_and_lint("warn_source_unverified.toml");
    assert!(result.has_unverified_sources);
}

#[test]
fn no_unverified_sources_when_checksums_present() {
    let result = load_and_lint("valid_source_verified.toml");
    assert!(!result.has_unverified_sources);
}

#[test]
fn duplicate_resolved_source_names_are_rejected() {
    let result = load_and_lint("err_duplicate_source_names.toml");
    assert!(result.has_errors());
    assert!(has_error(&result, "resolved filename"));
}

#[test]
fn unsafe_resolved_source_name_is_rejected_during_lint() {
    let result = load_and_lint("err_unsafe_source_name.toml");
    assert!(result.has_errors());
    assert!(has_error(&result, "unsafe filename"));
}

#[test]
fn trailing_separator_resolved_source_name_is_rejected() {
    let result = load_and_lint("err_trailing_separator_source_name.toml");
    assert!(result.has_errors());
    assert!(has_error(&result, "unsafe filename"));
}

#[test]
fn todo_text_inside_toml_strings_is_ignored() {
    let result = load_and_lint("valid_todo_text_in_strings.toml");
    assert!(!has_warning(&result, "unresolved # TODO"));
}

#[test]
fn unsafe_local_source_does_not_expose_filesystem_state() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(directory.path().join("outside.tar.gz"), "existing").unwrap();
    let toml = load_fixture("err_unsafe_local_source.toml");
    let spec = parse_rpmspec(&toml).unwrap();
    let result = lint(&spec, &project, &toml);
    assert!(has_error(&result, "unsafe filename"));
    assert!(!has_error(&result, "local source file"));
    assert!(!has_error(&result, "regular file"));
}
