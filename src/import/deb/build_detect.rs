use std::collections::BTreeMap;
use std::path::Path;

use crate::model::{BuildSpec, BuildSystem};

#[derive(Debug)]
pub struct BuildDetection {
    pub build: BuildSpec,
    pub overrides: Vec<DetectedOverride>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetectedOverride {
    pub target: String,
    pub step: &'static str,
}

pub fn detect(source_dir: &Path, rules_text: Option<&str>) -> BuildDetection {
    let Ok(canonical_source_dir) = source_dir.canonicalize() else {
        return BuildDetection {
            build: BuildSpec {
                system: BuildSystem::None,
                ..BuildSpec::default()
            },
            overrides: rules_text.map(detect_overrides).unwrap_or_default(),
        };
    };

    let system = if is_confined_file(&canonical_source_dir, "CMakeLists.txt") {
        BuildSystem::Cmake
    } else if is_confined_file(&canonical_source_dir, "meson.build") {
        BuildSystem::Meson
    } else if is_confined_file(&canonical_source_dir, "configure.ac")
        || is_confined_file(&canonical_source_dir, "configure")
    {
        BuildSystem::Autotools
    } else if is_confined_file(&canonical_source_dir, "Cargo.toml") {
        BuildSystem::Cargo
    } else if is_confined_file(&canonical_source_dir, "pyproject.toml") {
        BuildSystem::PythonPyproject
    } else if is_confined_file(&canonical_source_dir, "Makefile") {
        BuildSystem::Make
    } else {
        BuildSystem::None
    };

    let overrides = rules_text.map(detect_overrides).unwrap_or_default();
    let mut build = BuildSpec {
        system,
        ..BuildSpec::default()
    };
    for override_target in &overrides {
        let placeholder = Some("# TODO: translate the detected debian/rules override".to_string());
        match override_target.step {
            "prep" => build.steps.prep = placeholder,
            "build" => build.steps.build = placeholder,
            "install" => build.steps.install = placeholder,
            "check" => build.steps.check = placeholder,
            _ => unreachable!("override detector only returns known step names"),
        }
    }

    BuildDetection { build, overrides }
}

fn is_confined_file(canonical_source_dir: &Path, name: &str) -> bool {
    let path = canonical_source_dir.join(name);
    path.canonicalize()
        .is_ok_and(|resolved| resolved.starts_with(canonical_source_dir) && resolved.is_file())
}

fn detect_overrides(rules: &str) -> Vec<DetectedOverride> {
    let mut targets = BTreeMap::<String, &'static str>::new();
    for line in rules.lines() {
        let Some((target, _)) = line.trim().split_once(':') else {
            continue;
        };
        if !target.starts_with("override_dh_")
            || !target
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            continue;
        }
        let step = if target.contains("install") {
            "install"
        } else if target.contains("test") || target.contains("check") {
            "check"
        } else if target.contains("configure") || target.contains("clean") {
            "prep"
        } else {
            "build"
        };
        targets.insert(target.to_string(), step);
    }
    targets
        .into_iter()
        .map(|(target, step)| DetectedOverride { target, step })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_priority_is_deterministic() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("CMakeLists.txt"), "").unwrap();
        std::fs::write(directory.path().join("Cargo.toml"), "").unwrap();
        let detection = detect(directory.path(), None);
        assert_eq!(detection.build.system, BuildSystem::Cmake);
    }

    #[test]
    fn detects_every_supported_marker() {
        for (marker, expected) in [
            ("meson.build", BuildSystem::Meson),
            ("configure.ac", BuildSystem::Autotools),
            ("Cargo.toml", BuildSystem::Cargo),
            ("pyproject.toml", BuildSystem::PythonPyproject),
            ("Makefile", BuildSystem::Make),
        ] {
            let directory = tempfile::tempdir().unwrap();
            std::fs::write(directory.path().join(marker), "").unwrap();
            assert_eq!(detect(directory.path(), None).build.system, expected);
        }
    }

    #[test]
    fn only_reads_override_target_names_from_rules_text() {
        let rules = r#"#!/usr/bin/make -f
%:
	dh $@

override_dh_auto_configure:
	dh_auto_configure -- --unsafe-content-is-opaque

override_dh_auto_install: private prerequisite
	dh_auto_install
"#;
        let directory = tempfile::tempdir().unwrap();
        let detection = detect(directory.path(), Some(rules));
        assert_eq!(
            detection.overrides,
            [
                DetectedOverride {
                    target: "override_dh_auto_configure".to_string(),
                    step: "prep",
                },
                DetectedOverride {
                    target: "override_dh_auto_install".to_string(),
                    step: "install",
                }
            ]
        );
        assert!(detection
            .build
            .steps
            .prep
            .as_deref()
            .unwrap()
            .contains("TODO"));
        assert!(detection
            .build
            .steps
            .install
            .as_deref()
            .unwrap()
            .contains("TODO"));
    }

    #[cfg(unix)]
    #[test]
    fn ignores_marker_symlinks_that_escape_the_source_tree() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        symlink(outside.path(), directory.path().join("CMakeLists.txt")).unwrap();

        assert_eq!(
            detect(directory.path(), None).build.system,
            BuildSystem::None
        );
    }
}
