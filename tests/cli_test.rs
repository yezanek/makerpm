use assert_cmd::Command;
use predicates::prelude::*;

fn makerpm() -> Command {
    Command::cargo_bin("makerpm").unwrap()
}

#[test]
fn lint_clean_fixture_exits_zero() {
    makerpm()
        .arg("lint")
        .arg("tests/fixtures/valid_minimal.toml")
        .assert()
        .success();
}

#[test]
fn lint_defaults_to_pkgspec_in_current_directory() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::copy(
        "tests/fixtures/valid_minimal.toml",
        directory.path().join("PKGSPEC.toml"),
    )
    .unwrap();

    makerpm()
        .arg("lint")
        .current_dir(directory.path())
        .assert()
        .success();
}

#[test]
fn lint_version_hyphen_exits_nonzero() {
    makerpm()
        .arg("lint")
        .arg("tests/fixtures/err_version_hyphen.toml")
        .assert()
        .failure();
}

#[test]
fn lint_local_source_missing_exits_nonzero() {
    makerpm()
        .arg("lint")
        .arg("tests/fixtures/err_local_source_missing.toml")
        .assert()
        .failure();
}

#[test]
fn lint_unverified_source_exits_zero_with_warning() {
    makerpm()
        .arg("lint")
        .arg("tests/fixtures/warn_unverified_source.toml")
        .assert()
        .success();
}

#[test]
fn lint_strict_exits_nonzero_for_warnings() {
    makerpm()
        .arg("lint")
        .arg("tests/fixtures/warn_release_without_dist.toml")
        .arg("--strict")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Warnings:"));
}

#[test]
fn lint_prints_errors_before_warnings() {
    let output = makerpm()
        .arg("lint")
        .arg("tests/fixtures/err_subpackage_empty_fields.toml")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let errors = stderr.find("Errors:").expect("error group should be shown");
    let warnings = stderr
        .find("Warnings:")
        .expect("warning group should be shown");
    assert!(
        errors < warnings,
        "errors should be printed before warnings"
    );
}

#[test]
fn validate_is_hidden_from_help() {
    makerpm()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("lint"))
        .stdout(predicate::str::contains("validate").not());
}

#[test]
fn lint_suffix_not_unique_exits_nonzero() {
    makerpm()
        .arg("lint")
        .arg("tests/fixtures/err_suffix_not_unique.toml")
        .assert()
        .failure();
}

#[test]
fn validate_nonexistent_file_exits_nonzero() {
    makerpm()
        .arg("validate")
        .arg("tests/fixtures/nonexistent.toml")
        .assert()
        .failure()
        .stderr(predicate::str::contains("failed to read"));
}

#[test]
fn fetch_fails_without_allow_unverified() {
    makerpm()
        .arg("fetch")
        .arg("tests/fixtures/warn_unverified_source.toml")
        .assert()
        .failure()
        .stderr(predicate::str::contains("unverified"));
}

#[test]
fn fetch_with_allow_unverified_passes_validation_gate() {
    let cache_dir = tempfile::tempdir().unwrap();
    let output = makerpm()
        .arg("fetch")
        .arg("tests/fixtures/warn_unverified_source.toml")
        .arg("--allow-unverified")
        .arg("--offline")
        .env("MAKERPM_SRCDEST", cache_dir.path())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unverified"),
        "should pass validation gate, got: {stderr}"
    );
    assert!(
        stderr.contains("download") || stderr.contains("uncached") || !output.status.success(),
        "should fail on download/uncached, not validation gate: {stderr}"
    );
}

#[test]
fn verbose_spec_reports_injected_build_dependencies() {
    let directory = tempfile::tempdir().unwrap();
    let spec_path = directory.path().join("PKGSPEC.toml");
    std::fs::write(
        &spec_path,
        r#"
[package]
name = "verbose-test"
version = "1.0"
summary = "Verbose test"
license = "MIT"
description = "Tests verbose dependency reporting."
[package.build]
system = "cmake"
"#,
    )
    .unwrap();
    makerpm()
        .args(["-v", "spec"])
        .arg(spec_path)
        .assert()
        .success()
        .stderr(predicates::str::contains(
            "adding build requirement for the selected build system",
        ))
        .stderr(predicates::str::contains("dependency=cmake"));
}
