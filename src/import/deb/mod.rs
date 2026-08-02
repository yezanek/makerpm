pub mod build_detect;
pub mod changelog;
pub mod control;
pub mod deps;

use std::io::Read;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::model::{
    ChangelogEntry, DependencySet, FilesSpec, Package, PkgSpecFile, Scriptlets, Subpackage,
};

use super::{Confidence, ImportDraft, ImportNote};

const FILES_PLACEHOLDER_PREFIX: &str = "%{_prefix}/TODO_REPLACE_WITH_";
const FILES_NOTE: &str = "file list not imported — Debian and Fedora filesystem layouts differ; populate manually after a test build";
const CONTROL_LIMIT: u64 = 8 * 1024 * 1024;
const CHANGELOG_LIMIT: u64 = 32 * 1024 * 1024;
const OPTIONAL_METADATA_LIMIT: u64 = 8 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum DebImportError {
    #[error("not a Debian source package: missing {path}")]
    NotDebianSource { path: PathBuf },

    #[error("failed to read {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("refusing to read {path}: resolved path is outside the Debian source directory")]
    OutsideSource { path: PathBuf },

    #[error("refusing to read {path}: input exceeds the {limit} byte limit")]
    InputTooLarge { path: PathBuf, limit: u64 },

    #[error("failed to parse debian/control: {0}")]
    Control(#[from] control::ControlError),

    #[error("failed to parse debian/changelog: {0}")]
    Changelog(#[from] changelog::ChangelogError),
}

pub fn import_debian_source(source_dir: &Path) -> Result<ImportDraft, DebImportError> {
    let source_dir = std::fs::canonicalize(source_dir).map_err(|source| DebImportError::Read {
        path: source_dir.to_path_buf(),
        source,
    })?;
    let debian_dir = source_dir.join("debian");
    let control_path = debian_dir.join("control");
    let changelog_path = debian_dir.join("changelog");
    let control_text = read_required(&source_dir, &control_path, CONTROL_LIMIT)?;
    let changelog_text = read_required(&source_dir, &changelog_path, CHANGELOG_LIMIT)?;
    let control = control::parse(&control_text)?;
    let debian_changelog = changelog::parse(&changelog_text)?;
    let latest_version = changelog::split_version(&debian_changelog[0].version)?;

    let source_name = control
        .source
        .get("Source")
        .expect("control parser guarantees a Source field");
    let matching_base = control
        .binaries
        .iter()
        .position(|stanza| stanza.get("Package") == Some(source_name));
    let base_index = matching_base.unwrap_or(0);
    let base_stanza = &control.binaries[base_index];
    let base_name = base_stanza
        .get("Package")
        .expect("control parser only returns binary Package stanzas");
    let (summary, description) = control::description(base_stanza);
    let mut notes = Vec::new();

    if matching_base.is_some() {
        confident(
            &mut notes,
            "package.name",
            "copied from the Debian binary Package stanza matching Source",
        );
    } else {
        note(
            &mut notes,
            "package.name",
            "no binary Package matched Source; selected the first binary stanza",
            Confidence::BestEffort,
        );
    }
    confident(
        &mut notes,
        "package.summary",
        "copied from the Debian Description short form",
    );
    confident(
        &mut notes,
        "package.description",
        "copied from the Debian Description long form",
    );

    let (version, version_confidence, version_note) =
        sanitize_upstream_version(&latest_version.upstream);
    note(
        &mut notes,
        "package.version",
        version_note,
        version_confidence,
    );
    let release = format!(
        "{}%{{?dist}}",
        latest_version.revision.as_deref().unwrap_or("1")
    );
    note(
        &mut notes,
        "package.release",
        "derived from the Debian revision and adapted to Fedora's dist-tag convention",
        Confidence::BestEffort,
    );
    if latest_version.epoch.is_some() {
        confident(
            &mut notes,
            "package.epoch",
            "copied from the Debian version epoch",
        );
    }

    let (license, license_confidence, license_note) = import_license(&source_dir, &debian_dir)?;
    note(
        &mut notes,
        "package.license",
        license_note,
        license_confidence,
    );

    let homepage = control.source.get("Homepage").map(str::to_string);
    if homepage.is_some() {
        confident(
            &mut notes,
            "package.url",
            "copied from the Debian source Homepage field",
        );
    }

    let mut package_deps = DependencySet::default();
    import_dependencies(
        control.source.get("Build-Depends"),
        "package.deps.build_depends",
        &mut package_deps.build_depends,
        &mut notes,
    );
    import_dependencies(
        base_stanza.get("Depends"),
        "package.deps.depends",
        &mut package_deps.depends,
        &mut notes,
    );

    let rules_path = debian_dir.join("rules");
    let rules_text = read_optional(&source_dir, &rules_path, OPTIONAL_METADATA_LIMIT)?;
    let detection = build_detect::detect(&source_dir, rules_text.as_deref());
    note(
        &mut notes,
        "package.build.system",
        "build system inferred from source-tree marker files; review debian/rules manually",
        Confidence::BestEffort,
    );
    for override_target in &detection.overrides {
        note(
            &mut notes,
            &format!("package.build.steps.{}", override_target.step),
            format!(
                "Debian rules target {} was detected but its body was not translated",
                override_target.target
            ),
            Confidence::Unsupported,
        );
    }

    let files = placeholder_files("PACKAGE");
    note(
        &mut notes,
        "package.sources",
        "package sources were not imported from Debian metadata; populate them manually",
        Confidence::Unsupported,
    );
    note(
        &mut notes,
        "package.sha256sums",
        "source checksums were not imported from Debian metadata; calculate SHA-256 checksums manually",
        Confidence::Unsupported,
    );
    note(
        &mut notes,
        "package.files.paths",
        FILES_NOTE,
        Confidence::Unsupported,
    );

    let changelog = debian_changelog
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            note(
                &mut notes,
                &format!("package.changelog[{index}]"),
                "copied from a well-formed Debian changelog entry",
                if entry.date_normalization_failed {
                    Confidence::BestEffort
                } else {
                    Confidence::Confident
                },
            );
            ChangelogEntry {
                version: entry.version.clone(),
                date: entry.date.clone(),
                packager: if entry.maintainer.is_empty() {
                    control
                        .source
                        .get("Maintainer")
                        .unwrap_or("Unknown Maintainer")
                        .to_string()
                } else {
                    entry.maintainer.clone()
                },
                entries: entry.entries.clone(),
            }
        })
        .collect();

    let noarch = base_stanza.get("Architecture") == Some("all");
    if let Some(architecture) = base_stanza.get("Architecture") {
        confident(
            &mut notes,
            "package.noarch",
            if architecture == "all" {
                "mapped Debian Architecture: all directly to noarch"
            } else {
                "mapped a non-all Debian Architecture directly to architecture-specific output"
            },
        );
    }

    let mut subpackages = Vec::new();
    let mut used_suffixes = std::collections::HashSet::new();
    for (binary_index, stanza) in control.binaries.iter().enumerate() {
        if binary_index == base_index {
            continue;
        }
        let index = subpackages.len();
        let binary_name = stanza
            .get("Package")
            .expect("control parser only returns binary Package stanzas");
        let derived_suffix = derive_suffix(source_name, binary_name);
        let mut suffix = derived_suffix.clone();
        if !used_suffixes.insert(suffix.clone()) {
            let mut discriminator = 2;
            while !used_suffixes.insert(format!("{derived_suffix}-{discriminator}")) {
                discriminator += 1;
            }
            suffix = format!("{derived_suffix}-{discriminator}");
            note(
                &mut notes,
                &format!("subpackage[{index}].suffix"),
                format!(
                    "derived suffix {derived_suffix:?} for Debian package {binary_name} collides with an earlier subpackage; changed it to {suffix:?}"
                ),
                Confidence::Unsupported,
            );
        } else {
            note(
                &mut notes,
                &format!("subpackage[{index}].suffix"),
                suffix_note(binary_name, &suffix),
                Confidence::BestEffort,
            );
        }
        let (sub_summary, sub_description) = control::description(stanza);
        confident(
            &mut notes,
            &format!("subpackage[{index}].summary"),
            "copied from the Debian Description short form",
        );
        confident(
            &mut notes,
            &format!("subpackage[{index}].description"),
            "copied from the Debian Description long form",
        );

        let mut sub_deps = DependencySet::default();
        import_dependencies(
            stanza.get("Depends"),
            &format!("subpackage[{index}].deps.depends"),
            &mut sub_deps.depends,
            &mut notes,
        );
        let sub_noarch = stanza.get("Architecture") == Some("all");
        if let Some(architecture) = stanza.get("Architecture") {
            confident(
                &mut notes,
                &format!("subpackage[{index}].noarch"),
                if architecture == "all" {
                    "mapped Debian Architecture: all directly to noarch"
                } else {
                    "mapped a non-all Debian Architecture directly to architecture-specific output"
                },
            );
        }
        note(
            &mut notes,
            &format!("subpackage[{index}].files.paths"),
            FILES_NOTE,
            Confidence::Unsupported,
        );
        let sub_files = placeholder_files(&suffix.to_ascii_uppercase());
        subpackages.push(Subpackage {
            suffix,
            summary: sub_summary,
            description: sub_description,
            noarch: Some(sub_noarch),
            license: None,
            url: None,
            deps: sub_deps,
            files: sub_files,
            scriptlets: Scriptlets::default(),
        });
    }

    Ok(ImportDraft {
        spec: PkgSpecFile {
            package: Package {
                name: base_name.to_string(),
                version,
                release,
                epoch: latest_version.epoch,
                summary,
                license,
                url: homepage,
                group: None,
                noarch,
                description,
                sources: Vec::new(),
                sha256sums: Vec::new(),
                patches: Vec::new(),
                patch_sha256sums: Vec::new(),
                deps: package_deps,
                build: detection.build,
                files,
                scriptlets: Scriptlets::default(),
                changelog,
            },
            subpackages,
        },
        notes,
    })
}

fn read_required(root: &Path, path: &Path, limit: u64) -> Result<String, DebImportError> {
    if !path.is_file() {
        return Err(DebImportError::NotDebianSource {
            path: path.to_path_buf(),
        });
    }
    let resolved = confined_path(root, path)?;
    read_limited(&resolved, path, limit)
}

fn read_optional(root: &Path, path: &Path, limit: u64) -> Result<Option<String>, DebImportError> {
    if !path.exists() {
        return Ok(None);
    }
    let resolved = confined_path(root, path)?;
    read_limited(&resolved, path, limit).map(Some)
}

fn confined_path(root: &Path, path: &Path) -> Result<PathBuf, DebImportError> {
    let root = std::fs::canonicalize(root).map_err(|source| DebImportError::Read {
        path: root.to_path_buf(),
        source,
    })?;
    let resolved = std::fs::canonicalize(path).map_err(|source| DebImportError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    if !resolved.starts_with(&root) {
        return Err(DebImportError::OutsideSource {
            path: path.to_path_buf(),
        });
    }
    Ok(resolved)
}

fn read_limited(
    resolved: &Path,
    display_path: &Path,
    limit: u64,
) -> Result<String, DebImportError> {
    let file = std::fs::File::open(resolved).map_err(|source| DebImportError::Read {
        path: display_path.to_path_buf(),
        source,
    })?;
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| DebImportError::Read {
            path: display_path.to_path_buf(),
            source,
        })?;
    if bytes.len() as u64 > limit {
        return Err(DebImportError::InputTooLarge {
            path: display_path.to_path_buf(),
            limit,
        });
    }
    String::from_utf8(bytes).map_err(|source| DebImportError::Read {
        path: display_path.to_path_buf(),
        source: std::io::Error::new(std::io::ErrorKind::InvalidData, source),
    })
}

fn sanitize_upstream_version(version: &str) -> (String, Confidence, String) {
    if version.contains('-') || version.contains('~') {
        let mut reasons = Vec::new();
        if version.contains('-') {
            reasons.push("contained '-' and was sanitized for RPM");
        }
        if version.contains('~') {
            reasons.push("contains '~'; verify RPM upgrade ordering");
        }
        (
            version.replace('-', "."),
            Confidence::BestEffort,
            format!("Debian upstream version {}", reasons.join("; ")),
        )
    } else {
        (
            version.to_string(),
            Confidence::Confident,
            "copied directly from the Debian upstream version".to_string(),
        )
    }
}

fn import_dependencies(
    raw: Option<&str>,
    field_path: &str,
    output: &mut Vec<String>,
    notes: &mut Vec<ImportNote>,
) {
    for dependency in raw.into_iter().flat_map(deps::split_dependencies) {
        let translated = deps::translate(dependency);
        output.push(translated.value);
        note(notes, field_path, translated.note, translated.confidence);
    }
}

fn import_license(
    root: &Path,
    debian_dir: &Path,
) -> Result<(String, Confidence, String), DebImportError> {
    let copyright_path = debian_dir.join("copyright");
    let Some(copyright) = read_optional(root, &copyright_path, OPTIONAL_METADATA_LIMIT)? else {
        return Ok((
            "LicenseRef-UNKNOWN".to_string(),
            Confidence::Unsupported,
            "debian/copyright is missing; determine the SPDX license manually".to_string(),
        ));
    };
    let licenses = copyright
        .lines()
        .filter_map(|line| line.strip_prefix("License:").map(str::trim))
        .filter(|license| !license.is_empty())
        .collect::<Vec<_>>();
    let Some(first) = licenses.first() else {
        return Ok((
            "LicenseRef-UNKNOWN".to_string(),
            Confidence::Unsupported,
            "debian/copyright has no License field; determine the SPDX license manually"
                .to_string(),
        ));
    };
    let mapped = match *first {
        "GPL-2+" => Some("GPL-2.0-or-later"),
        "GPL-3+" => Some("GPL-3.0-or-later"),
        "LGPL-2.1+" => Some("LGPL-2.1-or-later"),
        "Expat" | "MIT" => Some("MIT"),
        "BSD-3-clause" => Some("BSD-3-Clause"),
        "Apache-2.0" => Some("Apache-2.0"),
        _ => None,
    };
    if licenses.iter().skip(1).any(|license| license != first) {
        return Ok((
            mapped.unwrap_or(first).to_string(),
            Confidence::Unsupported,
            "multiple Debian license stanzas found; reconstruct the complete SPDX expression manually"
                .to_string(),
        ));
    }
    match mapped {
        Some(mapped) => Ok((
            mapped.to_string(),
            Confidence::BestEffort,
            format!("mapped first Debian license identifier {first} to SPDX"),
        )),
        None => Ok((
            first.to_string(),
            Confidence::Unsupported,
            "license string not recognized; verify the SPDX expression manually".to_string(),
        )),
    }
}

fn derive_suffix(source_name: &str, binary_name: &str) -> String {
    binary_name
        .strip_prefix(source_name)
        .and_then(|suffix| suffix.strip_prefix('-'))
        .filter(|suffix| !suffix.is_empty())
        .unwrap_or(binary_name)
        .to_string()
}

fn suffix_note(binary_name: &str, suffix: &str) -> String {
    if suffix == "dev" {
        format!(
            "derived from Debian package {binary_name}; Fedora convention usually uses suffix devel"
        )
    } else {
        format!("derived from Debian package name {binary_name}; verify Fedora naming conventions")
    }
}

fn placeholder_files(scope: &str) -> FilesSpec {
    FilesSpec {
        paths: vec![format!("{FILES_PLACEHOLDER_PREFIX}{scope}_FILES")],
        ..FilesSpec::default()
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
    use crate::lint::lint;
    use crate::parse::parse_rpmspec;

    const CONTROL: &str = r#"Source: sample
Maintainer: Jane Doe <jane@example.org>
Build-Depends: debhelper-compat (= 13), libssl-dev, python3-setuptools
Homepage: https://example.org/sample

Package: sample
Architecture: any
Depends: libssl3 (>= 3.0), ${misc:Depends}
Description: Sample command-line application
 The sample package demonstrates Debian importing.

Package: sample-dev
Architecture: all
Depends: sample (= ${binary:Version}), libsample1
Description: Sample development files
 Headers and static libraries for sample.
"#;

    const CHANGELOG: &str = r#"sample (1:2.0~rc1-3) unstable; urgency=medium

  * Add import coverage.

 -- Jane Doe <jane@example.org>  Sun, 02 Aug 2026 12:34:56 +0200

sample (1.5-1) stable; urgency=low

  * Previous release.

 -- John Doe <john@example.org>  Sat, 01 Aug 2026 10:00:00 +0200
"#;

    fn debian_source_fixture() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        let debian = directory.path().join("debian");
        std::fs::create_dir(&debian).unwrap();
        std::fs::write(debian.join("control"), CONTROL).unwrap();
        std::fs::write(debian.join("changelog"), CHANGELOG).unwrap();
        std::fs::write(
            debian.join("copyright"),
            "Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/\nLicense: MIT\n",
        )
        .unwrap();
        std::fs::write(
            debian.join("rules"),
            "#!/usr/bin/make -f\n%:\n\tdh $@\noverride_dh_auto_install:\n\tdh_auto_install --destdir=debian/tmp\n",
        )
        .unwrap();
        std::fs::write(
            debian.join("sample.install"),
            "usr/bin/sample\nusr/lib/*/libsample.so\n",
        )
        .unwrap();
        std::fs::write(directory.path().join("CMakeLists.txt"), "project(sample)\n").unwrap();
        directory
    }

    #[test]
    fn missing_required_metadata_is_not_a_debian_source_error() {
        let directory = tempfile::tempdir().unwrap();
        let error = import_debian_source(directory.path()).unwrap_err();
        assert!(
            matches!(error, DebImportError::NotDebianSource { path } if path.ends_with("debian/control"))
        );

        std::fs::create_dir(directory.path().join("debian")).unwrap();
        std::fs::write(directory.path().join("debian/control"), CONTROL).unwrap();
        let error = import_debian_source(directory.path()).unwrap_err();
        assert!(
            matches!(error, DebImportError::NotDebianSource { path } if path.ends_with("debian/changelog"))
        );
    }

    #[test]
    fn imports_complete_source_tree_without_using_debian_file_lists() {
        let directory = debian_source_fixture();
        let draft = import_debian_source(directory.path()).unwrap();

        assert_eq!(draft.spec.package.name, "sample");
        assert_eq!(draft.spec.package.epoch, Some(1));
        assert_eq!(draft.spec.package.version, "2.0~rc1");
        assert_eq!(draft.spec.package.release, "3%{?dist}");
        assert_eq!(draft.spec.package.license, "MIT");
        assert_eq!(
            draft.spec.package.build.system,
            crate::model::BuildSystem::Cmake
        );
        assert_eq!(draft.spec.package.changelog.len(), 2);
        assert!(draft.spec.package.sources.is_empty());
        assert!(draft.spec.package.sha256sums.is_empty());
        assert!(draft.notes.iter().any(|note| {
            note.field_path == "package.sources" && note.confidence == Confidence::Unsupported
        }));
        assert!(draft.notes.iter().any(|note| {
            note.field_path == "package.sha256sums" && note.confidence == Confidence::Unsupported
        }));
        assert_eq!(draft.spec.subpackages.len(), 1);
        assert_eq!(draft.spec.subpackages[0].suffix, "dev");
        assert_eq!(draft.spec.subpackages[0].noarch, Some(true));

        for files in std::iter::once(&draft.spec.package.files).chain(
            draft
                .spec
                .subpackages
                .iter()
                .map(|subpackage| &subpackage.files),
        ) {
            assert_eq!(files.paths.len(), 1);
            assert!(files.paths[0].starts_with(FILES_PLACEHOLDER_PREFIX));
            assert!(!files
                .paths
                .iter()
                .any(|path| path.contains("usr/bin/sample")));
        }

        let dependency_count = draft.spec.package.deps.build_depends.len()
            + draft.spec.package.deps.depends.len()
            + draft
                .spec
                .subpackages
                .iter()
                .map(|subpackage| subpackage.deps.depends.len())
                .sum::<usize>();
        let dependency_note_count = draft
            .notes
            .iter()
            .filter(|note| note.field_path.contains(".deps."))
            .count();
        assert_eq!(dependency_note_count, dependency_count);

        let rendered = render_import_draft(&draft).unwrap();
        let reparsed = parse_rpmspec(&rendered).expect("imported TOML should parse cleanly");
        assert_eq!(reparsed.package.name, "sample");
        let lint_result = lint(&reparsed, directory.path(), &rendered);
        assert!(
            !lint_result.has_errors(),
            "imported draft should have warnings but no errors: {:?}",
            lint_result.findings
        );
        assert!(rendered.contains("# TODO: file list not imported"));
        assert!(rendered.contains("override_dh_auto_install"));
        assert!(!rendered.contains("usr/lib/*/libsample.so"));
    }

    #[test]
    fn marks_changelog_entries_with_raw_dates_as_best_effort() {
        let directory = debian_source_fixture();
        let changelog_path = directory.path().join("debian/changelog");
        let changelog = std::fs::read_to_string(&changelog_path)
            .unwrap()
            .replace("Sun, 02 Aug 2026 12:34:56 +0200", "unparseable date");
        std::fs::write(changelog_path, changelog).unwrap();

        let draft = import_debian_source(directory.path()).unwrap();
        assert!(draft.notes.iter().any(|note| {
            note.field_path == "package.changelog[0]" && note.confidence == Confidence::BestEffort
        }));
        assert_eq!(draft.spec.package.changelog[0].date, "unparseable date");

        let rendered = render_import_draft(&draft).unwrap();
        parse_rpmspec(&rendered).expect("rendered changelog draft should parse cleanly");
    }

    #[test]
    fn disambiguates_colliding_subpackage_suffixes() {
        let directory = debian_source_fixture();
        let control_path = directory.path().join("debian/control");
        let mut control = std::fs::read_to_string(&control_path).unwrap();
        control.push_str(
            "\nPackage: dev\nArchitecture: any\nDescription: Alternate development files\n Additional development files.\n",
        );
        std::fs::write(control_path, control).unwrap();

        let draft = import_debian_source(directory.path()).unwrap();
        assert_eq!(draft.spec.subpackages.len(), 2);
        assert_eq!(draft.spec.subpackages[0].suffix, "dev");
        assert_eq!(draft.spec.subpackages[1].suffix, "dev-2");
        assert!(draft.notes.iter().any(|note| {
            note.field_path == "subpackage[1].suffix" && note.confidence == Confidence::Unsupported
        }));

        let rendered = render_import_draft(&draft).unwrap();
        parse_rpmspec(&rendered).expect("rendered collision draft should parse cleanly");
    }

    #[test]
    fn maps_known_license_and_flags_multiple_license_stanzas() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("copyright"), "License: GPL-2+\n").unwrap();
        let (license, confidence, _) = import_license(directory.path(), directory.path()).unwrap();
        assert_eq!(license, "GPL-2.0-or-later");
        assert_eq!(confidence, Confidence::BestEffort);

        std::fs::write(
            directory.path().join("copyright"),
            "Files: *\nLicense: MIT\n\nFiles: vendor/*\nLicense: Apache-2.0\n",
        )
        .unwrap();
        let (license, confidence, note) =
            import_license(directory.path(), directory.path()).unwrap();
        assert_eq!(license, "MIT");
        assert_eq!(confidence, Confidence::Unsupported);
        assert!(note.contains("multiple Debian license stanzas"));
    }

    #[test]
    fn rejects_metadata_over_the_configured_size_limit() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("control");
        std::fs::write(&path, "12345").unwrap();

        assert!(matches!(
            read_required(directory.path(), &path, 4),
            Err(DebImportError::InputTooLarge { limit: 4, .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_required_metadata_symlinks_that_escape_the_source_tree() {
        use std::os::unix::fs::symlink;

        let source = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        let debian = source.path().join("debian");
        std::fs::create_dir(&debian).unwrap();
        let control = debian.join("control");
        symlink(outside.path(), &control).unwrap();

        assert!(matches!(
            read_required(source.path(), &control, CONTROL_LIMIT),
            Err(DebImportError::OutsideSource { path }) if path == control
        ));
    }
}
