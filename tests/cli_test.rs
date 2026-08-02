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

#[test]
fn import_aur_writes_annotated_draft_without_executing_pkgbuild() {
    let directory = tempfile::tempdir().unwrap();
    let pkgbuild = directory.path().join("PKGBUILD");
    let output = directory.path().join("package.toml");
    std::fs::write(
        &pkgbuild,
        r#"pkgname=cli-aur-import
pkgver=$(printf 'this must stay literal')
pkgrel=1
pkgdesc='CLI AUR import fixture with a sufficiently detailed summary'
arch=('any')
license=('MIT')
source=('https://example.test/source.tar.gz')
sha256sums=('aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa')
package() { install -Dm755 app "$pkgdir/usr/bin/app"; }
"#,
    )
    .unwrap();

    makerpm()
        .args(["import", "aur"])
        .arg(&pkgbuild)
        .arg("-o")
        .arg(&output)
        .assert()
        .success()
        .stderr(predicate::str::contains("Import summary"))
        .stderr(predicate::str::contains("Unsupported:"));

    let written = std::fs::read_to_string(output).unwrap();
    assert!(written.contains("$(printf 'this must stay literal')"));
    assert!(written.contains("%{buildroot}/usr/bin/app"));
    assert!(written.contains("# TODO:"));
}

fn write_debian_fixture(root: &std::path::Path) {
    let debian = root.join("debian");
    std::fs::create_dir(&debian).unwrap();
    std::fs::write(
        debian.join("control"),
        r#"Source: cli-import
Maintainer: Test <test@example.org>

Package: cli-import
Architecture: all
Description: CLI import fixture
 A complete Debian import fixture.
"#,
    )
    .unwrap();
    std::fs::write(
        debian.join("changelog"),
        r#"cli-import (1.0-1) unstable; urgency=medium

  * Initial release.

 -- Test <test@example.org>  Sun, 02 Aug 2026 12:34:56 +0200
"#,
    )
    .unwrap();
    std::fs::write(debian.join("copyright"), "License: MIT\n").unwrap();
}

#[test]
fn import_deb_writes_draft_and_summary() {
    let directory = tempfile::tempdir().unwrap();
    write_debian_fixture(directory.path());
    let output = directory.path().join("package.toml");

    makerpm()
        .args(["import", "deb"])
        .arg(directory.path())
        .args(["-o"])
        .arg(&output)
        .assert()
        .success()
        .stderr(predicate::str::contains("Import summary"))
        .stderr(predicate::str::contains("makerpm lint"));

    let written = std::fs::read_to_string(output).unwrap();
    assert!(written.contains("# TODO: file list not imported"));
}

#[test]
fn import_deb_requires_force_to_overwrite() {
    let directory = tempfile::tempdir().unwrap();
    write_debian_fixture(directory.path());
    let output = directory.path().join("package.toml");
    std::fs::write(&output, "existing").unwrap();

    makerpm()
        .args(["import", "deb"])
        .arg(directory.path())
        .args(["-o"])
        .arg(&output)
        .assert()
        .failure()
        .stderr(predicate::str::contains("--force"));

    makerpm()
        .args(["import", "deb"])
        .arg(directory.path())
        .args(["-o"])
        .arg(&output)
        .arg("--force")
        .assert()
        .success();
}

#[test]
fn import_deb_rejects_non_debian_source_directory() {
    let directory = tempfile::tempdir().unwrap();
    makerpm()
        .args(["import", "deb"])
        .arg(directory.path())
        .args(["-o"])
        .arg(directory.path().join("package.toml"))
        .assert()
        .failure()
        .stderr(predicate::str::contains("not a Debian source package"));
}
