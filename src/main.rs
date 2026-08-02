use std::process::ExitCode;

use clap::Parser;

enum EarlyReturn {
    Code(ExitCode),
    Parsed(
        Box<makerpm::model::PkgSpecFile>,
        std::path::PathBuf,
        makerpm::lint::LintResult,
    ),
}

fn load_and_lint(spec_file: &std::path::Path) -> EarlyReturn {
    let toml_str = match std::fs::read_to_string(spec_file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error: failed to read {}: {e}", spec_file.display());
            return EarlyReturn::Code(ExitCode::from(2));
        }
    };

    let spec = match makerpm::parse::parse_pkgspec(&toml_str) {
        Ok(s) => s,
        Err(e) => {
            let report = miette::Report::new(e);
            eprintln!("{report:?}");
            return EarlyReturn::Code(ExitCode::from(1));
        }
    };

    let toml_dir = spec_file
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf();

    let result = makerpm::lint::lint(&spec, &toml_dir, &toml_str);

    EarlyReturn::Parsed(Box::new(spec), toml_dir, result)
}

fn main() -> ExitCode {
    let cli = makerpm::cli::Cli::parse();
    let verbosity = cli.verbose;
    init_logging(verbosity);

    match cli.command {
        makerpm::cli::Commands::Lint(args) => {
            let (_spec, _toml_dir, result) = match load_and_lint(&args.path) {
                EarlyReturn::Parsed(s, d, r) => (s, d, r),
                EarlyReturn::Code(code) => return code,
            };

            if result.findings.is_empty() {
                return ExitCode::SUCCESS;
            }

            let has_errors = result.has_errors();
            let has_warnings = result.has_warnings();
            print_findings(&result.findings);

            if has_errors || (args.strict && has_warnings) {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }

        makerpm::cli::Commands::Spec(args) => {
            let (spec, _toml_dir, result) = match load_and_lint(&args.path) {
                EarlyReturn::Parsed(s, d, r) => (s, d, r),
                EarlyReturn::Code(code) => return code,
            };

            let has_errors = result.has_errors();
            print_findings(&result.findings);

            if has_errors {
                return ExitCode::from(1);
            }

            log_injected_dependencies(verbosity, &result.injected_build_deps);

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
            let (spec, toml_dir, result) = match load_and_lint(&args.path) {
                EarlyReturn::Parsed(s, d, r) => (s, d, r),
                EarlyReturn::Code(code) => return code,
            };

            let has_errors = result.has_errors();
            print_findings(&result.findings);
            if has_errors {
                return ExitCode::from(1);
            }

            log_injected_dependencies(verbosity, &result.injected_build_deps);
            if !may_use_unverified_sources(
                result.has_unverified_sources,
                args.fetch_flags.allow_unverified,
            ) {
                return ExitCode::from(1);
            }

            let opts = makerpm::fetch::FetchOptions {
                cache_dir: makerpm::fetch::resolve_cache_dir(),
                offline: args.fetch_flags.offline,
                refetch: args.fetch_flags.refetch,
                skip_checksums: args.fetch_flags.skip_checksums,
            };

            let downloader = makerpm::fetch::UreqDownloader::default();

            match makerpm::fetch::fetch_sources(&spec, &toml_dir, &opts, &downloader) {
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
            let (spec, toml_dir, result) = match load_and_lint(&args.path) {
                EarlyReturn::Parsed(s, d, r) => (s, d, r),
                EarlyReturn::Code(code) => return code,
            };

            let has_errors = result.has_errors();
            print_findings(&result.findings);
            if has_errors {
                return ExitCode::from(1);
            }

            log_injected_dependencies(verbosity, &result.injected_build_deps);
            if !may_use_unverified_sources(
                result.has_unverified_sources,
                args.fetch_flags.allow_unverified,
            ) {
                return ExitCode::from(1);
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
            };

            let downloader = makerpm::fetch::UreqDownloader::default();

            let resolved =
                match makerpm::fetch::fetch_sources(&spec, &toml_dir, &fetch_opts, &downloader) {
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

            let topdir =
                match makerpm::build_tree::setup_build_tree(&spec, &toml_dir, &resolved, &rendered)
                {
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
                    ref stderr_tail, ..
                } = e
                {
                    if !stderr_tail.is_empty() {
                        eprintln!("\n--- rpmbuild stderr tail ---\n{stderr_tail}");
                    }
                }
                eprintln!("build tree preserved at: {}", topdir.display());
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
                    eprintln!("build tree preserved at: {}", topdir.display());
                    return ExitCode::from(1);
                }
            }

            let _ = makerpm::build_tree::clean_build_tree(&toml_dir);
            ExitCode::SUCCESS
        }

        makerpm::cli::Commands::Init(args) => {
            let name = match args.name {
                Some(n) => n,
                None => {
                    let cwd = match std::env::current_dir() {
                        Ok(d) => d,
                        Err(e) => {
                            eprintln!("Error: could not determine current directory: {e}");
                            return ExitCode::from(1);
                        }
                    };
                    match cwd.file_name() {
                        Some(n) => n.to_string_lossy().to_string(),
                        None => {
                            eprintln!("Error: could not determine directory name");
                            return ExitCode::from(1);
                        }
                    }
                }
            };

            if name
                .chars()
                .any(|c| c == '"' || c == '\\' || c.is_control())
            {
                eprintln!(
                    "Error: package name {name:?} contains characters that are not valid in a TOML string"
                );
                return ExitCode::from(1);
            }

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
                date = today_utc(),
            );

            let path = std::path::PathBuf::from("PKGSPEC.toml");
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    use std::io::Write;
                    if let Err(e) = file.write_all(toml_content.as_bytes()) {
                        eprintln!("Error: failed to write {}: {e}", path.display());
                        return ExitCode::from(1);
                    }
                    eprintln!("created {}", path.display());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("Error: failed to create {}: {e}", path.display());
                    ExitCode::from(1)
                }
            }
        }
    }
}

fn today_utc() -> String {
    let now = time::OffsetDateTime::now_utc();
    let format = time::macros::format_description!("[year]-[month]-[day]");
    now.format(format)
        .expect("the static ISO date format is always valid")
}

fn print_findings(findings: &[makerpm::lint::LintFinding]) {
    use makerpm::lint::Severity;

    for (severity, heading) in [(Severity::Error, "Errors"), (Severity::Warning, "Warnings")] {
        let matching = findings
            .iter()
            .filter(|finding| finding.severity == severity)
            .collect::<Vec<_>>();
        if matching.is_empty() {
            continue;
        }

        eprintln!("{heading}:");
        for finding in matching {
            eprintln!("  {}: {}", finding.field_path, finding.message);
            if let Some(suggestion) = &finding.suggestion {
                eprintln!("    help: {suggestion}");
            }
        }
    }
}

fn log_injected_dependencies(verbosity: u8, dependencies: &[String]) {
    if verbosity > 0 {
        for dependency in dependencies {
            tracing::info!(
                dependency = %dependency,
                "adding build requirement for the selected build system"
            );
        }
    }
}

fn init_logging(verbosity: u8) {
    let level = match verbosity {
        0 => tracing::Level::WARN,
        1 => tracing::Level::INFO,
        _ => tracing::Level::DEBUG,
    };
    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_target(false)
        .without_time()
        .with_writer(std::io::stderr)
        .init();
}

fn may_use_unverified_sources(has_unverified: bool, explicitly_allowed: bool) -> bool {
    use std::io::{IsTerminal, Write};

    if !has_unverified || explicitly_allowed {
        return true;
    }
    if !std::io::stdin().is_terminal() {
        eprintln!(
            "Error: remote sources without checksums detected. Use --allow-unverified to proceed in non-interactive mode."
        );
        return false;
    }

    eprint!("Continue with the unverified remote sources? [y/N] ");
    if std::io::stderr().flush().is_err() {
        return false;
    }
    let mut answer = String::new();
    match std::io::stdin().read_line(&mut answer) {
        Ok(_) if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") => true,
        Ok(_) => {
            eprintln!("cancelled");
            false
        }
        Err(error) => {
            eprintln!("Error: failed to read confirmation: {error}");
            false
        }
    }
}
