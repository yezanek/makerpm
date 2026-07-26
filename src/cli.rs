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
    /// Validate a PKGSPEC.toml without building or fetching
    Validate(ValidateArgs),
    /// Render the RPM spec file from a PKGSPEC.toml
    Spec(SpecArgs),
    /// Download remote sources declared in the spec
    Fetch(FetchArgs),
    /// Build RPMs from a PKGSPEC.toml
    Build(BuildArgs),
    /// Scaffold a new PKGSPEC.toml in the current directory
    Init(InitArgs),
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
pub struct BuildArgs {
    /// Path to PKGSPEC.toml
    #[arg(long, default_value = "PKGSPEC.toml")]
    pub spec_file: PathBuf,

    /// Output directory for built RPMs (default: ./rpms/)
    #[arg(long)]
    pub output_dir: Option<PathBuf>,

    #[command(flatten)]
    pub fetch_flags: FetchFlags,
}

#[derive(clap::Args)]
pub struct InitArgs {
    /// Package name (defaults to current directory name)
    #[arg(long)]
    pub name: Option<String>,
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
