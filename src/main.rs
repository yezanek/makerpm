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
    }
}
