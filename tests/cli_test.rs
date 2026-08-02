use assert_cmd::Command;
use predicates::prelude::*;

fn makerpm() -> Command {
    Command::cargo_bin("makerpm").unwrap()
}

#[test]
fn validate_clean_fixture_exits_zero() {
    makerpm()
        .arg("validate")
        .arg("--spec-file")
        .arg("tests/fixtures/valid_minimal.toml")
        .assert()
        .success();
}

#[test]
fn validate_version_hyphen_exits_nonzero() {
    makerpm()
        .arg("validate")
        .arg("--spec-file")
        .arg("tests/fixtures/err_version_hyphen.toml")
        .assert()
        .failure();
}

#[test]
fn validate_local_source_missing_exits_nonzero() {
    makerpm()
        .arg("validate")
        .arg("--spec-file")
        .arg("tests/fixtures/err_local_source_missing.toml")
        .assert()
        .failure();
}

#[test]
fn validate_unverified_source_exits_zero_with_warning() {
    makerpm()
        .arg("validate")
        .arg("--spec-file")
        .arg("tests/fixtures/warn_unverified_source.toml")
        .assert()
        .success();
}

#[test]
fn validate_suffix_not_unique_exits_nonzero() {
    makerpm()
        .arg("validate")
        .arg("--spec-file")
        .arg("tests/fixtures/err_suffix_not_unique.toml")
        .assert()
        .failure();
}

#[test]
fn validate_nonexistent_file_exits_nonzero() {
    makerpm()
        .arg("validate")
        .arg("--spec-file")
        .arg("tests/fixtures/nonexistent.toml")
        .assert()
        .failure()
        .stderr(predicate::str::contains("failed to read"));
}

#[test]
fn fetch_fails_without_allow_unverified() {
    makerpm()
        .arg("fetch")
        .arg("--spec-file")
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
        .arg("--spec-file")
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
        .args(["-v", "spec", "--spec-file"])
        .arg(spec_path)
        .assert()
        .success()
        .stderr(predicates::str::contains(
            "adding build requirement for the selected build system",
        ))
        .stderr(predicates::str::contains("dependency=cmake"));
}
