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
    Fetch(FetchArgs),
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

#[derive(clap::Args)]
pub struct FetchArgs {
    /// Path to PKGSPEC.toml
    #[arg(long, default_value = "PKGSPEC.toml")]
    pub spec_file: PathBuf,

    #[command(flatten)]
    pub fetch_flags: FetchFlags,
}

#[derive(clap::Args)]
pub struct FetchFlags {
    /// Never touch the network; fail if an uncached remote source is needed
    #[arg(long)]
    pub offline: bool,

    /// Ignore cache and re-download every remote source
    #[arg(long)]
    pub refetch: bool,

    /// Proceed on checksum mismatch with a warning
    #[arg(long)]
    pub skip_checksums: bool,

    /// Proceed past unverified-source warnings without prompting (CI mode)
    #[arg(long)]
    pub allow_unverified: bool,
}
