use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;

use crate::error::MakerpmError;

const STDERR_TAIL_LINES: usize = 50;

/// Invoke `rpmbuild -ba` against the staged spec in the given topdir.
///
/// Streams stdout/stderr live to the terminal. On failure, returns
/// [`MakerpmError::RpmbuildFailed`] with the exit code and tail of stderr.
pub fn run_rpmbuild(topdir: &Path, spec_name: &str) -> Result<(), MakerpmError> {
    let spec_path = topdir.join("SPECS").join(format!("{spec_name}.spec"));
    let topdir_abs = topdir.canonicalize().map_err(|e| MakerpmError::Io {
        path: topdir.to_path_buf(),
        source: e,
    })?;
    let topdir_str = topdir_abs.to_string_lossy().to_string();

    let mut child = Command::new("rpmbuild")
        .args(["--define", &format!("_topdir {topdir_str}"), "-ba"])
        .arg(&spec_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| MakerpmError::Io {
            path: PathBuf::from("rpmbuild"),
            source: e,
        })?;

    let stdout = child.stdout.take().ok_or_else(|| MakerpmError::Io {
        path: PathBuf::from("rpmbuild stdout"),
        source: std::io::Error::other("failed to capture rpmbuild stdout"),
    })?;
    let stdout_thread = thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            match line {
                Ok(l) => println!("{l}"),
                Err(_) => break,
            }
        }
    });

    let stderr = child.stderr.take().ok_or_else(|| MakerpmError::Io {
        path: PathBuf::from("rpmbuild stderr"),
        source: std::io::Error::other("failed to capture rpmbuild stderr"),
    })?;
    let stderr_thread = thread::spawn(move || {
        let reader = BufReader::new(stderr);
        let mut tail = VecDeque::new();
        for line in reader.lines() {
            match line {
                Ok(l) => {
                    eprintln!("{l}");
                    tail.push_back(l);
                    if tail.len() > STDERR_TAIL_LINES {
                        tail.pop_front();
                    }
                }
                Err(_) => break,
            }
        }
        tail
    });

    let status = child.wait().map_err(|e| MakerpmError::Io {
        path: PathBuf::from("rpmbuild"),
        source: e,
    })?;

    stdout_thread.join().map_err(|_| MakerpmError::Io {
        path: PathBuf::from("rpmbuild stdout"),
        source: std::io::Error::other("rpmbuild stdout reader thread panicked"),
    })?;
    let stderr_tail = stderr_thread.join().map_err(|_| MakerpmError::Io {
        path: PathBuf::from("rpmbuild stderr"),
        source: std::io::Error::other("rpmbuild stderr reader thread panicked"),
    })?;

    if !status.success() {
        return Err(MakerpmError::RpmbuildFailed {
            exit_code: status.code().unwrap_or(-1),
            stderr_tail: stderr_tail.iter().cloned().collect::<Vec<_>>().join("\n"),
        });
    }

    Ok(())
}

/// Collect all `.rpm` and `.src.rpm` files from the build tree into `output_dir`.
///
/// Returns the list of paths to the copied RPM files.
pub fn collect_artifacts(topdir: &Path, output_dir: &Path) -> Result<Vec<PathBuf>, MakerpmError> {
    std::fs::create_dir_all(output_dir).map_err(|e| MakerpmError::Io {
        path: output_dir.to_path_buf(),
        source: e,
    })?;

    let mut artifacts = Vec::new();

    for dir_name in &["RPMS", "SRPMS"] {
        let dir = topdir.join(dir_name);
        if !dir.exists() {
            continue;
        }
        collect_rpms_from_dir(&dir, output_dir, &mut artifacts)?;
    }

    Ok(artifacts)
}

fn collect_rpms_from_dir(
    dir: &Path,
    output_dir: &Path,
    artifacts: &mut Vec<PathBuf>,
) -> Result<(), MakerpmError> {
    let entries = std::fs::read_dir(dir).map_err(|e| MakerpmError::Io {
        path: dir.to_path_buf(),
        source: e,
    })?;

    for entry in entries {
        let entry = entry.map_err(|e| MakerpmError::Io {
            path: dir.to_path_buf(),
            source: e,
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect_rpms_from_dir(&path, output_dir, artifacts)?;
        } else if path.extension().is_some_and(|ext| ext == "rpm") {
            let filename = path.file_name().ok_or_else(|| MakerpmError::Io {
                path: path.clone(),
                source: std::io::Error::other("RPM artifact has no filename"),
            })?;
            let dest = output_dir.join(filename);
            std::fs::copy(&path, &dest).map_err(|e| MakerpmError::Io {
                path: dest.clone(),
                source: e,
            })?;
            artifacts.push(dest);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn collect_artifacts_empty_tree() {
        let tmp = tempdir().unwrap();
        let topdir = tmp.path().join("topdir");
        let output = tmp.path().join("output");
        std::fs::create_dir_all(topdir.join("RPMS")).unwrap();
        std::fs::create_dir_all(topdir.join("SRPMS")).unwrap();

        let artifacts = collect_artifacts(&topdir, &output).unwrap();
        assert!(artifacts.is_empty());
    }

    #[test]
    fn collect_artifacts_finds_rpms() {
        let tmp = tempdir().unwrap();
        let topdir = tmp.path().join("topdir");
        let output = tmp.path().join("output");

        std::fs::create_dir_all(topdir.join("RPMS/x86_64")).unwrap();
        std::fs::write(
            topdir.join("RPMS/x86_64/test-1.0-1.x86_64.rpm"),
            b"fake-rpm",
        )
        .unwrap();
        std::fs::create_dir_all(topdir.join("SRPMS")).unwrap();
        std::fs::write(topdir.join("SRPMS/test-1.0-1.src.rpm"), b"fake-srpm").unwrap();

        let artifacts = collect_artifacts(&topdir, &output).unwrap();
        assert_eq!(artifacts.len(), 2);
        assert!(output.join("test-1.0-1.x86_64.rpm").exists());
        assert!(output.join("test-1.0-1.src.rpm").exists());
    }
}
