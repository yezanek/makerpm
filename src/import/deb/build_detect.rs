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
    let system = if source_dir.join("CMakeLists.txt").is_file() {
        BuildSystem::Cmake
    } else if source_dir.join("meson.build").is_file() {
        BuildSystem::Meson
    } else if source_dir.join("configure.ac").is_file() || source_dir.join("configure").is_file() {
        BuildSystem::Autotools
    } else if source_dir.join("Cargo.toml").is_file() {
        BuildSystem::Cargo
    } else if source_dir.join("pyproject.toml").is_file() {
        BuildSystem::PythonPyproject
    } else if source_dir.join("Makefile").is_file() {
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
        let placeholder = Some(String::new());
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
        assert_eq!(detection.build.steps.prep, Some(String::new()));
        assert_eq!(detection.build.steps.install, Some(String::new()));
    }
}
