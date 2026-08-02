use makerpm::parse::parse_rpmspec;

#[test]
fn valid_toml_parses() {
    let toml_str = include_str!("fixtures/valid_minimal.toml");
    let spec = parse_rpmspec(toml_str).expect("should parse");
    assert_eq!(spec.package.name, "hello-world");
    assert_eq!(spec.package.version, "1.0");
    assert_eq!(spec.package.license, "MIT");
}

#[test]
fn malformed_toml_returns_error() {
    let toml_str = r#"
[package]
name = "broken"
version = 
"#;
    let result = parse_rpmspec(toml_str);
    assert!(result.is_err());
    let err = result.unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("failed to parse RPMSPEC.toml"));
}

#[test]
fn missing_required_field_returns_error() {
    let toml_str = r#"
[package]
name = "no-version"
summary = "Missing version"
license = "MIT"
description = "No version field."
"#;
    let result = parse_rpmspec(toml_str);
    assert!(result.is_err());
}

#[test]
fn empty_toml_returns_error() {
    let result = parse_rpmspec("");
    assert!(result.is_err());
}

#[test]
fn subpackages_deserialize() {
    let toml_str = r#"
[package]
name = "multi"
version = "1.0"
summary = "Multi"
license = "MIT"
description = "Has subpackages."

[[subpackage]]
suffix = "devel"
summary = "Devel"
description = "Dev files."
files.paths = ["%{_includedir}/foo"]

[[subpackage]]
suffix = "doc"
summary = "Doc"
description = "Documentation."
files.docs = ["%{_docdir}/foo"]
"#;
    let spec = parse_rpmspec(toml_str).unwrap();
    assert_eq!(spec.subpackages.len(), 2);
    assert_eq!(spec.subpackages[0].suffix, "devel");
    assert_eq!(spec.subpackages[1].suffix, "doc");
}
