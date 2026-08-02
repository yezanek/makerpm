use std::path::Path;

use assert_cmd::Command;
use makerpm::parse::parse_pkgspec;
use makerpm::validate::validate;

fn makerpm() -> Command {
    Command::cargo_bin("makerpm").unwrap()
}

#[test]
#[ignore = "requires rpmbuild installed"]
fn build_hello_world_end_to_end() {
    let spec_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/hello-world/PKGSPEC.toml");
    let output_dir = tempfile::tempdir().unwrap();

    makerpm()
        .arg("build")
        .arg("--spec-file")
        .arg(&spec_path)
        .arg("--output-dir")
        .arg(output_dir.path())
        .assert()
        .success();

    let rpms: Vec<_> = std::fs::read_dir(output_dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "rpm"))
        .collect();

    assert!(
        !rpms.is_empty(),
        "expected at least one .rpm in {}",
        output_dir.path().display()
    );

    let has_binary_rpm = rpms.iter().any(|e| {
        e.path().to_string_lossy().contains("hello-world")
            && !e.path().to_string_lossy().contains(".src.")
    });
    assert!(has_binary_rpm, "expected a hello-world binary RPM");
}

#[test]
#[ignore = "requires rpmbuild and network access"]
fn build_remote_source_end_to_end() {
    let spec_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/remote-hello/PKGSPEC.toml");
    let output_dir = tempfile::tempdir().unwrap();

    makerpm()
        .arg("build")
        .arg("--spec-file")
        .arg(&spec_path)
        .arg("--output-dir")
        .arg(output_dir.path())
        .assert()
        .success();

    let rpms: Vec<_> = std::fs::read_dir(output_dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "rpm"))
        .collect();

    assert!(
        !rpms.is_empty(),
        "expected at least one .rpm in {}",
        output_dir.path().display()
    );

    let has_binary_rpm = rpms.iter().any(|e| {
        let name = e.path().to_string_lossy().to_string();
        name.contains("remote-hello") && !name.contains(".src.")
    });
    assert!(has_binary_rpm, "expected a remote-hello binary RPM");
}

#[test]
fn build_failure_surfaces_rpmbuild_error() {
    let tmp = tempfile::tempdir().unwrap();
    let source_path = tmp.path().join("data.txt");
    std::fs::write(&source_path, b"content").unwrap();

    let broken_toml = r#"
[package]
name = "broken-pkg"
version = "1.0"
summary = "Broken"
license = "MIT"
description = "This package will fail to build."
sources = ["data.txt"]

[package.files]
paths = ["/usr/bin/nope"]
"#;
    let spec_path = tmp.path().join("PKGSPEC.toml");
    std::fs::write(&spec_path, broken_toml).unwrap();

    let output_dir = tmp.path().join("rpms");

    makerpm()
        .arg("build")
        .arg("--spec-file")
        .arg(&spec_path)
        .arg("--output-dir")
        .arg(&output_dir)
        .assert()
        .failure();
}

#[test]
fn init_creates_valid_pkgspec() {
    let tmp = tempfile::tempdir().unwrap();

    makerpm()
        .arg("init")
        .arg("--name")
        .arg("test-pkg")
        .current_dir(tmp.path())
        .assert()
        .success();

    let toml_path = tmp.path().join("PKGSPEC.toml");
    assert!(toml_path.exists(), "PKGSPEC.toml should be created");

    let toml_str = std::fs::read_to_string(&toml_path).unwrap();
    let spec = parse_pkgspec(&toml_str).expect("init-created TOML should parse");

    assert_eq!(spec.package.name, "test-pkg");

    let result = validate(&spec, tmp.path());
    assert!(
        !result.has_errors(),
        "init-created TOML should pass validation, got: {:?}",
        result
            .diagnostics
            .iter()
            .map(|d| format!("{d}"))
            .collect::<Vec<_>>()
    );
}
