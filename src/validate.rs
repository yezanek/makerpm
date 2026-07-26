use std::path::Path;

use miette::{miette, Diagnostic, Report};
use thiserror::Error;

use crate::model::{BuildSystem, PkgSpecFile};
use crate::source_spec::{self, SourceEntry};

#[derive(Debug, Error, Diagnostic)]
#[error("{message}")]
#[diagnostic(severity(Warning))]
struct WarningDiagnostic {
    message: String,
}

fn warning(message: impl std::fmt::Display) -> Report {
    WarningDiagnostic {
        message: message.to_string(),
    }
    .into()
}

fn error(message: impl std::fmt::Display) -> Report {
    miette!("{message}")
}

pub struct ValidationResult {
    pub diagnostics: Vec<Report>,
    pub injected_build_deps: Vec<String>,
    pub has_unverified_sources: bool,
}

impl ValidationResult {
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity().unwrap_or(miette::Severity::Error) >= miette::Severity::Error)
    }
}

pub fn validate(spec: &PkgSpecFile, toml_dir: &Path) -> ValidationResult {
    let mut diagnostics = Vec::new();
    let mut injected_build_deps = Vec::new();

    validate_version(&spec.package.version, &mut diagnostics);
    validate_license(&spec.package.license, &mut diagnostics);
    validate_sources(spec, toml_dir, &mut diagnostics);
    validate_sha256_lengths(spec, &mut diagnostics);
    let has_unverified = validate_unverified_sources(spec, &mut diagnostics);
    validate_subpackages(spec, &mut diagnostics);
    validate_suffixes(spec, &mut diagnostics);
    inject_build_requires(spec, &mut injected_build_deps);
    validate_extra_args_with_no_macros(spec, &mut diagnostics);
    validate_test_defaults(spec, &mut diagnostics);
    validate_changelog(spec, &mut diagnostics);
    validate_file_overlap(spec, &mut diagnostics);

    ValidationResult {
        diagnostics,
        injected_build_deps,
        has_unverified_sources: has_unverified,
    }
}

fn validate_version(version: &str, diags: &mut Vec<Report>) {
    if version.contains('-') {
        diags.push(error(format!(
            "package.version must not contain a literal '-' (RPM restriction); \
             use the 'release' field instead. Found: \"{version}\""
        )));
    }
}

fn validate_license(license: &str, diags: &mut Vec<Report>) {
    use spdx::Expression;
    if Expression::parse(license).is_err() {
        diags.push(warning(format!(
            "package.license is not a valid SPDX expression: \"{license}\""
        )));
    }
}

fn validate_sources(spec: &PkgSpecFile, toml_dir: &Path, diags: &mut Vec<Report>) {
    let all_sources = spec
        .package
        .sources
        .iter()
        .map(|s| (s.as_str(), "source"))
        .chain(
            spec.package
                .patches
                .iter()
                .map(|s| (s.as_str(), "patch")),
        );

    for (raw, kind) in all_sources {
        if let SourceEntry::Local { filename } = source_spec::parse_source_entry(raw) {
            if filename.is_empty() {
                diags.push(error(format!(
                    "local {kind} has an empty filename"
                )));
                continue;
            }
            let path = toml_dir.join(&filename);
            if !path.exists() {
                diags.push(error(format!(
                    "local {kind} file not found: \"{filename}\" \
                     (expected at {})",
                    path.display()
                )));
            } else if !path.is_file() {
                diags.push(error(format!(
                    "local {kind} path is not a regular file: \"{filename}\" \
                     (at {})",
                    path.display()
                )));
            }
        }
    }
}

fn validate_sha256_lengths(spec: &PkgSpecFile, diags: &mut Vec<Report>) {
    if !spec.package.sha256sums.is_empty() {
        let s_len = spec.package.sha256sums.len();
        let src_len = spec.package.sources.len();
        if s_len != src_len {
            diags.push(error(format!(
                "sha256sums has {s_len} entries but sources has {src_len} entries; \
                 each source must have a corresponding sha256sums entry"
            )));
        }
    }
    if !spec.package.patch_sha256sums.is_empty() {
        let p_len = spec.package.patch_sha256sums.len();
        let patch_len = spec.package.patches.len();
        if p_len != patch_len {
            diags.push(error(format!(
                "patch_sha256sums has {p_len} entries but patches has {patch_len} entries; \
                 each patch must have a corresponding patch_sha256sums entry"
            )));
        }
    }
}

fn validate_unverified_sources(spec: &PkgSpecFile, diags: &mut Vec<Report>) -> bool {
    let mut found = false;
    for (i, raw) in spec.package.sources.iter().enumerate() {
        if let SourceEntry::Remote { filename, .. } = source_spec::parse_source_entry(raw) {
            let checksum = spec.package.sha256sums.get(i).map(String::as_str);
            if checksum.is_none() || checksum == Some("SKIP") {
                diags.push(warning(format!(
                    "remote source \"{filename}\" has no declared sha256sums entry; \
                     consider adding a checksum for verification"
                )));
                found = true;
            }
        }
    }
    for (i, raw) in spec.package.patches.iter().enumerate() {
        if let SourceEntry::Remote { filename, .. } = source_spec::parse_source_entry(raw) {
            let checksum = spec.package.patch_sha256sums.get(i).map(String::as_str);
            if checksum.is_none() || checksum == Some("SKIP") {
                diags.push(warning(format!(
                    "remote patch \"{filename}\" has no declared patch_sha256sums entry; \
                     consider adding a checksum for verification"
                )));
                found = true;
            }
        }
    }
    found
}

fn validate_subpackages(spec: &PkgSpecFile, diags: &mut Vec<Report>) {
    for sub in &spec.subpackages {
        if sub.summary.is_empty() {
            diags.push(error(format!(
                "subpackage \"{}\" has an empty summary",
                sub.suffix
            )));
        }
        if sub.description.is_empty() {
            diags.push(error(format!(
                "subpackage \"{}\" has an empty description",
                sub.suffix
            )));
        }
        if sub.files.is_empty() {
            diags.push(error(format!(
                "subpackage \"{}\" has no files declared; \
                 add at least one entry to [subpackage.files]",
                sub.suffix
            )));
        }
    }
}

fn validate_suffixes(spec: &PkgSpecFile, diags: &mut Vec<Report>) {
    let mut seen = std::collections::HashSet::new();
    for sub in &spec.subpackages {
        if sub.suffix.is_empty() {
            diags.push(error("subpackage has an empty suffix"));
        } else if !seen.insert(sub.suffix.clone()) {
            diags.push(error(format!(
                "duplicate subpackage suffix: \"{}\"",
                sub.suffix
            )));
        }
    }
}

fn inject_build_requires(spec: &PkgSpecFile, injected: &mut Vec<String>) {
    let system = &spec.package.build.system;
    if *system == BuildSystem::None {
        return;
    }
    for req in system.required_build_requires() {
        if !spec.package.deps.build_depends.iter().any(|d| d == req) {
            injected.push(req.to_string());
        }
    }
}

fn validate_extra_args_with_no_macros(spec: &PkgSpecFile, diags: &mut Vec<Report>) {
    if spec.package.build.system.macros().is_none() {
        if !spec.package.build.extra_build_args.is_empty() {
            diags.push(warning(
                "extra_build_args is set but the build system has no macros to apply them to; \
                 the args will be ignored",
            ));
        }
        if !spec.package.build.extra_install_args.is_empty() {
            diags.push(warning(
                "extra_install_args is set but the build system has no macros to apply them to; \
                 the args will be ignored",
            ));
        }
    }
}

fn validate_test_defaults(spec: &PkgSpecFile, diags: &mut Vec<Report>) {
    let has_check = spec.package.build.steps.check.is_some();
    let run_tests = spec.package.build.run_tests;
    if has_check && run_tests == Some(false) {
        diags.push(warning(
            "build.run_tests is explicitly false but build.steps.check is set; \
             %check will be omitted",
        ));
    }
}

fn validate_changelog(spec: &PkgSpecFile, diags: &mut Vec<Report>) {
    if spec.package.changelog.is_empty() {
        diags.push(warning(
            "package.changelog is empty; rpm/rpmlint will flag this",
        ));
    }
}

fn validate_file_overlap(spec: &PkgSpecFile, diags: &mut Vec<Report>) {
    let mut seen: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    for (path, owner) in spec
        .package
        .files
        .all_paths()
        .map(|p| (p.to_string(), "package".to_string()))
        .chain(
            spec.subpackages
                .iter()
                .flat_map(|sub| {
                    sub.files
                        .all_paths()
                        .map(move |p| (p.to_string(), format!("subpackage '{}'", sub.suffix)))
                }),
        )
    {
        if let Some(prev_owner) = seen.get(&path) {
            diags.push(error(format!(
                "file path \"{path}\" is claimed by both {prev_owner} and {owner}"
            )));
        } else {
            seen.insert(path, owner);
        }
    }
}
