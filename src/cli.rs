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
}

#[derive(clap::Args)]
pub struct ValidateArgs {
    /// Path to PKGSPEC.toml
    #[arg(long, default_value = "PKGSPEC.toml")]
    pub spec_file: PathBuf,
}
