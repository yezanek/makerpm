use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

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

const CURL_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
enum CurlExit {
    Success,
    Failed { status: String, stderr: String },
    TimedOut,
}

trait CurlRunner: Send + Sync {
    fn run(
        &self,
        url: &str,
        dest: &Path,
        transfer_timeout: Option<Duration>,
    ) -> std::io::Result<CurlExit>;
}

struct SystemCurlRunner;

impl CurlRunner for SystemCurlRunner {
    fn run(
        &self,
        url: &str,
        dest: &Path,
        transfer_timeout: Option<Duration>,
    ) -> std::io::Result<CurlExit> {
        let connect_timeout = CURL_CONNECT_TIMEOUT.as_secs().to_string();
        let mut command = std::process::Command::new("curl");
        command.args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--max-redirs",
            "10",
            "--connect-timeout",
            &connect_timeout,
        ]);
        let transfer_timeout_arg = transfer_timeout.map(|timeout| timeout.as_secs().to_string());
        if let Some(timeout) = &transfer_timeout_arg {
            command.args(["--max-time", timeout]);
        }
        let mut child = command
            .arg("--output")
            .arg(dest)
            .arg("--")
            .arg(url)
            .stderr(Stdio::piped())
            .spawn()?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| std::io::Error::other("failed to capture curl stderr"))?;

        let deadline =
            transfer_timeout.map(|timeout| Instant::now() + timeout + Duration::from_secs(1));
        loop {
            if let Some(status) = child.try_wait()? {
                return curl_exit(status, &mut stderr);
            }
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                if let Some(status) = child.try_wait()? {
                    return curl_exit(status, &mut stderr);
                }
                child.kill()?;
                child.wait()?;
                return Ok(CurlExit::TimedOut);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

fn curl_exit(
    status: std::process::ExitStatus,
    stderr: &mut impl Read,
) -> std::io::Result<CurlExit> {
    let mut stderr_text = String::new();
    stderr.read_to_string(&mut stderr_text)?;
    Ok(if status.success() {
        CurlExit::Success
    } else {
        CurlExit::Failed {
            status: status.to_string(),
            stderr: stderr_text.trim().to_string(),
        }
    })
}

fn configured_transfer_timeout() -> Option<Duration> {
    std::env::var("MAKERPM_CURL_TIMEOUT")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
}

/// Real downloader using ureq for HTTP(S) and curl for FTP.
pub struct UreqDownloader {
    curl_runner: Box<dyn CurlRunner>,
}

impl Default for UreqDownloader {
    fn default() -> Self {
        Self {
            curl_runner: Box::new(SystemCurlRunner),
        }
    }
}

impl Downloader for UreqDownloader {
    fn download_to(&self, url: &str, dest: &Path) -> Result<u64, MakerpmError> {
        if url::Url::parse(url).is_ok_and(|parsed| parsed.scheme() == "ftp") {
            return download_ftp_with_curl(url, dest, self.curl_runner.as_ref());
        }

        let response = ureq::get(url).call().map_err(|e| MakerpmError::Fetch {
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

fn download_ftp_with_curl(
    url: &str,
    dest: &Path,
    runner: &dyn CurlRunner,
) -> Result<u64, MakerpmError> {
    let outcome = runner
        .run(url, dest, configured_transfer_timeout())
        .map_err(|source| MakerpmError::Fetch {
            url: url.to_string(),
            source: Box::new(source),
        })?;
    match outcome {
        CurlExit::Success => {}
        CurlExit::Failed { status, stderr } => {
            let details = if stderr.is_empty() {
                format!("curl exited with {status}")
            } else {
                format!("curl exited with {status}: {stderr}")
            };
            return Err(MakerpmError::Fetch {
                url: url.to_string(),
                source: Box::new(std::io::Error::other(details)),
            });
        }
        CurlExit::TimedOut => {
            return Err(MakerpmError::Fetch {
                url: url.to_string(),
                source: Box::new(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "curl exceeded the configured transfer timeout",
                )),
            });
        }
    }
    std::fs::metadata(dest)
        .map(|metadata| metadata.len())
        .map_err(|source| MakerpmError::Fetch {
            url: url.to_string(),
            source: Box::new(source),
        })
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

fn validate_cache_filename(filename: &str) -> Result<(), MakerpmError> {
    let path = std::path::Path::new(filename);
    let mut components = path.components();
    match components.next() {
        None => {
            return Err(MakerpmError::Fetch {
                url: String::new(),
                source: Box::new(std::io::Error::other(format!(
                    "invalid source filename: \"{filename}\""
                ))),
            });
        }
        Some(std::path::Component::Normal(_)) => {}
        Some(_) => {
            return Err(MakerpmError::Fetch {
                url: String::new(),
                source: Box::new(std::io::Error::other(format!(
                    "source filename is not a single path component: \"{filename}\""
                ))),
            });
        }
    }
    if components.next().is_some() {
        return Err(MakerpmError::Fetch {
            url: String::new(),
            source: Box::new(std::io::Error::other(format!(
                "source filename contains path separators: \"{filename}\""
            ))),
        });
    }
    Ok(())
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
}

/// Where a resolved source now lives on disk.
#[derive(Debug)]
pub struct ResolvedSource {
    pub local_path: PathBuf,
    pub filename: String,
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
                let fn_clone = filename.clone();
                results.push(ResolvedSource {
                    local_path,
                    filename: fn_clone,
                    was_download: false,
                });
            }
            SourceEntry::Remote {
                ref filename,
                ref url,
            } => {
                validate_cache_filename(filename)?;
                let cache_path = cache_path_for(&opts.cache_dir, filename);
                let declared_checksum = checksum_for(spec, idx, _kind);

                // §8.2 algorithm: decide whether to download
                let mut cached_hash: Option<String> = None;
                let should_download = if opts.refetch || !cache_path.exists() {
                    true
                } else if let Some(expected) = declared_checksum {
                    if expected != "SKIP" {
                        let actual = compute_sha256(&cache_path)?;
                        let mismatch = actual != expected;
                        cached_hash = Some(actual);
                        mismatch // §8.2 step 3c: mismatch → re-download
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
                    if let Err(e) = downloader.download_to(url, &part_path) {
                        let _ = std::fs::remove_file(&part_path);
                        return Err(e);
                    }
                    std::fs::rename(&part_path, &cache_path).map_err(|e| {
                        MakerpmError::CacheDir {
                            path: cache_path.clone(),
                            source: e,
                        }
                    })?;
                    cached_hash = None; // file changed, must re-hash
                }

                // §8.2 step 3e: verify checksum after download or cache hit
                if let Some(expected) = declared_checksum {
                    if expected != "SKIP" {
                        let actual = match cached_hash {
                            Some(h) => h,
                            None => compute_sha256(&cache_path)?,
                        };
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
                    filename: filename.clone(),
                    was_download: should_download,
                });
            }
        }
    }

    Ok(results)
}

fn checksum_for<'a>(spec: &'a PkgSpecFile, idx: usize, kind: &str) -> Option<&'a str> {
    let sums = match kind {
        "patch" => &spec.package.patch_sha256sums,
        _ => &spec.package.sha256sums,
    };
    sums.get(idx).map(String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    enum FakeCurlBehavior {
        Success(Vec<u8>),
        Nonzero,
        MissingOutput,
        SpawnFailure,
        Timeout,
    }

    struct FakeCurlRunner {
        behavior: FakeCurlBehavior,
    }

    impl CurlRunner for FakeCurlRunner {
        fn run(
            &self,
            _url: &str,
            dest: &Path,
            _transfer_timeout: Option<Duration>,
        ) -> std::io::Result<CurlExit> {
            match &self.behavior {
                FakeCurlBehavior::Success(content) => {
                    std::fs::write(dest, content)?;
                    Ok(CurlExit::Success)
                }
                FakeCurlBehavior::Nonzero => Ok(CurlExit::Failed {
                    status: "exit status: 22".into(),
                    stderr: "server denied access".into(),
                }),
                FakeCurlBehavior::MissingOutput => Ok(CurlExit::Success),
                FakeCurlBehavior::SpawnFailure => {
                    Err(std::io::Error::new(std::io::ErrorKind::NotFound, "curl"))
                }
                FakeCurlBehavior::Timeout => Ok(CurlExit::TimedOut),
            }
        }
    }

    fn ftp_downloader(behavior: FakeCurlBehavior) -> UreqDownloader {
        UreqDownloader {
            curl_runner: Box::new(FakeCurlRunner { behavior }),
        }
    }

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
            let data = self.responses.get(url).ok_or_else(|| MakerpmError::Fetch {
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
        let spec = make_spec(vec!["data.bin::https://example.com/data.bin"], vec!["SKIP"]);
        let opts = opts_with_cache(&cache_dir);
        let dl =
            MockDownloader::new().with_response("https://example.com/data.bin", content.to_vec());

        let results = fetch_sources(&spec, tmp.path(), &opts, &dl).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].was_download);
        assert_eq!(dl.calls(), vec!["https://example.com/data.bin"]);
        assert!(cache_dir.join("data.bin").exists());
    }

    #[test]
    fn bare_ftp_source_uses_downloader() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();

        let content = b"ftp data";
        let spec = make_spec(vec!["ftp://example.com/data.bin"], vec!["SKIP"]);
        let opts = opts_with_cache(&cache_dir);
        let dl =
            MockDownloader::new().with_response("ftp://example.com/data.bin", content.to_vec());

        let results = fetch_sources(&spec, tmp.path(), &opts, &dl).unwrap();
        assert_eq!(results[0].filename, "data.bin");
        assert_eq!(dl.calls(), vec!["ftp://example.com/data.bin"]);
    }

    #[test]
    fn real_ftp_flow_accepts_successful_curl_output() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("download.part");
        let downloader = ftp_downloader(FakeCurlBehavior::Success(b"ftp data".to_vec()));

        let written = downloader
            .download_to("ftp://example.com/data.bin", &dest)
            .unwrap();

        assert_eq!(written, 8);
        assert_eq!(std::fs::read(dest).unwrap(), b"ftp data");
    }

    #[test]
    fn real_ftp_flow_reports_nonzero_curl_and_cleans_part_file() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("cache");
        let spec = make_spec(vec!["ftp://example.com/data.bin"], vec!["SKIP"]);
        let opts = opts_with_cache(&cache_dir);
        let downloader = ftp_downloader(FakeCurlBehavior::Nonzero);

        let result = fetch_sources(&spec, tmp.path(), &opts, &downloader);

        assert!(result.is_err());
        assert!(format!("{}", result.unwrap_err()).contains("server denied access"));
        assert!(!cache_dir.join("data.part").exists());
        assert!(!cache_dir.join("data.bin").exists());
    }

    #[test]
    fn real_ftp_flow_rejects_missing_output_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("missing.part");
        let downloader = ftp_downloader(FakeCurlBehavior::MissingOutput);

        let result = downloader.download_to("ftp://example.com/data.bin", &dest);

        assert!(result.is_err());
    }

    #[test]
    fn real_ftp_flow_reports_subprocess_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("download.part");
        let downloader = ftp_downloader(FakeCurlBehavior::SpawnFailure);

        let result = downloader.download_to("ftp://example.com/data.bin", &dest);

        assert!(result.is_err());
    }

    #[test]
    fn real_ftp_flow_reports_timeout() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("download.part");
        let downloader = ftp_downloader(FakeCurlBehavior::Timeout);

        let result = downloader.download_to("ftp://example.com/data.bin", &dest);

        assert!(matches!(
            result,
            Err(MakerpmError::Fetch { source, .. })
                if source.downcast_ref::<std::io::Error>().is_some_and(|error| error.kind() == std::io::ErrorKind::TimedOut)
        ));
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
        let dl =
            MockDownloader::new().with_response("https://example.com/data.bin", content.to_vec());

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
        let dl =
            MockDownloader::new().with_response("https://example.com/data.bin", content.to_vec());

        let results = fetch_sources(&spec, tmp.path(), &opts, &dl).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].was_download);
    }

    #[test]
    fn offline_mode_uncached_remote_hard_error() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();

        let spec = make_spec(vec!["data.bin::https://example.com/data.bin"], vec!["SKIP"]);
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
        let spec = make_spec(vec!["data.bin::https://example.com/data.bin"], vec!["SKIP"]);
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
        let spec = make_spec(vec!["data.bin::https://example.com/data.bin"], vec!["SKIP"]);
        let opts = opts_with_cache(&cache_dir);
        let dl =
            MockDownloader::new().with_response("https://example.com/data.bin", content.to_vec());

        let results = fetch_sources(&spec, tmp.path(), &opts, &dl).unwrap();
        assert_eq!(results.len(), 1);
        assert!(
            results[0].was_download,
            "cache miss should trigger download"
        );
        assert_eq!(std::fs::read(cache_dir.join("data.bin")).unwrap(), content);
    }

    #[test]
    fn no_checksum_entry_no_verification() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();

        let content = b"anything";
        let spec = make_spec(vec!["data.bin::https://example.com/data.bin"], vec![]);
        let opts = opts_with_cache(&cache_dir);
        let dl =
            MockDownloader::new().with_response("https://example.com/data.bin", content.to_vec());

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
            fn download_to(&self, _url: &str, dest: &Path) -> Result<u64, MakerpmError> {
                std::fs::write(dest, b"partial").map_err(|e| MakerpmError::Fetch {
                    url: "fail".to_string(),
                    source: Box::new(e),
                })?;
                Err(MakerpmError::Fetch {
                    url: "fail".to_string(),
                    source: Box::new(std::io::Error::other("simulated failure")),
                })
            }
        }

        let spec = make_spec(vec!["data.bin::https://example.com/data.bin"], vec!["SKIP"]);
        let opts = opts_with_cache(&cache_dir);
        let dl = FailingDownloader;

        let result = fetch_sources(&spec, tmp.path(), &opts, &dl);
        assert!(result.is_err());
        assert!(!cache_path_for(&cache_dir, "data.bin").exists());
        assert!(!cache_path_for(&cache_dir, "data.bin")
            .with_extension("part")
            .exists());
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

    #[test]
    fn validate_cache_filename_rejects_empty() {
        assert!(validate_cache_filename("").is_err());
    }

    #[test]
    fn validate_cache_filename_rejects_dot_dot() {
        assert!(validate_cache_filename("..").is_err());
    }

    #[test]
    fn validate_cache_filename_rejects_traversal() {
        assert!(validate_cache_filename("foo/../../etc/passwd").is_err());
    }

    #[test]
    fn validate_cache_filename_rejects_absolute() {
        assert!(validate_cache_filename("/etc/passwd").is_err());
    }

    #[test]
    fn validate_cache_filename_rejects_nested() {
        assert!(validate_cache_filename("sub/dir/file.tar.gz").is_err());
    }

    #[test]
    fn validate_cache_filename_accepts_normal() {
        assert!(validate_cache_filename("test.tar.gz").is_ok());
        assert!(validate_cache_filename("foo-bar_1.0-1.x86_64.rpm").is_ok());
    }
}
