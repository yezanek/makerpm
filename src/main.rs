use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    let cli = makerpm::cli::Cli::parse();

    match cli.command {
        makerpm::cli::Commands::Validate(args) => {
            let toml_str = match std::fs::read_to_string(&args.spec_file) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Error: failed to read {}: {e}", args.spec_file.display());
                    return ExitCode::from(2);
                }
            };

            let spec = match makerpm::parse::parse_pkgspec(&toml_str) {
                Ok(s) => s,
                Err(e) => {
                    let report = miette::Report::new(e);
                    eprintln!("{report:?}");
                    return ExitCode::from(1);
                }
            };

            let toml_dir = args
                .spec_file
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."));

            let result = makerpm::validate::validate(&spec, toml_dir);

            if result.diagnostics.is_empty() {
                return ExitCode::SUCCESS;
            }

            let has_errors = result.has_errors();
            for diag in &result.diagnostics {
                eprintln!("{diag:?}");
            }

            if has_errors {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }

        makerpm::cli::Commands::Spec(args) => {
            let toml_str = match std::fs::read_to_string(&args.spec_file) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Error: failed to read {}: {e}", args.spec_file.display());
                    return ExitCode::from(2);
                }
            };

            let mut spec = match makerpm::parse::parse_pkgspec(&toml_str) {
                Ok(s) => s,
                Err(e) => {
                    let report = miette::Report::new(e);
                    eprintln!("{report:?}");
                    return ExitCode::from(1);
                }
            };

            let toml_dir = args
                .spec_file
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."));

            let result = makerpm::validate::validate(&spec, toml_dir);

            let has_errors = result.has_errors();
            for diag in &result.diagnostics {
                eprintln!("{diag:?}");
            }

            if has_errors {
                return ExitCode::from(1);
            }

            for dep in &result.injected_build_deps {
                if !spec.package.deps.build_depends.contains(dep) {
                    spec.package.deps.build_depends.push(dep.clone());
                }
            }

            let rendered = match makerpm::spec_gen::render(&spec, &result.injected_build_deps) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Error: {e}");
                    return ExitCode::from(1);
                }
            };

            match &args.output {
                Some(path) => {
                    if let Err(e) = std::fs::write(path, &rendered) {
                        eprintln!("Error: failed to write {}: {e}", path.display());
                        return ExitCode::from(1);
                    }
                }
                None => {
                    print!("{rendered}");
                }
            }

            ExitCode::SUCCESS
        }

        makerpm::cli::Commands::Fetch(args) => {
            let toml_str = match std::fs::read_to_string(&args.spec_file) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Error: failed to read {}: {e}", args.spec_file.display());
                    return ExitCode::from(2);
                }
            };

            let spec = match makerpm::parse::parse_pkgspec(&toml_str) {
                Ok(s) => s,
                Err(e) => {
                    let report = miette::Report::new(e);
                    eprintln!("{report:?}");
                    return ExitCode::from(1);
                }
            };

            let toml_dir = args
                .spec_file
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."));

            let result = makerpm::validate::validate(&spec, toml_dir);
            let has_errors = result.has_errors();
            for diag in &result.diagnostics {
                eprintln!("{diag:?}");
            }
            if has_errors {
                return ExitCode::from(1);
            }

            let opts = makerpm::fetch::FetchOptions {
                cache_dir: makerpm::fetch::resolve_cache_dir(),
                offline: args.fetch_flags.offline,
                refetch: args.fetch_flags.refetch,
                skip_checksums: args.fetch_flags.skip_checksums,
                allow_unverified: args.fetch_flags.allow_unverified,
            };

            let downloader = makerpm::fetch::UreqDownloader;

            match makerpm::fetch::fetch_sources(&spec, toml_dir, &opts, &downloader) {
                Ok(resolved) => {
                    let downloaded = resolved.iter().filter(|r| r.was_download).count();
                    let cached = resolved.iter().filter(|r| !r.was_download).count();
                    eprintln!(
                        "fetched {} source(s): {} downloaded, {} cached",
                        resolved.len(),
                        downloaded,
                        cached
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    ExitCode::from(1)
                }
            }
        }

        makerpm::cli::Commands::Build(args) => {
            let toml_str = match std::fs::read_to_string(&args.spec_file) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Error: failed to read {}: {e}", args.spec_file.display());
                    return ExitCode::from(2);
                }
            };

            let mut spec = match makerpm::parse::parse_pkgspec(&toml_str) {
                Ok(s) => s,
                Err(e) => {
                    let report = miette::Report::new(e);
                    eprintln!("{report:?}");
                    return ExitCode::from(1);
                }
            };

            let toml_dir = args
                .spec_file
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."));

            let result = makerpm::validate::validate(&spec, toml_dir);
            let has_errors = result.has_errors();
            for diag in &result.diagnostics {
                eprintln!("{diag:?}");
            }
            if has_errors {
                return ExitCode::from(1);
            }

            for dep in &result.injected_build_deps {
                if !spec.package.deps.build_depends.contains(dep) {
                    spec.package.deps.build_depends.push(dep.clone());
                }
            }

            let rendered = match makerpm::spec_gen::render(&spec, &result.injected_build_deps) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Error: {e}");
                    return ExitCode::from(1);
                }
            };

            let fetch_opts = makerpm::fetch::FetchOptions {
                cache_dir: makerpm::fetch::resolve_cache_dir(),
                offline: args.fetch_flags.offline,
                refetch: args.fetch_flags.refetch,
                skip_checksums: args.fetch_flags.skip_checksums,
                allow_unverified: args.fetch_flags.allow_unverified,
            };

            let downloader = makerpm::fetch::UreqDownloader;

            let resolved = match makerpm::fetch::fetch_sources(
                &spec,
                toml_dir,
                &fetch_opts,
                &downloader,
            ) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("Error fetching sources: {e}");
                    return ExitCode::from(1);
                }
            };

            let downloaded = resolved.iter().filter(|r| r.was_download).count();
            let cached = resolved.iter().filter(|r| !r.was_download).count();
            eprintln!(
                "fetched {} source(s): {} downloaded, {} cached",
                resolved.len(),
                downloaded,
                cached
            );

            let topdir = match makerpm::build_tree::setup_build_tree(
                &spec,
                toml_dir,
                &resolved,
                &rendered,
            ) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("Error setting up build tree: {e}");
                    return ExitCode::from(1);
                }
            };

            eprintln!("building RPMs...");

            if let Err(e) = makerpm::runner::run_rpmbuild(&topdir, &spec.package.name) {
                eprintln!("Error: {e}");
                if let makerpm::error::MakerpmError::RpmbuildFailed {
                    ref stderr_tail,
                    ..
                } = e
                {
                    if !stderr_tail.is_empty() {
                        eprintln!("\n--- rpmbuild stderr tail ---\n{stderr_tail}");
                    }
                }
                let _ = makerpm::build_tree::clean_build_tree(toml_dir);
                return ExitCode::from(1);
            }

            let output_dir = args
                .output_dir
                .clone()
                .unwrap_or_else(|| std::path::PathBuf::from("rpms"));

            match makerpm::runner::collect_artifacts(&topdir, &output_dir) {
                Ok(artifacts) => {
                    if artifacts.is_empty() {
                        eprintln!("warning: build succeeded but no RPMs were produced");
                    } else {
                        eprintln!("built {} RPM(s):", artifacts.len());
                        for rpm in &artifacts {
                            eprintln!("  {}", rpm.display());
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Error collecting artifacts: {e}");
                    let _ = makerpm::build_tree::clean_build_tree(toml_dir);
                    return ExitCode::from(1);
                }
            }

            let _ = makerpm::build_tree::clean_build_tree(toml_dir);
            ExitCode::SUCCESS
        }

        makerpm::cli::Commands::Init(args) => {
            let name = match args.name {
                Some(n) => n,
                None => {
                    let cwd = std::env::current_dir().unwrap_or_else(|_| {
                        eprintln!("Error: could not determine current directory");
                        std::process::exit(1);
                    });
                    cwd.file_name()
                        .unwrap_or_else(|| {
                            eprintln!("Error: could not determine directory name");
                            std::process::exit(1);
                        })
                        .to_string_lossy()
                        .to_string()
                }
            };

            let toml_content = format!(
                r#"[package]
name = "{name}"
version = "0.1.0"
summary = "A brief description of {name}"
license = "MIT"
description = """
{name} does something useful.
"""

[package.build]
system = "make"

[package.files]
paths = ["%{{_bindir}}/{name}"]

[[package.changelog]]
version = "0.1.0-1"
date = "{date}"
packager = "Your Name <you@example.com>"
entries = ["Initial package"]
"#,
                date = chrono_date_now(),
            );

            let path = std::path::PathBuf::from("PKGSPEC.toml");
            match std::fs::write(&path, &toml_content) {
                Ok(()) => {
                    eprintln!("created {}", path.display());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("Error: failed to write {}: {e}", path.display());
                    ExitCode::from(1)
                }
            }
        }
    }
}

fn chrono_date_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let days = now / 86400;
    let (y, m, d) = days_to_ymd(days as i64 + 719468);
    format!("{y:04}-{m:02}-{d:02}")
}

fn days_to_ymd(g: i64) -> (i64, u32, u32) {
    let y = (10000 * g + 14780) / 3652425;
    let mut doy = g - (365 * y + y / 4 - y / 100 + y / 400);
    if doy < 0 {
        let y2 = y - 1;
        doy = g - (365 * y2 + y2 / 4 - y2 / 100 + y2 / 400);
    }
    let mi = (100 * doy + 52) / 3060;
    let month = (mi + 2) % 12 + 1;
    let year = y + (mi + 2) / 12;
    let day = doy - (mi * 306 + 5) / 10 + 1;
    (year, month as u32, day as u32)
}
