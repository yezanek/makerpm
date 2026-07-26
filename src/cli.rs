use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "makerpm", version, about = "A modern RPM builder")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    Validate(ValidateArgs),
    Spec(SpecArgs),
}

#[derive(clap::Args)]
pub struct ValidateArgs {
    /// Path to PKGSPEC.toml
    #[arg(long, default_value = "PKGSPEC.toml")]
    pub spec_file: PathBuf,
}

#[derive(clap::Args)]
pub struct SpecArgs {
    /// Path to PKGSPEC.toml
    #[arg(long, default_value = "PKGSPEC.toml")]
    pub spec_file: PathBuf,

    /// Output file path (prints to stdout if omitted)
    #[arg(short, long)]
    pub output: Option<PathBuf>,
}
