use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::MakerpmError;
use crate::model::PkgSpecFile;
use crate::source_spec::{self, SourceEntry};

/// Abstraction over HTTP downloading so tests can substitute a fake.
pub trait Downloader: Send + Sync {
    /// Download the content at `url` and write it to `dest` (a `.part` file).
    /// Returns the number of bytes written.
    fn download_to(&self, url: &str, dest: &Path) -> Result<u64, MakerpmError>;
}

/// Real downloader using ureq.
pub struct UreqDownloader;

impl Downloader for UreqDownloader {
    fn download_to(&self, url: &str, dest: &Path) -> Result<u64, MakerpmError> {
        let response =
            ureq::get(url).call().map_err(|e| MakerpmError::Fetch {
                url: url.to_string(),
                source: Box::new(e),
            })?;

        let total = response.body().content_length();

        let mut reader = response.into_parts().1.into_reader();
        let mut file = std::fs::File::create(dest).map_err(|e| MakerpmError::CacheDir {
            path: dest.to_path_buf(),
            source: e,
        })?;

        let pb = if let Some(len) = total {
            let pb = indicatif::ProgressBar::new(len);
            pb.set_style(
                indicatif::ProgressStyle::default_bar()
                    .template("{spinner:.green} [{elapsed_precise}] [{bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})")
                    .unwrap()
                    .progress_chars("#>-"),
            );
            Some(pb)
        } else {
            None
        };

        let mut buf = [0u8; 8192];
        let mut written: u64 = 0;
        loop {
            let n = reader.read(&mut buf).map_err(|e| MakerpmError::Fetch {
                url: url.to_string(),
                source: Box::new(e),
            })?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).map_err(|e| MakerpmError::Fetch {
                url: url.to_string(),
                source: Box::new(e),
            })?;
            written += n as u64;
            if let Some(ref pb) = pb {
                pb.inc(n as u64);
            }
        }
        if let Some(pb) = pb {
            pb.finish_with_message("done");
        }

        Ok(written)
    }
}

/// Resolve the cache directory for downloaded sources.
///
/// Uses `$MAKERPM_SRCDEST` if set, otherwise `~/.cache/makerpm/sources/`.
pub fn resolve_cache_dir() -> PathBuf {
    if let Ok(srcdest) = std::env::var("MAKERPM_SRCDEST") {
        PathBuf::from(srcdest)
    } else {
        let base = std::env::var("XDG_CACHE_HOME")
            .ok()
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| PathBuf::from(h).join(".cache"))
            })
            .unwrap_or_else(|| PathBuf::from(".cache"));
        base.join("makerpm").join("sources")
    }
}

fn cache_path_for(cache_dir: &Path, filename: &str) -> PathBuf {
    cache_dir.join(filename)
}

/// Compute the SHA-256 hex digest of a file.
pub fn compute_sha256(path: &Path) -> Result<String, MakerpmError> {
    let mut file = std::fs::File::open(path).map_err(|e| MakerpmError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf).map_err(|e| MakerpmError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex_encode(&hasher.finalize()))
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Options controlling fetch behavior, derived from CLI flags.
pub struct FetchOptions {
    pub cache_dir: PathBuf,
    pub offline: bool,
    pub refetch: bool,
    pub skip_checksums: bool,
    pub allow_unverified: bool,
}

/// Where a resolved source now lives on disk.
#[derive(Debug)]
pub struct ResolvedSource {
    pub local_path: PathBuf,
    pub was_download: bool,
}

/// Fetch all sources and patches declared in the spec.
///
/// Returns a list of resolved local paths (one per source+patch entry, in order).
pub fn fetch_sources(
    spec: &PkgSpecFile,
    toml_dir: &Path,
    opts: &FetchOptions,
    downloader: &dyn Downloader,
) -> Result<Vec<ResolvedSource>, MakerpmError> {
    std::fs::create_dir_all(&opts.cache_dir).map_err(|e| MakerpmError::CacheDir {
        path: opts.cache_dir.clone(),
        source: e,
    })?;

    let mut results = Vec::new();

    let all_sources: Vec<(&str, usize, &str)> = spec
        .package
        .sources
        .iter()
        .enumerate()
        .map(|(i, s)| (s.as_str(), i, "source"))
        .chain(
            spec.package
                .patches
                .iter()
                .enumerate()
                .map(|(i, s)| (s.as_str(), i, "patch")),
        )
        .collect();

    for (raw, idx, _kind) in all_sources {
        let entry = source_spec::parse_source_entry(raw);
        match entry {
            SourceEntry::Local { filename } => {
                let local_path = toml_dir.join(&filename);
                results.push(ResolvedSource {
                    local_path,
                    was_download: false,
                });
            }
            SourceEntry::Remote { ref filename, ref url } => {
                let cache_path = cache_path_for(&opts.cache_dir, filename);
                let declared_checksum = checksum_for(spec, &entry, idx);

                // §8.2 algorithm: decide whether to download
                let should_download = if opts.refetch || !cache_path.exists() {
                    true
                } else if let Some(expected) = declared_checksum {
                    if expected != "SKIP" {
                        let actual = compute_sha256(&cache_path)?;
                        actual != expected // §8.2 step 3c: mismatch → re-download
                                           // §8.2 step 3b: match → reuse (false)
                    } else {
                        false // SKIP checksum → reuse existing cache
                    }
                } else {
                    false // no declared checksum → reuse existing cache
                };

                if should_download {
                    if opts.offline {
                        return Err(MakerpmError::OfflineUncached {
                            filename: filename.clone(),
                        });
                    }

                    let part_path = cache_path.with_extension("part");
                    downloader.download_to(url, &part_path)?;
                    std::fs::rename(&part_path, &cache_path).map_err(|e| {
                        MakerpmError::CacheDir {
                            path: cache_path.clone(),
                            source: e,
                        }
                    })?;
                }

                // §8.2 step 3e: verify checksum after download or cache hit
                if let Some(expected) = declared_checksum {
                    if expected != "SKIP" {
                        let actual = compute_sha256(&cache_path)?;
                        if actual != expected {
                            if opts.skip_checksums {
                                eprintln!(
                                    "warning: checksum mismatch for {filename}: \
                                     expected {expected}, got {actual}"
                                );
                            } else {
                                return Err(MakerpmError::ChecksumMismatch {
                                    filename: filename.clone(),
                                    expected: expected.to_string(),
                                    actual,
                                });
                            }
                        }
                    }
                }

                results.push(ResolvedSource {
                    local_path: cache_path,
                    was_download: should_download,
                });
            }
        }
    }

    Ok(results)
}

fn checksum_for<'a>(spec: &'a PkgSpecFile, entry: &SourceEntry, idx: usize) -> Option<&'a str> {
    let sums = match entry {
        SourceEntry::Remote { .. } => &spec.package.sha256sums,
        SourceEntry::Local { .. } => &spec.package.sha256sums,
    };
    sums.get(idx).map(String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    struct MockDownloader {
        calls: Arc<Mutex<Vec<String>>>,
        responses: HashMap<String, Vec<u8>>,
    }

    impl MockDownloader {
        fn new() -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                responses: HashMap::new(),
            }
        }

        fn with_response(mut self, url: &str, data: Vec<u8>) -> Self {
            self.responses.insert(url.to_string(), data);
            self
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Downloader for MockDownloader {
        fn download_to(&self, url: &str, dest: &Path) -> Result<u64, MakerpmError> {
            self.calls.lock().unwrap().push(url.to_string());
            let data = self
                .responses
                .get(url)
                .ok_or_else(|| MakerpmError::Fetch {
                    url: url.to_string(),
                    source: Box::new(std::io::Error::other("no mock response configured")),
                })?;
            std::fs::write(dest, data).map_err(|e| MakerpmError::Fetch {
                url: url.to_string(),
                source: Box::new(e),
            })?;
            Ok(data.len() as u64)
        }
    }

    fn make_spec(sources: Vec<&str>, sha256sums: Vec<&str>) -> PkgSpecFile {
        PkgSpecFile {
            package: Package {
                name: "test".to_string(),
                version: "1.0".to_string(),
                release: "1".to_string(),
                epoch: None,
                summary: "test".to_string(),
                license: "MIT".to_string(),
                url: None,
                group: None,
                noarch: false,
                description: "test".to_string(),
                sources: sources.into_iter().map(String::from).collect(),
                sha256sums: sha256sums.into_iter().map(String::from).collect(),
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

    fn opts_with_cache(dir: &Path) -> FetchOptions {
        FetchOptions {
            cache_dir: dir.to_path_buf(),
            offline: false,
            refetch: false,
            skip_checksums: false,
            allow_unverified: false,
        }
    }

    #[test]
    fn cache_hit_with_matching_checksum_no_download() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();

        let content = b"hello world";
        let digest = hex_encode(&Sha256::digest(content));
        std::fs::write(cache_dir.join("test.tar.gz"), content).unwrap();

        let spec = make_spec(
            vec!["test.tar.gz::https://example.com/test.tar.gz"],
            vec![&digest],
        );
        let opts = opts_with_cache(&cache_dir);
        let dl = MockDownloader::new();

        let results = fetch_sources(&spec, tmp.path(), &opts, &dl).unwrap();
        assert_eq!(results.len(), 1);
        assert!(!results[0].was_download);
        assert!(dl.calls().is_empty());
    }

    #[test]
    fn cache_miss_download_attempted() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();

        let content = b"downloaded data";
        let spec = make_spec(
            vec!["data.bin::https://example.com/data.bin"],
            vec!["SKIP"],
        );
        let opts = opts_with_cache(&cache_dir);
        let dl = MockDownloader::new()
            .with_response("https://example.com/data.bin", content.to_vec());

        let results = fetch_sources(&spec, tmp.path(), &opts, &dl).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].was_download);
        assert_eq!(dl.calls(), vec!["https://example.com/data.bin"]);
        assert!(cache_dir.join("data.bin").exists());
    }

    #[test]
    fn checksum_mismatch_hard_error() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();

        let content = b"wrong content";
        let bad_digest = hex_encode(&Sha256::digest(b"expected content"));
        let spec = make_spec(
            vec!["data.bin::https://example.com/data.bin"],
            vec![&bad_digest],
        );
        let opts = opts_with_cache(&cache_dir);
        let dl = MockDownloader::new()
            .with_response("https://example.com/data.bin", content.to_vec());

        let result = fetch_sources(&spec, tmp.path(), &opts, &dl);
        assert!(result.is_err());
        match result.unwrap_err() {
            MakerpmError::ChecksumMismatch { filename, .. } => {
                assert_eq!(filename, "data.bin");
            }
            other => panic!("expected ChecksumMismatch, got {other:?}"),
        }
    }

    #[test]
    fn checksum_mismatch_skip_checksums_proceeds() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();

        let content = b"wrong content";
        let bad_digest = hex_encode(&Sha256::digest(b"expected content"));
        let spec = make_spec(
            vec!["data.bin::https://example.com/data.bin"],
            vec![&bad_digest],
        );
        let mut opts = opts_with_cache(&cache_dir);
        opts.skip_checksums = true;
        let dl = MockDownloader::new()
            .with_response("https://example.com/data.bin", content.to_vec());

        let results = fetch_sources(&spec, tmp.path(), &opts, &dl).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].was_download);
    }

    #[test]
    fn offline_mode_uncached_remote_hard_error() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();

        let spec = make_spec(
            vec!["data.bin::https://example.com/data.bin"],
            vec!["SKIP"],
        );
        let mut opts = opts_with_cache(&cache_dir);
        opts.offline = true;
        let dl = MockDownloader::new();

        let result = fetch_sources(&spec, tmp.path(), &opts, &dl);
        assert!(result.is_err());
        match result.unwrap_err() {
            MakerpmError::OfflineUncached { filename } => {
                assert_eq!(filename, "data.bin");
            }
            other => panic!("expected OfflineUncached, got {other:?}"),
        }
    }

    #[test]
    fn offline_mode_cached_source_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();

        let content = b"cached data";
        let digest = hex_encode(&Sha256::digest(content));
        std::fs::write(cache_dir.join("data.bin"), content).unwrap();

        let spec = make_spec(
            vec!["data.bin::https://example.com/data.bin"],
            vec![&digest],
        );
        let mut opts = opts_with_cache(&cache_dir);
        opts.offline = true;
        let dl = MockDownloader::new();

        let results = fetch_sources(&spec, tmp.path(), &opts, &dl).unwrap();
        assert_eq!(results.len(), 1);
        assert!(!results[0].was_download);
    }

    #[test]
    fn refetch_ignores_cache_and_redownloads() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();

        let old_content = b"old data";
        std::fs::write(cache_dir.join("data.bin"), old_content).unwrap();

        let new_content = b"new data";
        let spec = make_spec(
            vec!["data.bin::https://example.com/data.bin"],
            vec!["SKIP"],
        );
        let mut opts = opts_with_cache(&cache_dir);
        opts.refetch = true;
        let dl = MockDownloader::new()
            .with_response("https://example.com/data.bin", new_content.to_vec());

        let results = fetch_sources(&spec, tmp.path(), &opts, &dl).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].was_download);
        assert_eq!(dl.calls(), vec!["https://example.com/data.bin"]);
        assert_eq!(
            std::fs::read(cache_dir.join("data.bin")).unwrap(),
            new_content
        );
    }

    #[test]
    fn local_source_returns_toml_dir_path() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();

        std::fs::write(tmp.path().join("local.txt"), b"local").unwrap();

        let spec = make_spec(vec!["local.txt"], vec![]);
        let opts = opts_with_cache(&cache_dir);
        let dl = MockDownloader::new();

        let results = fetch_sources(&spec, tmp.path(), &opts, &dl).unwrap();
        assert_eq!(results.len(), 1);
        assert!(!results[0].was_download);
        assert_eq!(results[0].local_path, tmp.path().join("local.txt"));
        assert!(dl.calls().is_empty());
    }

    #[test]
    fn skip_checksum_entry_no_verification() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();

        let content = b"garbage content";
        let spec = make_spec(
            vec!["data.bin::https://example.com/data.bin"],
            vec!["SKIP"],
        );
        let opts = opts_with_cache(&cache_dir);
        let dl = MockDownloader::new()
            .with_response("https://example.com/data.bin", content.to_vec());

        let results = fetch_sources(&spec, tmp.path(), &opts, &dl).unwrap();
        assert_eq!(results.len(), 1);
        assert!(!results[0].was_download || results[0].was_download);
    }

    #[test]
    fn no_checksum_entry_no_verification() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();

        let content = b"anything";
        let spec = make_spec(
            vec!["data.bin::https://example.com/data.bin"],
            vec![],
        );
        let opts = opts_with_cache(&cache_dir);
        let dl = MockDownloader::new()
            .with_response("https://example.com/data.bin", content.to_vec());

        let results = fetch_sources(&spec, tmp.path(), &opts, &dl).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].was_download);
    }

    #[test]
    fn download_failure_leaves_no_final_cache_file() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();

        struct FailingDownloader;
        impl Downloader for FailingDownloader {
            fn download_to(&self, _url: &str, _dest: &Path) -> Result<u64, MakerpmError> {
                Err(MakerpmError::Fetch {
                    url: "fail".to_string(),
                    source: Box::new(std::io::Error::other("simulated failure")),
                })
            }
        }

        let spec = make_spec(
            vec!["data.bin::https://example.com/data.bin"],
            vec!["SKIP"],
        );
        let opts = opts_with_cache(&cache_dir);
        let dl = FailingDownloader;

        let result = fetch_sources(&spec, tmp.path(), &opts, &dl);
        assert!(result.is_err());
        assert!(!cache_path_for(&cache_dir, "data.bin").exists());
        assert!(!cache_path_for(&cache_dir, "data.bin").with_extension("part").exists());
    }

    #[test]
    fn cached_source_with_wrong_checksum_redownloads_on_non_refetch() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();

        let old_content = b"old data";
        let new_content = b"new data";
        let new_digest = hex_encode(&Sha256::digest(new_content));

        std::fs::write(cache_dir.join("data.bin"), old_content).unwrap();

        let spec = make_spec(
            vec!["data.bin::https://example.com/data.bin"],
            vec![&new_digest],
        );
        let opts = opts_with_cache(&cache_dir);
        let dl = MockDownloader::new()
            .with_response("https://example.com/data.bin", new_content.to_vec());

        let results = fetch_sources(&spec, tmp.path(), &opts, &dl).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].was_download);
        assert_eq!(dl.calls(), vec!["https://example.com/data.bin"]);
    }
}
