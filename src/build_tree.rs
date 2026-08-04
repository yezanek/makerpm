use std::path::{Path, PathBuf};

use crate::error::MakerpmError;
use crate::fetch::ResolvedSource;
use crate::model::PkgSpecFile;

const RPM_DIRS: &[&str] = &["BUILD", "BUILDROOT", "RPMS", "SOURCES", "SPECS", "SRPMS"];

fn validate_single_component(name: &str, context: &str) -> Result<(), MakerpmError> {
    if name.is_empty() {
        return Err(MakerpmError::Io {
            path: PathBuf::new(),
            source: std::io::Error::other(format!("{context} must not be empty")),
        });
    }
    let path = Path::new(name);
    let mut components = path.components();
    match components.next() {
        None => {
            return Err(MakerpmError::Io {
                path: PathBuf::new(),
                source: std::io::Error::other(format!("{context} is empty: \"{name}\"")),
            });
        }
        Some(std::path::Component::Normal(_)) => {}
        Some(_) => {
            return Err(MakerpmError::Io {
                path: PathBuf::new(),
                source: std::io::Error::other(format!(
                    "{context} contains path traversal or is absolute: \"{name}\""
                )),
            });
        }
    }
    if components.next().is_some() {
        return Err(MakerpmError::Io {
            path: PathBuf::new(),
            source: std::io::Error::other(format!(
                "{context} contains path separators: \"{name}\""
            )),
        });
    }
    Ok(())
}

/// Set up the rpmbuild tree layout and stage sources + spec.
///
/// Creates `.makerpm/` next to the RPMSPEC.toml as the `_topdir`.
/// Returns the path to the topdir.
pub fn setup_build_tree(
    spec: &PkgSpecFile,
    toml_dir: &Path,
    resolved_sources: &[ResolvedSource],
    rendered_spec: &str,
) -> Result<PathBuf, MakerpmError> {
    let topdir = toml_dir.join(".makerpm");

    for dir in RPM_DIRS {
        let path = topdir.join(dir);
        std::fs::create_dir_all(&path).map_err(|e| MakerpmError::Io { path, source: e })?;
    }

    for source in resolved_sources {
        validate_single_component(&source.filename, "source filename")?;
        let dest = topdir.join("SOURCES").join(&source.filename);
        std::fs::copy(&source.local_path, &dest).map_err(|e| MakerpmError::Io {
            path: dest,
            source: e,
        })?;
    }

    validate_single_component(&spec.package.name, "package name")?;
    let spec_filename = format!("{}.spec", spec.package.name);
    let spec_path = topdir.join("SPECS").join(&spec_filename);
    std::fs::write(&spec_path, rendered_spec).map_err(|e| MakerpmError::Io {
        path: spec_path,
        source: e,
    })?;

    Ok(topdir)
}

/// Remove the `.makerpm/` build tree.
pub fn clean_build_tree(toml_dir: &Path) -> Result<(), MakerpmError> {
    let topdir = toml_dir.join(".makerpm");
    if topdir.exists() {
        std::fs::remove_dir_all(&topdir).map_err(|e| MakerpmError::Io {
            path: topdir,
            source: e,
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use crate::spec_gen;

    fn make_spec() -> PkgSpecFile {
        PkgSpecFile {
            package: Package {
                name: "test-pkg".to_string(),
                version: "1.0".to_string(),
                release: "1".to_string(),
                epoch: None,
                summary: "test".to_string(),
                license: "MIT".to_string(),
                url: None,
                group: None,
                noarch: false,
                description: "test".to_string(),
                sources: vec!["data.txt".to_string()],
                sha256sums: vec![],
                patches: vec![],
                patch_sha256sums: vec![],
                deps: DependencySet::default(),
                build: BuildSpec::default(),
                files: FilesSpec::default(),
                scriptlets: Scriptlets::default(),
                changelog: vec![],
            },
            subpackages: vec![],
        }
    }

    #[test]
    fn creates_topdir_and_subdirs() {
        let tmp = tempfile::tempdir().unwrap();
        let spec = make_spec();
        let resolved = vec![ResolvedSource {
            local_path: tmp.path().join("data.txt"),
            filename: "data.txt".to_string(),
            was_download: false,
        }];
        std::fs::write(tmp.path().join("data.txt"), b"hello").unwrap();

        let topdir = setup_build_tree(&spec, tmp.path(), &resolved, "Name: test-pkg\n").unwrap();

        assert!(topdir.exists());
        for dir in RPM_DIRS {
            assert!(topdir.join(dir).exists(), "missing {dir}");
        }
    }

    #[test]
    fn copies_sources_to_sources_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let spec = make_spec();
        let resolved = vec![ResolvedSource {
            local_path: tmp.path().join("data.txt"),
            filename: "data.txt".to_string(),
            was_download: false,
        }];
        std::fs::write(tmp.path().join("data.txt"), b"content").unwrap();

        let topdir = setup_build_tree(&spec, tmp.path(), &resolved, "").unwrap();
        let dest = topdir.join("SOURCES").join("data.txt");
        assert!(dest.exists());
        assert_eq!(std::fs::read(&dest).unwrap(), b"content");
    }

    #[test]
    fn writes_spec_file() {
        let tmp = tempfile::tempdir().unwrap();
        let spec = make_spec();
        let rendered = spec_gen::render(&spec, &[]).unwrap();

        let topdir = setup_build_tree(&spec, tmp.path(), &[], &rendered).unwrap();
        let spec_path = topdir.join("SPECS").join("test-pkg.spec");
        assert!(spec_path.exists());
        let content = std::fs::read_to_string(&spec_path).unwrap();
        assert!(content.contains("Name:           test-pkg"));
    }

    #[test]
    fn clean_removes_topdir() {
        let tmp = tempfile::tempdir().unwrap();
        let topdir = tmp.path().join(".makerpm");
        std::fs::create_dir_all(&topdir).unwrap();
        assert!(topdir.exists());

        clean_build_tree(tmp.path()).unwrap();
        assert!(!topdir.exists());
    }

    #[test]
    fn clean_on_missing_dir_is_ok() {
        let tmp = tempfile::tempdir().unwrap();
        clean_build_tree(tmp.path()).unwrap();
    }
}
