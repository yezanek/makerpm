pub mod deps;
pub mod pkgbuild_parser;

use std::io::Read;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::model::{
    BuildSpec, BuildSteps, BuildSystem, DependencySet, FilesSpec, Package, PkgSpecFile, Scriptlets,
};

use super::{Confidence, ImportDraft, ImportNote};
use pkgbuild_parser::{AssignmentValue, ParsedPkgbuild};

const FILES_PLACEHOLDER: &str = "%{_prefix}/TODO_REPLACE_WITH_PACKAGE_FILES";
const FILES_NOTE: &str = "file list not imported — PKGBUILD package() functions do not declare a structured file list; populate manually after a test build";
const PKGBUILD_LIMIT: u64 = 16 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum ArchImportError {
    #[error("not a PKGBUILD: missing file {0}")]
    Missing(PathBuf),

    #[error("failed to read PKGBUILD {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("refusing to read PKGBUILD {path}: input exceeds the {limit} byte limit")]
    InputTooLarge { path: PathBuf, limit: u64 },

    #[error("failed to parse PKGBUILD: {0}")]
    Parse(#[from] pkgbuild_parser::ParseError),

    #[error("PKGBUILD is missing required field {0}")]
    MissingField(&'static str),
}

pub fn import_pkgbuild(path: &Path) -> Result<ImportDraft, ArchImportError> {
    if !path.is_file() {
        return Err(ArchImportError::Missing(path.to_path_buf()));
    }
    let input = read_pkgbuild(path, PKGBUILD_LIMIT)?;
    let parsed = pkgbuild_parser::parse(&input)?;
    draft_from_parsed(&parsed)
}

fn read_pkgbuild(path: &Path, limit: u64) -> Result<String, ArchImportError> {
    let file = std::fs::File::open(path).map_err(|source| ArchImportError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| ArchImportError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    if bytes.len() as u64 > limit {
        return Err(ArchImportError::InputTooLarge {
            path: path.to_path_buf(),
            limit,
        });
    }
    String::from_utf8(bytes).map_err(|source| ArchImportError::Read {
        path: path.to_path_buf(),
        source: std::io::Error::new(std::io::ErrorKind::InvalidData, source),
    })
}

pub fn draft_from_parsed(parsed: &ParsedPkgbuild) -> Result<ImportDraft, ArchImportError> {
    let mut notes = Vec::new();

    let pkgname = required(parsed, "pkgname")?;
    let names = pkgname.values();
    let name = names[0].to_string();
    if names.len() > 1 {
        note(
            &mut notes,
            "package.name",
            format!(
                "split-package PKGBUILD detected; imported {} only, convert additional packages ({}) into subpackages manually",
                names[0],
                names[1..].join(", ")
            ),
            Confidence::Unsupported,
        );
    } else {
        confident(&mut notes, "package.name", "copied directly from pkgname");
    }
    flag_command_substitution(parsed, "pkgname", "package.name", &mut notes);

    let version = first(required(parsed, "pkgver")?).to_string();
    if parsed.functions.contains_key("pkgver") {
        note(
            &mut notes,
            "package.version",
            "PKGBUILD defines a dynamic pkgver(); this value may be stale, verify manually",
            Confidence::Unsupported,
        );
    } else if !flag_command_substitution(parsed, "pkgver", "package.version", &mut notes) {
        confident(&mut notes, "package.version", "copied directly from pkgver");
    }

    let pkgrel = first(required(parsed, "pkgrel")?);
    let release = format!("{pkgrel}%{{?dist}}");
    if !flag_command_substitution(parsed, "pkgrel", "package.release", &mut notes) {
        confident(
            &mut notes,
            "package.release",
            "copied from pkgrel and appended with the Fedora dist macro",
        );
    }

    let epoch = parsed
        .assignments
        .get("epoch")
        .and_then(|value| first(value).parse::<u32>().ok());
    if parsed.assignments.contains_key("epoch") {
        let has_command_substitution =
            flag_command_substitution(parsed, "epoch", "package.epoch", &mut notes);
        if epoch.is_some() && !has_command_substitution {
            confident(&mut notes, "package.epoch", "copied directly from epoch");
        } else if epoch.is_none() && !has_command_substitution {
            note(
                &mut notes,
                "package.epoch",
                "PKGBUILD epoch is not a static unsigned integer; set it manually",
                Confidence::Unsupported,
            );
        }
    }

    let summary = first(required(parsed, "pkgdesc")?).to_string();
    if !flag_command_substitution(parsed, "pkgdesc", "package.summary", &mut notes) {
        confident(
            &mut notes,
            "package.summary",
            "copied directly from pkgdesc",
        );
        confident(
            &mut notes,
            "package.description",
            "reused pkgdesc because PKGBUILD has no long-description field",
        );
    } else {
        note(
            &mut notes,
            "package.description",
            "package description also contains the unevaluated pkgdesc command substitution",
            Confidence::Unsupported,
        );
    }

    let arch = values(parsed, "arch");
    let noarch = arch.len() == 1 && arch[0] == "any";
    if !flag_command_substitution(parsed, "arch", "package.noarch", &mut notes) {
        if arch.is_empty() {
            note(
                &mut notes,
                "package.noarch",
                "PKGBUILD has no arch array; noarch was set to false",
                Confidence::BestEffort,
            );
        } else if noarch {
            confident(
                &mut notes,
                "package.noarch",
                "mapped directly from the PKGBUILD arch array containing only any",
            );
        } else {
            note(
                &mut notes,
                "package.noarch",
                "mapped from a PKGBUILD arch array that specifies concrete architectures",
                Confidence::BestEffort,
            );
        }
    }

    let url = parsed.assignments.get("url").map(first).map(str::to_string);
    if url.is_some() && !flag_command_substitution(parsed, "url", "package.url", &mut notes) {
        confident(&mut notes, "package.url", "copied directly from url");
    }

    let (license, license_confidence, license_note) = map_licenses(&values(parsed, "license"));
    note(
        &mut notes,
        "package.license",
        license_note,
        license_confidence,
    );
    flag_command_substitution(parsed, "license", "package.license", &mut notes);

    let mut package_deps = DependencySet::default();
    import_dependencies(
        parsed,
        &["depends", "depends_x86_64"],
        "package.deps.depends",
        &mut package_deps.depends,
        &mut notes,
    );
    import_dependencies(
        parsed,
        &["makedepends", "makedepends_x86_64"],
        "package.deps.build_depends",
        &mut package_deps.build_depends,
        &mut notes,
    );
    import_optdepends(parsed, &mut package_deps.recommends, &mut notes);
    import_dependencies(
        parsed,
        &["provides"],
        "package.deps.provides",
        &mut package_deps.provides,
        &mut notes,
    );
    import_dependencies(
        parsed,
        &["conflicts"],
        "package.deps.conflicts",
        &mut package_deps.conflicts,
        &mut notes,
    );
    import_dependencies(
        parsed,
        &["replaces"],
        "package.deps.obsoletes",
        &mut package_deps.obsoletes,
        &mut notes,
    );

    let sources = values(parsed, "source")
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    if parsed.assignments.contains_key("source")
        && !flag_command_substitution(parsed, "source", "package.sources", &mut notes)
    {
        confident(&mut notes, "package.sources", "copied directly from source");
    }
    let sha256sums = import_checksums(parsed, &mut notes);

    let steps = BuildSteps {
        prep: import_function(parsed, "prepare", "package.build.steps.prep", &mut notes),
        build: import_function(parsed, "build", "package.build.steps.build", &mut notes),
        install: import_function(parsed, "package", "package.build.steps.install", &mut notes),
        check: import_function(parsed, "check", "package.build.steps.check", &mut notes),
    };
    confident(
        &mut notes,
        "package.build.system",
        "set to none because imported PKGBUILD functions contain the complete raw build logic",
    );

    note(
        &mut notes,
        "package.files.paths",
        FILES_NOTE,
        Confidence::Unsupported,
    );

    if let Some(install) = parsed.assignments.get("install") {
        note(
            &mut notes,
            "package.scriptlets",
            format!(
                "PKGBUILD references .install scriptlet {}; populate scriptlets.* manually",
                first(install)
            ),
            Confidence::Unsupported,
        );
    }
    if parsed.has_additional_logic {
        note(
            &mut notes,
            "package.name",
            "PKGBUILD contains additional logic outside recognized fields; review the original file for anything not reflected here",
            Confidence::Unsupported,
        );
    }

    let consumed_fields = [
        "pkgname",
        "pkgver",
        "pkgrel",
        "epoch",
        "pkgdesc",
        "arch",
        "url",
        "license",
        "depends",
        "depends_x86_64",
        "makedepends",
        "makedepends_x86_64",
        "optdepends",
        "provides",
        "conflicts",
        "replaces",
        "source",
        "sha256sums",
        "b2sums",
        "md5sums",
        "install",
    ];
    let unrecognized = parsed
        .assignments
        .keys()
        .filter(|field| !consumed_fields.contains(&field.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !unrecognized.is_empty() {
        note(
            &mut notes,
            "package.name",
            format!(
                "PKGBUILD assignments not imported: {}",
                unrecognized.join(", ")
            ),
            Confidence::Unsupported,
        );
    }

    Ok(ImportDraft {
        spec: PkgSpecFile {
            package: Package {
                name,
                version,
                release,
                epoch,
                summary: summary.clone(),
                license,
                url,
                group: None,
                noarch,
                description: summary,
                sources,
                sha256sums,
                patches: Vec::new(),
                patch_sha256sums: Vec::new(),
                deps: package_deps,
                build: BuildSpec {
                    system: BuildSystem::None,
                    steps,
                    ..BuildSpec::default()
                },
                files: FilesSpec {
                    paths: vec![FILES_PLACEHOLDER.to_string()],
                    ..FilesSpec::default()
                },
                scriptlets: Scriptlets::default(),
                changelog: Vec::new(),
            },
            subpackages: Vec::new(),
        },
        notes,
    })
}

fn required<'a>(
    parsed: &'a ParsedPkgbuild,
    field: &'static str,
) -> Result<&'a AssignmentValue, ArchImportError> {
    parsed
        .assignments
        .get(field)
        .filter(|value| !value.values().is_empty())
        .ok_or(ArchImportError::MissingField(field))
}

fn first(value: &AssignmentValue) -> &str {
    value.values().into_iter().next().unwrap_or_default()
}

fn values<'a>(parsed: &'a ParsedPkgbuild, field: &str) -> Vec<&'a str> {
    parsed
        .assignments
        .get(field)
        .map(AssignmentValue::values)
        .unwrap_or_default()
}

fn flag_command_substitution(
    parsed: &ParsedPkgbuild,
    source_field: &str,
    target_field: &str,
    notes: &mut Vec<ImportNote>,
) -> bool {
    let found = parsed
        .assignments
        .get(source_field)
        .is_some_and(AssignmentValue::contains_command_substitution);
    if found {
        note(
            notes,
            target_field,
            format!(
                "PKGBUILD field {source_field} contains unevaluated command substitution; the literal text was retained as a placeholder"
            ),
            Confidence::Unsupported,
        );
    }
    found
}

fn import_dependencies(
    parsed: &ParsedPkgbuild,
    source_fields: &[&str],
    target_field: &str,
    output: &mut Vec<String>,
    notes: &mut Vec<ImportNote>,
) {
    for source_field in source_fields {
        let architecture_specific = source_field.ends_with("_x86_64");
        for dependency in values(parsed, source_field) {
            let translated = deps::translate(dependency);
            output.push(translated.value);
            let (confidence, message) = if pkgbuild_parser::contains_command_substitution(
                dependency,
            ) {
                (
                    Confidence::Unsupported,
                    format!(
                        "{source_field} dependency contains unevaluated command substitution; retained literally"
                    ),
                )
            } else if architecture_specific {
                (
                    Confidence::BestEffort,
                    format!(
                        "merged x86_64-specific dependency into the common list; {}",
                        translated.note
                    ),
                )
            } else {
                (Confidence::BestEffort, translated.note)
            };
            note(notes, target_field, message, confidence);
        }
    }
}

fn import_optdepends(
    parsed: &ParsedPkgbuild,
    output: &mut Vec<String>,
    notes: &mut Vec<ImportNote>,
) {
    for optional in values(parsed, "optdepends") {
        let (dependency, reason) = optional
            .split_once(": ")
            .map_or((optional, None), |(dependency, reason)| {
                (dependency, Some(reason))
            });
        let translated = deps::translate(dependency);
        output.push(translated.value);
        let note_text = match reason {
            Some(reason) => format!(
                "{}; dropped Arch optdepends reason: {reason}",
                translated.note
            ),
            None => format!("{}; no optdepends reason was present", translated.note),
        };
        let confidence = if pkgbuild_parser::contains_command_substitution(optional) {
            Confidence::Unsupported
        } else {
            Confidence::BestEffort
        };
        note(notes, "package.deps.recommends", note_text, confidence);
    }
}

fn import_checksums(parsed: &ParsedPkgbuild, notes: &mut Vec<ImportNote>) -> Vec<String> {
    if let Some(sha256) = parsed.assignments.get("sha256sums") {
        let sums = sha256
            .values()
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        if !flag_command_substitution(parsed, "sha256sums", "package.sha256sums", notes) {
            let source_count = parsed
                .assignments
                .iter()
                .filter(|(field, _)| *field == "source" || field.starts_with("source_"))
                .map(|(_, value)| value.values().len())
                .sum::<usize>();
            if sums.len() == source_count {
                confident(
                    notes,
                    "package.sha256sums",
                    "copied directly from sha256sums",
                );
            } else {
                note(
                    notes,
                    "package.sha256sums",
                    format!(
                        "copied directly from sha256sums, but found {} checksums for {source_count} effective source entries",
                        sums.len()
                    ),
                    Confidence::BestEffort,
                );
            }
        }
        return sums;
    }

    for field in ["b2sums", "md5sums"] {
        if let Some(checksums) = parsed.assignments.get(field) {
            note(
                notes,
                "package.sha256sums",
                format!(
                    "PKGBUILD provides {field} only; values were not written to sha256sums and sources must be re-hashed with SHA-256: {}",
                    checksums.values().join(", ")
                ),
                Confidence::Unsupported,
            );
            return Vec::new();
        }
    }
    if parsed.assignments.contains_key("source") {
        note(
            notes,
            "package.sha256sums",
            "PKGBUILD has sources but no sha256sums; calculate SHA-256 checksums manually",
            Confidence::Unsupported,
        );
    }
    Vec::new()
}

fn import_function(
    parsed: &ParsedPkgbuild,
    function: &str,
    target_field: &str,
    notes: &mut Vec<ImportNote>,
) -> Option<String> {
    let body = parsed.functions.get(function)?;
    let body = body
        .replace("${pkgdir}", "%{buildroot}")
        .replace("$pkgdir", "%{buildroot}");
    note(
        notes,
        target_field,
        format!("copied opaque {function}() shell body without execution"),
        Confidence::BestEffort,
    );
    if body.contains("$srcdir") || body.contains("${srcdir}") {
        note(
            notes,
            target_field,
            "function body retains $srcdir; adapt it to rpmbuild's prepared source directory manually",
            Confidence::Unsupported,
        );
    }
    if pkgbuild_parser::contains_command_substitution(&body) {
        note(
            notes,
            target_field,
            "function body contains unevaluated command substitution; review the opaque shell manually",
            Confidence::Unsupported,
        );
    }
    Some(body)
}

fn map_licenses(licenses: &[&str]) -> (String, Confidence, String) {
    if licenses.is_empty() {
        return (
            "LicenseRef-UNKNOWN".to_string(),
            Confidence::Unsupported,
            "PKGBUILD has no license value; determine the SPDX expression manually".to_string(),
        );
    }
    let mut mapped = Vec::new();
    let mut unsupported = Vec::new();
    for license in licenses {
        let spdx = match *license {
            "GPL2" => "GPL-2.0-or-later",
            "GPL3" => "GPL-3.0-or-later",
            "MIT" => "MIT",
            "Apache" => "Apache-2.0",
            "BSD" => "BSD-3-Clause",
            "ZLIB" => "Zlib",
            other => {
                unsupported.push(other);
                other
            }
        };
        mapped.push(spdx);
    }
    if unsupported.is_empty() {
        (
            mapped.join(" AND "),
            Confidence::BestEffort,
            "mapped Arch license identifiers to an SPDX expression".to_string(),
        )
    } else {
        (
            mapped.join(" AND "),
            Confidence::Unsupported,
            format!(
                "license identifiers were passed through without a known SPDX mapping: {}",
                unsupported.join(", ")
            ),
        )
    }
}

fn confident(notes: &mut Vec<ImportNote>, field_path: &str, message: &str) {
    note(notes, field_path, message, Confidence::Confident);
}

fn note(
    notes: &mut Vec<ImportNote>,
    field_path: &str,
    message: impl Into<String>,
    confidence: Confidence,
) {
    notes.push(ImportNote {
        field_path: field_path.to_string(),
        note: message.into(),
        confidence,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::render_import_draft;
    use crate::parse::parse_rpmspec;

    const PKGBUILD: &str = r#"
pkgname=hello-arch
pkgver=1.2.3
pkgrel=4
epoch=2
pkgdesc='A friendly example package with enough detail for RPM metadata'
arch=('any')
url='https://example.test/hello'
license=('MIT')
depends=('openssl>=3')
depends_x86_64=('libx86')
makedepends=('cmake')
optdepends=('bash: enables shell integration')
provides=('hello')
conflicts=('hello-old')
replaces=('hello-legacy')
source=('https://example.test/hello-1.2.3.tar.gz')
sha256sums=('aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa')

prepare() {
  patch -p1 < fix.patch
}
build() {
  make DESTDIR="$pkgdir"
}
package() {
  install -Dm755 hello "$pkgdir/usr/bin/hello"
}
check() {
  make test
}
"#;

    fn draft(input: &str) -> ImportDraft {
        let parsed = pkgbuild_parser::parse(input).unwrap();
        draft_from_parsed(&parsed).unwrap()
    }

    #[test]
    fn maps_a_complete_pkgbuild_and_round_trips_through_lint() {
        let draft = draft(PKGBUILD);
        assert!(draft.spec.package.noarch);
        assert_eq!(draft.spec.package.epoch, Some(2));
        assert_eq!(draft.spec.package.deps.depends, ["openssl >= 3", "libx86"]);
        assert_eq!(draft.spec.package.deps.recommends, ["bash"]);
        assert!(draft
            .spec
            .package
            .build
            .steps
            .install
            .as_deref()
            .unwrap()
            .contains("%{buildroot}/usr/bin/hello"));
        assert_eq!(draft.spec.package.files.paths, [FILES_PLACEHOLDER]);

        let rendered = render_import_draft(&draft).unwrap();
        let parsed = parse_rpmspec(&rendered).unwrap();
        let lint = crate::lint::lint(&parsed, Path::new("."), &rendered);
        assert!(!lint.has_errors(), "{:?}", lint.findings);
        assert!(rendered.contains("# TODO: file list not imported"));
    }

    #[test]
    fn marks_dynamic_values_and_opaque_srcdir_use_unsupported() {
        let input = PKGBUILD
            .replace("pkgver=1.2.3", "pkgver=$(git describe --tags)")
            .replace("epoch=2", "epoch=$(date +%s)")
            .replace(
                "prepare() {",
                "pkgver() {\n  git describe --tags\n}\nprepare() {",
            )
            .replace(
                "patch -p1 < fix.patch",
                "patch -p1 -d \"$srcdir\" < fix.patch",
            );
        let draft = draft(&input);
        assert!(draft.notes.iter().any(|note| {
            note.field_path == "package.version" && note.confidence == Confidence::Unsupported
        }));
        assert!(draft.notes.iter().any(|note| {
            note.field_path == "package.epoch"
                && note.note.contains("unevaluated command substitution")
                && note.confidence == Confidence::Unsupported
        }));
        assert!(!draft.notes.iter().any(|note| {
            note.field_path == "package.epoch"
                && note.note.contains("not a static unsigned integer")
        }));
        assert!(draft.notes.iter().any(|note| {
            note.field_path == "package.build.steps.prep"
                && note.confidence == Confidence::Unsupported
        }));
        assert_eq!(draft.spec.package.version, "$(git describe --tags)");
    }

    #[test]
    fn split_packages_install_scripts_and_absent_files_are_placeholders() {
        let input = PKGBUILD
            .replace("pkgname=hello-arch", "pkgname=('hello-arch' 'hello-docs')")
            .replace("pkgrel=4", "pkgrel=4\ninstall=hello.install");
        let draft = draft(&input);
        assert!(draft.notes.iter().any(|note| {
            note.note.contains("split-package") && note.confidence == Confidence::Unsupported
        }));
        assert!(draft.notes.iter().any(|note| {
            note.field_path == "package.scriptlets" && note.confidence == Confidence::Unsupported
        }));
        assert!(draft.spec.subpackages.is_empty());
        assert!(draft.spec.package.scriptlets.pre.is_none());
        assert_eq!(draft.spec.package.files.paths, [FILES_PLACEHOLDER]);
    }

    #[test]
    fn never_places_non_sha256_checksums_in_sha256_field() {
        let input = PKGBUILD.replace(
            "sha256sums=('aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa')",
            "b2sums=('not-a-sha256-value')",
        );
        let draft = draft(&input);
        assert!(draft.spec.package.sha256sums.is_empty());
        assert!(draft.notes.iter().any(|note| {
            note.field_path == "package.sha256sums"
                && note.note.contains("b2sums")
                && note.confidence == Confidence::Unsupported
        }));
    }

    #[test]
    fn checksum_confidence_accounts_for_architecture_specific_sources() {
        let input = PKGBUILD.replace(
            "source=('https://example.test/hello-1.2.3.tar.gz')",
            "source=('https://example.test/hello-1.2.3.tar.gz')\nsource_x86_64=('extra.tar.gz')",
        );
        let draft = draft(&input);
        assert!(draft.notes.iter().any(|note| {
            note.field_path == "package.sha256sums"
                && note.confidence == Confidence::BestEffort
                && note
                    .note
                    .contains("1 checksums for 2 effective source entries")
        }));
    }

    #[test]
    fn preserves_optdepends_reason_in_the_note() {
        let draft = draft(PKGBUILD);
        assert!(draft.notes.iter().any(|note| {
            note.field_path == "package.deps.recommends"
                && note.note.contains("enables shell integration")
        }));
    }

    #[test]
    fn rejects_pkgbuilds_over_the_configured_size_limit() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("PKGBUILD");
        std::fs::write(&path, "12345").unwrap();

        assert!(matches!(
            read_pkgbuild(&path, 4),
            Err(ArchImportError::InputTooLarge { limit: 4, .. })
        ));
    }
}
