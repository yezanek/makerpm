use std::collections::BTreeMap;
use std::fmt;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use thiserror::Error;
use toml_edit::{DocumentMut, Table};

use crate::model::PkgSpecFile;

pub mod arch;
pub mod deb;

#[derive(Debug)]
pub struct ImportDraft {
    pub spec: PkgSpecFile,
    pub notes: Vec<ImportNote>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportNote {
    pub field_path: String,
    pub note: String,
    pub confidence: Confidence,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Confidence {
    Confident,
    BestEffort,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ImportSummary {
    pub confident: usize,
    pub best_effort: usize,
    pub unsupported: usize,
}

impl ImportSummary {
    pub fn from_draft(draft: &ImportDraft) -> Self {
        let mut summary = Self::default();
        for note in &draft.notes {
            match note.confidence {
                Confidence::Confident => summary.confident += 1,
                Confidence::BestEffort => summary.best_effort += 1,
                Confidence::Unsupported => summary.unsupported += 1,
            }
        }
        summary
    }
}

impl fmt::Display for ImportSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Confident: {}, BestEffort: {}, Unsupported: {}",
            self.confident, self.best_effort, self.unsupported
        )
    }
}

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("failed to serialize import draft: {0}")]
    Serialize(#[from] toml_edit::ser::Error),

    #[error("failed to build editable import document: {0}")]
    EditableDocument(#[from] toml_edit::TomlError),

    #[error("import note references an unknown field: {0}")]
    UnknownFieldPath(String),

    #[error("invalid import note field path: {0}")]
    InvalidFieldPath(String),

    #[error("refusing to overwrite existing file {0}; pass --force to overwrite it")]
    OutputExists(PathBuf),

    #[error("failed to write import draft to {path}")]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("failed to print import summary")]
    Report(#[source] io::Error),
}

pub fn render_import_draft(draft: &ImportDraft) -> Result<String, ImportError> {
    let serialized = toml_edit::ser::to_string_pretty(&draft.spec)?;
    let mut document = serialized.parse::<DocumentMut>()?;
    let mut annotations = BTreeMap::<&str, Vec<&str>>::new();

    for note in &draft.notes {
        if note.confidence != Confidence::Confident {
            annotations
                .entry(&note.field_path)
                .or_default()
                .push(&note.note);
        }
    }

    for (field_path, notes) in annotations {
        let comment = notes
            .into_iter()
            .flat_map(|note| note.split(['\r', '\n']))
            .map(|line| format!("# TODO: {}\n", sanitize_comment_line(line)))
            .collect::<String>();
        annotate_field(&mut document, field_path, &comment)?;
    }

    let rendered = document.to_string();
    rendered.parse::<DocumentMut>()?;
    Ok(rendered)
}

fn sanitize_comment_line(line: &str) -> String {
    let mut sanitized = String::with_capacity(line.len());
    for character in line.chars() {
        if character.is_control() && character != '\t' {
            sanitized.extend(character.escape_default());
        } else {
            sanitized.push(character);
        }
    }
    sanitized
}

pub fn write_import_draft(
    draft: &ImportDraft,
    output: &Path,
    force: bool,
) -> Result<ImportSummary, ImportError> {
    let rendered = render_import_draft(draft)?;
    let mut options = OpenOptions::new();
    options.write(true);
    if force {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }

    let mut file = options.open(output).map_err(|source| {
        if !force && source.kind() == io::ErrorKind::AlreadyExists {
            ImportError::OutputExists(output.to_path_buf())
        } else {
            ImportError::Write {
                path: output.to_path_buf(),
                source,
            }
        }
    })?;
    file.write_all(rendered.as_bytes())
        .map_err(|source| ImportError::Write {
            path: output.to_path_buf(),
            source,
        })?;
    file.flush().map_err(|source| ImportError::Write {
        path: output.to_path_buf(),
        source,
    })?;

    Ok(ImportSummary::from_draft(draft))
}

pub fn write_import_draft_and_report(
    draft: &ImportDraft,
    output: &Path,
    force: bool,
    report: &mut impl Write,
) -> Result<ImportSummary, ImportError> {
    let summary = write_import_draft(draft, output, force)?;
    writeln!(report, "Import summary — {summary}").map_err(ImportError::Report)?;
    writeln!(report, "Next: run makerpm lint {}", output.display()).map_err(ImportError::Report)?;
    Ok(summary)
}

#[derive(Debug)]
struct PathSegment<'a> {
    key: &'a str,
    index: Option<usize>,
}

fn annotate_field(
    document: &mut DocumentMut,
    field_path: &str,
    comment: &str,
) -> Result<(), ImportError> {
    let segments = parse_field_path(field_path)?;
    annotate_table(document.as_table_mut(), &segments, field_path, comment)
}

fn parse_field_path(field_path: &str) -> Result<Vec<PathSegment<'_>>, ImportError> {
    if field_path.is_empty() {
        return Err(ImportError::InvalidFieldPath(field_path.to_string()));
    }

    field_path
        .split('.')
        .map(|part| {
            if part.is_empty() {
                return Err(ImportError::InvalidFieldPath(field_path.to_string()));
            }
            if let Some((key, raw_index)) = part.split_once('[') {
                let Some(raw_index) = raw_index.strip_suffix(']') else {
                    return Err(ImportError::InvalidFieldPath(field_path.to_string()));
                };
                if key.is_empty() || raw_index.is_empty() || raw_index.contains(['[', ']']) {
                    return Err(ImportError::InvalidFieldPath(field_path.to_string()));
                }
                let index = raw_index
                    .parse::<usize>()
                    .map_err(|_| ImportError::InvalidFieldPath(field_path.to_string()))?;
                Ok(PathSegment {
                    key,
                    index: Some(index),
                })
            } else if part.contains(']') {
                Err(ImportError::InvalidFieldPath(field_path.to_string()))
            } else {
                Ok(PathSegment {
                    key: part,
                    index: None,
                })
            }
        })
        .collect()
}

fn set_prefix_preserving(decor: &mut toml_edit::Decor, comment: &str) {
    let existing = decor
        .prefix()
        .and_then(|raw| raw.as_str())
        .unwrap_or("");
    let combined = if existing.is_empty() {
        comment.to_string()
    } else {
        format!("{comment}{existing}")
    };
    decor.set_prefix(&combined);
}

fn annotate_table(
    table: &mut Table,
    segments: &[PathSegment<'_>],
    field_path: &str,
    comment: &str,
) -> Result<(), ImportError> {
    let Some((segment, remaining)) = segments.split_first() else {
        return Err(ImportError::InvalidFieldPath(field_path.to_string()));
    };

    if let Some(index) = segment.index {
        let child = table
            .get_mut(segment.key)
            .and_then(toml_edit::Item::as_array_of_tables_mut)
            .and_then(|array| array.get_mut(index))
            .ok_or_else(|| ImportError::UnknownFieldPath(field_path.to_string()))?;
        if remaining.is_empty() {
            set_prefix_preserving(child.decor_mut(), comment);
            return Ok(());
        }
        return annotate_table(child, remaining, field_path, comment);
    }

    if remaining.is_empty() {
        if let Some(child) = table
            .get_mut(segment.key)
            .and_then(toml_edit::Item::as_table_mut)
        {
            set_prefix_preserving(child.decor_mut(), comment);
            return Ok(());
        }
        let mut key = table
            .key_mut(segment.key)
            .ok_or_else(|| ImportError::UnknownFieldPath(field_path.to_string()))?;
        set_prefix_preserving(key.leaf_decor_mut(), comment);
        return Ok(());
    }

    let child = table
        .get_mut(segment.key)
        .and_then(toml_edit::Item::as_table_mut)
        .ok_or_else(|| ImportError::UnknownFieldPath(field_path.to_string()))?;
    annotate_table(child, remaining, field_path, comment)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        BuildSpec, BuildSteps, BuildSystem, ChangelogEntry, DependencySet, FilesSpec, Package,
        Scriptlets, Subpackage,
    };
    use crate::parse::parse_rpmspec;

    fn hand_built_draft() -> ImportDraft {
        ImportDraft {
            spec: PkgSpecFile {
                package: Package {
                    name: "imported-package".to_string(),
                    version: "1.2.3".to_string(),
                    release: "1%{?dist}".to_string(),
                    epoch: None,
                    summary: "An imported package".to_string(),
                    license: "MIT".to_string(),
                    url: Some("https://example.org/imported-package".to_string()),
                    group: None,
                    noarch: false,
                    description: "A package assembled as an import draft.".to_string(),
                    sources: vec!["https://example.org/source.tar.gz".to_string()],
                    sha256sums: vec!["SKIP".to_string()],
                    patches: Vec::new(),
                    patch_sha256sums: Vec::new(),
                    deps: DependencySet::default(),
                    build: BuildSpec {
                        system: BuildSystem::Cmake,
                        extra_build_args: Vec::new(),
                        extra_install_args: Vec::new(),
                        run_tests: None,
                        steps: BuildSteps::default(),
                    },
                    files: FilesSpec {
                        paths: vec!["%{_bindir}/imported-package".to_string()],
                        ..FilesSpec::default()
                    },
                    scriptlets: Scriptlets::default(),
                    changelog: vec![ChangelogEntry {
                        version: "1.2.3-1".to_string(),
                        date: "2026-08-02".to_string(),
                        packager: "Importer <importer@example.org>".to_string(),
                        entries: vec!["Initial import".to_string()],
                    }],
                },
                subpackages: vec![Subpackage {
                    suffix: "devel".to_string(),
                    summary: "Development files".to_string(),
                    description: "Development files need review.".to_string(),
                    noarch: None,
                    license: None,
                    url: None,
                    deps: DependencySet::default(),
                    files: FilesSpec {
                        paths: vec!["%{_includedir}/imported-package.h".to_string()],
                        ..FilesSpec::default()
                    },
                    scriptlets: Scriptlets::default(),
                }],
            },
            notes: vec![
                ImportNote {
                    field_path: "package.name".to_string(),
                    note: "copied directly".to_string(),
                    confidence: Confidence::Confident,
                },
                ImportNote {
                    field_path: "package.build.system".to_string(),
                    note: "inferred from build metadata".to_string(),
                    confidence: Confidence::BestEffort,
                },
                ImportNote {
                    field_path: "subpackage[0].description".to_string(),
                    note: "replace the generated placeholder".to_string(),
                    confidence: Confidence::Unsupported,
                },
            ],
        }
    }

    #[test]
    fn writer_round_trips_and_annotates_only_non_confident_fields() {
        let rendered = render_import_draft(&hand_built_draft()).unwrap();

        let parsed = parse_rpmspec(&rendered).expect("written draft should parse as an RPMSPEC");
        assert_eq!(parsed.package.name, "imported-package");
        assert_eq!(parsed.package.build.system, BuildSystem::Cmake);
        assert_eq!(parsed.subpackages[0].suffix, "devel");

        assert!(rendered.contains("# TODO: inferred from build metadata\nsystem = \"cmake\""));
        assert!(rendered.contains(
            "# TODO: replace the generated placeholder\ndescription = \"Development files need review.\""
        ));
        assert!(!rendered.contains("# TODO: copied directly"));
        assert_eq!(rendered.matches("# TODO:").count(), 2);
    }

    #[test]
    fn writer_protects_existing_output_unless_forced() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("package.toml");
        std::fs::write(&output, "original").unwrap();

        let error = write_import_draft(&hand_built_draft(), &output, false).unwrap_err();
        assert!(matches!(error, ImportError::OutputExists(path) if path == output));
        assert_eq!(std::fs::read_to_string(&output).unwrap(), "original");

        write_import_draft(&hand_built_draft(), &output, true).unwrap();
        let rendered = std::fs::read_to_string(&output).unwrap();
        parse_rpmspec(&rendered).expect("forced output should contain the replacement draft");
    }

    #[test]
    fn report_counts_each_confidence_and_recommends_lint() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("package.toml");
        let mut report = Vec::new();

        let summary =
            write_import_draft_and_report(&hand_built_draft(), &output, false, &mut report)
                .unwrap();

        assert_eq!(
            summary,
            ImportSummary {
                confident: 1,
                best_effort: 1,
                unsupported: 1,
            }
        );
        let report = String::from_utf8(report).unwrap();
        assert!(report.contains("Confident: 1, BestEffort: 1, Unsupported: 1"));
        assert!(report.contains(&format!("makerpm lint {}", output.display())));
    }

    #[test]
    fn writer_rejects_notes_for_unknown_fields() {
        let mut draft = hand_built_draft();
        draft.notes.push(ImportNote {
            field_path: "package.missing".to_string(),
            note: "cannot be placed".to_string(),
            confidence: Confidence::Unsupported,
        });

        assert!(matches!(
            render_import_draft(&draft),
            Err(ImportError::UnknownFieldPath(path)) if path == "package.missing"
        ));
    }

    #[test]
    fn writer_places_attacker_controlled_table_notes_outside_table_headers() {
        let mut draft = hand_built_draft();
        draft.notes.push(ImportNote {
            field_path: "package.scriptlets".to_string(),
            note: "referenced evil.install\n[injected]\rowned = true\0".to_string(),
            confidence: Confidence::Unsupported,
        });

        let rendered = render_import_draft(&draft).unwrap();
        let parsed = parse_rpmspec(&rendered).expect("decorated output must remain valid TOML");

        assert!(rendered.contains("# TODO: referenced evil.install"));
        assert!(rendered.contains("# TODO: [injected]"));
        assert!(rendered.contains("# TODO: owned = true\\u{0}"));
        let scriptlets_header = rendered.find("[package.scriptlets]").expect("scriptlets table");
        assert!(rendered[..scriptlets_header].contains("# TODO: referenced evil.install"));
        assert_eq!(parsed.package.name, "imported-package");
        assert!(!rendered.contains("\n[injected]\n"));
    }

    #[test]
    fn writer_escapes_toml_metacharacters_in_imported_values() {
        let mut draft = hand_built_draft();
        draft.spec.package.name = "safe\"\n[injected]\nowned = true".to_string();
        draft.spec.package.description = "''' \"\"\" # [table] \\".to_string();

        let rendered = render_import_draft(&draft).unwrap();
        let parsed = parse_rpmspec(&rendered).expect("serialized values must not escape strings");

        assert_eq!(parsed.package.name, draft.spec.package.name);
        assert_eq!(parsed.package.description, draft.spec.package.description);
    }
}
