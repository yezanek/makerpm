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
    }
}
