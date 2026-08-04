use std::path::Path;

use crate::model::{BuildSystem, PkgSpecFile};
use crate::source_spec::{self, SourceEntry};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LintFinding {
    pub severity: Severity,
    pub field_path: String,
    pub message: String,
    pub suggestion: Option<String>,
}

impl LintFinding {
    fn error(field_path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            field_path: field_path.into(),
            message: message.into(),
            suggestion: None,
        }
    }

    fn warning(
        field_path: impl Into<String>,
        message: impl Into<String>,
        suggestion: impl Into<String>,
    ) -> Self {
        Self {
            severity: Severity::Warning,
            field_path: field_path.into(),
            message: message.into(),
            suggestion: Some(suggestion.into()),
        }
    }
}

#[derive(Debug)]
pub struct LintResult {
    pub findings: Vec<LintFinding>,
    pub injected_build_deps: Vec<String>,
    pub has_unverified_sources: bool,
}

impl LintResult {
    pub fn has_errors(&self) -> bool {
        self.findings
            .iter()
            .any(|finding| finding.severity == Severity::Error)
    }

    pub fn has_warnings(&self) -> bool {
        self.findings
            .iter()
            .any(|finding| finding.severity == Severity::Warning)
    }
}

pub fn lint(spec: &PkgSpecFile, toml_dir: &Path, raw_toml: &str) -> LintResult {
    let mut findings = Vec::new();
    let mut injected_build_deps = Vec::new();

    lint_version(&spec.package.version, &mut findings);
    lint_licenses(spec, &mut findings);
    lint_source_filenames(spec, &mut findings);
    lint_sources(spec, toml_dir, &mut findings);
    lint_sha256_lengths(spec, &mut findings);
    let has_unverified = lint_unverified_sources(spec, &mut findings);
    lint_subpackages(spec, &mut findings);
    lint_suffixes(spec, &mut findings);
    inject_build_requires(spec, &mut injected_build_deps);
    lint_extra_args_with_no_macros(spec, &mut findings);
    lint_test_defaults(spec, &mut findings);
    lint_changelog(spec, &mut findings);
    lint_release(spec, &mut findings);
    lint_descriptions(spec, &mut findings);
    lint_todo_comments(raw_toml, &mut findings);
    lint_file_overlap(spec, &mut findings);

    LintResult {
        findings,
        injected_build_deps,
        has_unverified_sources: has_unverified,
    }
}

fn lint_version(version: &str, findings: &mut Vec<LintFinding>) {
    if version.contains('-') {
        findings.push(LintFinding::error(
            "package.version",
            format!(
                "package.version must not contain a literal '-' (RPM restriction); \
             use the 'release' field instead. Found: \"{version}\""
            ),
        ));
    }
}

fn lint_licenses(spec: &PkgSpecFile, findings: &mut Vec<LintFinding>) {
    use spdx::Expression;
    if Expression::parse(&spec.package.license).is_err() {
        findings.push(LintFinding::warning(
            "package.license",
            format!(
                "package.license is not a valid SPDX expression: \"{}\"",
                spec.package.license
            ),
            "use a valid SPDX license expression",
        ));
    }
    for (index, subpackage) in spec.subpackages.iter().enumerate() {
        if let Some(license) = &subpackage.license {
            if Expression::parse(license).is_err() {
                findings.push(LintFinding::warning(
                    format!("subpackage[{index}].license"),
                    format!("subpackage license is not a valid SPDX expression: \"{license}\""),
                    "use a valid SPDX license expression",
                ));
            }
        }
    }
}

fn all_source_and_patch_entries(
    spec: &PkgSpecFile,
) -> impl Iterator<Item = (&'static str, usize, &str)> {
    spec.package
        .sources
        .iter()
        .enumerate()
        .map(|(index, source)| ("source", index, source.as_str()))
        .chain(
            spec.package
                .patches
                .iter()
                .enumerate()
                .map(|(index, patch)| ("patch", index, patch.as_str())),
        )
}

fn lint_sources(spec: &PkgSpecFile, toml_dir: &Path, findings: &mut Vec<LintFinding>) {
    for (kind, index, raw) in all_source_and_patch_entries(spec) {
        let field_path = format!("package.{kind}s[{index}]");
        if let SourceEntry::Local { filename } = source_spec::parse_source_entry(raw) {
            if filename.is_empty() {
                findings.push(LintFinding::error(
                    &field_path,
                    format!("local {kind} has an empty filename"),
                ));
                continue;
            }
            if !is_safe_source_filename(&filename) {
                continue;
            }
            let path = toml_dir.join(&filename);
            if !path.exists() {
                findings.push(LintFinding::error(
                    &field_path,
                    format!(
                        "local {kind} file not found: \"{filename}\" \
                     (expected at {})",
                        path.display()
                    ),
                ));
            } else if !path.is_file() {
                findings.push(LintFinding::error(
                    &field_path,
                    format!(
                        "local {kind} path is not a regular file: \"{filename}\" \
                     (at {})",
                        path.display()
                    ),
                ));
            }
        }
    }
}

fn lint_sha256_lengths(spec: &PkgSpecFile, findings: &mut Vec<LintFinding>) {
    if !spec.package.sha256sums.is_empty() {
        let s_len = spec.package.sha256sums.len();
        let src_len = spec.package.sources.len();
        if s_len != src_len {
            let detail = length_mismatch_detail("sha256sums", s_len, "source", src_len);
            findings.push(LintFinding::error(
                "package.sha256sums",
                format!(
                    "sha256sums has {s_len} entries but sources has {src_len} entries; {detail}"
                ),
            ));
        }
    }
    if !spec.package.patch_sha256sums.is_empty() {
        let p_len = spec.package.patch_sha256sums.len();
        let patch_len = spec.package.patches.len();
        if p_len != patch_len {
            let detail = length_mismatch_detail("patch_sha256sums", p_len, "patch", patch_len);
            findings.push(LintFinding::error(
                "package.patch_sha256sums",
                format!(
                "patch_sha256sums has {p_len} entries but patches has {patch_len} entries; {detail}"
            ),
            ));
        }
    }
}

fn length_mismatch_detail(
    sums: &str,
    sums_len: usize,
    entries: &str,
    entries_len: usize,
) -> String {
    if sums_len > entries_len {
        format!(
            "entry {} of {sums} has no matching {entries}",
            entries_len + 1
        )
    } else {
        format!(
            "entry {} of {entries}s has no matching {sums} entry",
            sums_len + 1
        )
    }
}

fn lint_source_filenames(spec: &PkgSpecFile, findings: &mut Vec<LintFinding>) {
    let mut seen = std::collections::HashMap::<String, String>::new();
    for (kind, index, raw) in all_source_and_patch_entries(spec) {
        let filename = match source_spec::parse_source_entry(raw) {
            SourceEntry::Local { filename } | SourceEntry::Remote { filename, .. } => filename,
        };
        if !is_safe_source_filename(&filename) {
            findings.push(LintFinding::error(format!("package.{kind}s[{index}]"), format!(
                "{kind} entry {} resolves to an unsafe filename: {filename:?}; filenames must be a single path component",
                index + 1
            )));
            continue;
        }
        let owner = format!("{kind} entry {}", index + 1);
        if let Some(previous) = seen.insert(filename.clone(), owner.clone()) {
            findings.push(LintFinding::error(format!("package.{kind}s[{index}]"), format!(
                "resolved filename {filename:?} is used by both {previous} and {owner}; use the filename::URL form to make names unique"
            )));
        }
    }
}

fn is_safe_source_filename(filename: &str) -> bool {
    let path = Path::new(filename);
    !filename.is_empty()
        && matches!(
            path.components().next(),
            Some(std::path::Component::Normal(component))
                if path.components().count() == 1 && component == path.as_os_str()
        )
}

fn lint_unverified_sources(spec: &PkgSpecFile, findings: &mut Vec<LintFinding>) -> bool {
    let mut found = false;
    for (kind, index, raw) in all_source_and_patch_entries(spec) {
        if let SourceEntry::Remote { filename, .. } = source_spec::parse_source_entry(raw) {
            let checksum = match kind {
                "source" => spec.package.sha256sums.get(index).map(String::as_str),
                "patch" => spec.package.patch_sha256sums.get(index).map(String::as_str),
                _ => None,
            };
            if checksum.is_none() || checksum == Some("SKIP") {
                let (checksum_field, suggestion) = if kind == "source" {
                    ("sha256sums", "add a SHA-256 checksum in package.sha256sums")
                } else {
                    (
                        "patch_sha256sums",
                        "add a SHA-256 checksum in package.patch_sha256sums",
                    )
                };
                findings.push(LintFinding::warning(
                    format!("package.{kind}s[{index}]"),
                    format!(
                        "remote {kind} \"{filename}\" has no declared {checksum_field} entry; \
                     consider adding a checksum for verification"
                    ),
                    suggestion,
                ));
                found = true;
            }
        }
    }
    found
}

fn lint_subpackages(spec: &PkgSpecFile, findings: &mut Vec<LintFinding>) {
    for (index, sub) in spec.subpackages.iter().enumerate() {
        if sub.summary.is_empty() {
            findings.push(LintFinding::error(
                format!("subpackage[{index}].summary"),
                format!("subpackage \"{}\" has an empty summary", sub.suffix),
            ));
        }
        if sub.description.is_empty() {
            findings.push(LintFinding::error(
                format!("subpackage[{index}].description"),
                format!("subpackage \"{}\" has an empty description", sub.suffix),
            ));
        }
        if sub.files.is_empty() {
            findings.push(LintFinding::error(
                format!("subpackage[{index}].files"),
                format!(
                    "subpackage \"{}\" has no files declared; \
                 add at least one entry to [subpackage.files]",
                    sub.suffix
                ),
            ));
        }
        if sub.summary == spec.package.summary {
            findings.push(LintFinding::warning(
                format!("subpackage[{index}].summary"),
                format!(
                    "subpackage \"{}\" summary is identical to package.summary",
                    sub.suffix
                ),
                "write a summary that distinguishes the subpackage",
            ));
        }
    }
}

fn lint_suffixes(spec: &PkgSpecFile, findings: &mut Vec<LintFinding>) {
    let mut seen = std::collections::HashSet::new();
    for (index, sub) in spec.subpackages.iter().enumerate() {
        if sub.suffix.is_empty() {
            findings.push(LintFinding::error(
                format!("subpackage[{index}].suffix"),
                "subpackage has an empty suffix",
            ));
        } else if !seen.insert(sub.suffix.clone()) {
            findings.push(LintFinding::error(
                format!("subpackage[{index}].suffix"),
                format!("duplicate subpackage suffix: \"{}\"", sub.suffix),
            ));
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

fn lint_extra_args_with_no_macros(spec: &PkgSpecFile, findings: &mut Vec<LintFinding>) {
    if spec.package.build.system.macros().is_none() {
        if !spec.package.build.extra_build_args.is_empty() {
            findings.push(LintFinding::warning(
                "package.build.extra_build_args",
                "extra_build_args is set but the build system has no macros to apply them to; \
                 the args will be ignored",
                "remove extra_build_args or select a build system that uses them",
            ));
        }
        if !spec.package.build.extra_install_args.is_empty() {
            findings.push(LintFinding::warning(
                "package.build.extra_install_args",
                "extra_install_args is set but the build system has no macros to apply them to; \
                 the args will be ignored",
                "remove extra_install_args or select a build system that uses them",
            ));
        }
    }
}

fn lint_test_defaults(spec: &PkgSpecFile, findings: &mut Vec<LintFinding>) {
    let has_check = spec.package.build.steps.check.is_some();
    let run_tests = spec.package.build.run_tests;
    if has_check && run_tests == Some(false) {
        findings.push(LintFinding::warning(
            "package.build.run_tests",
            "build.run_tests is explicitly false but build.steps.check is set; \
             %check will be omitted",
            "enable run_tests or remove build.steps.check",
        ));
    }
}

fn lint_changelog(spec: &PkgSpecFile, findings: &mut Vec<LintFinding>) {
    if spec.package.changelog.is_empty() {
        findings.push(LintFinding::warning(
            "package.changelog",
            "package.changelog is empty; rpm/rpmlint will flag this",
            "add at least one changelog entry",
        ));
    }
}

fn lint_release(spec: &PkgSpecFile, findings: &mut Vec<LintFinding>) {
    if !spec.package.release.ends_with("%{?dist}") {
        findings.push(LintFinding::warning(
            "package.release",
            format!(
                "package.release does not end in %{{?dist}}: \"{}\"",
                spec.package.release
            ),
            "append %{?dist} to follow Fedora release conventions",
        ));
    }
}

fn lint_descriptions(spec: &PkgSpecFile, findings: &mut Vec<LintFinding>) {
    const MIN_DESCRIPTION_CHARS: usize = 10;

    if spec.package.description.trim().chars().count() < MIN_DESCRIPTION_CHARS {
        findings.push(LintFinding::warning(
            "package.description",
            "package.description is shorter than 10 characters",
            "replace the placeholder with a meaningful package description",
        ));
    }

    for (index, subpackage) in spec.subpackages.iter().enumerate() {
        if subpackage.description.trim().chars().count() < MIN_DESCRIPTION_CHARS {
            findings.push(LintFinding::warning(
                format!("subpackage[{index}].description"),
                format!(
                    "subpackage \"{}\" description is shorter than 10 characters",
                    subpackage.suffix
                ),
                "replace the placeholder with a meaningful subpackage description",
            ));
        }
    }
}

fn lint_todo_comments(raw_toml: &str, findings: &mut Vec<LintFinding>) {
    for line in toml_todo_comment_lines(raw_toml) {
        findings.push(LintFinding::warning(
            format!("line {line}"),
            "unresolved # TODO comment in package definition",
            "review the imported value and remove the TODO comment",
        ));
    }
}

fn toml_todo_comment_lines(raw_toml: &str) -> Vec<usize> {
    #[derive(Clone, Copy)]
    enum StringKind {
        Basic,
        Literal,
        MultilineBasic,
        MultilineLiteral,
    }

    let bytes = raw_toml.as_bytes();
    let mut lines = Vec::new();
    let mut index = 0;
    let mut line = 1;
    let mut string = None;
    let mut escaped = false;

    while index < bytes.len() {
        if bytes[index] == b'\n' {
            line += 1;
            escaped = false;
            index += 1;
            continue;
        }

        match string {
            Some(StringKind::Basic) => {
                if escaped {
                    escaped = false;
                } else if bytes[index] == b'\\' {
                    escaped = true;
                } else if bytes[index] == b'"' {
                    string = None;
                }
                index += 1;
            }
            Some(StringKind::Literal) => {
                if bytes[index] == b'\'' {
                    string = None;
                }
                index += 1;
            }
            Some(StringKind::MultilineBasic) => {
                if bytes[index..].starts_with(b"\"\"\"") && !escaped {
                    string = None;
                    index += 3;
                } else {
                    escaped = bytes[index] == b'\\' && !escaped;
                    if bytes[index] != b'\\' {
                        escaped = false;
                    }
                    index += 1;
                }
            }
            Some(StringKind::MultilineLiteral) => {
                if bytes[index..].starts_with(b"'''") {
                    string = None;
                    index += 3;
                } else {
                    index += 1;
                }
            }
            None if bytes[index] == b'#' => {
                let end = bytes[index..]
                    .iter()
                    .position(|byte| *byte == b'\n')
                    .map_or(bytes.len(), |offset| index + offset);
                if raw_toml[index..end].contains("# TODO") {
                    lines.push(line);
                }
                index = end;
            }
            None if bytes[index..].starts_with(b"\"\"\"") => {
                string = Some(StringKind::MultilineBasic);
                index += 3;
            }
            None if bytes[index..].starts_with(b"'''") => {
                string = Some(StringKind::MultilineLiteral);
                index += 3;
            }
            None if bytes[index] == b'"' => {
                string = Some(StringKind::Basic);
                index += 1;
            }
            None if bytes[index] == b'\'' => {
                string = Some(StringKind::Literal);
                index += 1;
            }
            None => index += 1,
        }
    }
    lines
}

fn lint_file_overlap(spec: &PkgSpecFile, findings: &mut Vec<LintFinding>) {
    struct FileOwner {
        label: String,
        field_path: String,
    }

    let mut seen: std::collections::HashMap<String, FileOwner> = std::collections::HashMap::new();

    for (path, owner) in spec
        .package
        .files
        .all_paths()
        .map(|path| {
            (
                path.to_string(),
                FileOwner {
                    label: "package".to_string(),
                    field_path: "package.files".to_string(),
                },
            )
        })
        .chain(
            spec.subpackages
                .iter()
                .enumerate()
                .flat_map(|(index, sub)| {
                    sub.files.all_paths().map(move |path| {
                        (
                            path.to_string(),
                            FileOwner {
                                label: format!("subpackage '{}'", sub.suffix),
                                field_path: format!("subpackage[{index}].files"),
                            },
                        )
                    })
                }),
        )
    {
        if let Some(previous) = seen.get(&path) {
            findings.push(LintFinding::error(
                &owner.field_path,
                format!(
                    "file path \"{path}\" is claimed by both {} and {}",
                    previous.label, owner.label
                ),
            ));
        } else {
            seen.insert(path, owner);
        }
    }
}
