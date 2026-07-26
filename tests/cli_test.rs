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
