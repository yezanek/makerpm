# makerpm

A command-line tool for building RPM packages from a single TOML file.
Inspired by Arch Linux's `makepkg`/`PKGBUILD` workflow, `makerpm` reads a
`PKGSPEC.toml`, validates it, downloads and checksum-verifies any declared
remote sources, transpiles the spec into a standard RPM `.spec` file, and
invokes `rpmbuild` to produce the final `.rpm` files — all in one command.

## Prerequisites

- **Rust** (1.70+) — install via [rustup](https://rustup.rs/)
- **rpm-build** — required at build time by `rpmbuild`, which `makerpm`
  shells out to. On Fedora/RHEL: `sudo dnf install rpm-build`. On
  openSUSE: `sudo zypper install rpm-build`.
- **curl** — required only when downloading an `ftp://` source.

## Building

```sh
cargo build --release
```

The binary will be at `target/release/makerpm`. To install it into your
`$PATH`:

```sh
cargo install --path .
```

## Quick start

```sh
# Scaffold a starter PKGSPEC.toml (uses the directory name as the package name)
makerpm init

# Edit PKGSPEC.toml with your package details, then build:
makerpm build

# RPMs are written to ./rpms/ by default (override with --output-dir)
```

## CLI usage

```
makerpm [-v|-vv|--verbose] <COMMAND>
makerpm build [PATH] [--output-dir DIR] [FLAGS]
makerpm spec  [PATH] [--output FILE]
makerpm fetch [PATH] [FLAGS]
makerpm lint  [PATH] [--strict]
makerpm init [--name NAME]
```

`-v`, `-vv`, and `--verbose` are global options and work with every subcommand.

`PATH` defaults to `./PKGSPEC.toml` in the current directory. The former
`validate` command remains available as a hidden alias for `lint`.

**Subcommands:**

| Command | Description |
|---|---|
| `build` | Full pipeline: parse, lint, fetch sources, render `.spec`, invoke `rpmbuild`, collect RPMs |
| `spec` | Render the `.spec` file only (no download, no build) |
| `fetch` | Download and verify remote sources without building |
| `lint` | Parse and lint the TOML; `--strict` also fails on warnings |
| `init` | Create a starter `PKGSPEC.toml` in the current directory |

**Build/fetch flags:**

| Flag | Description |
|---|---|
| `--offline` | Never touch the network; fail if an uncached remote source is needed |
| `--refetch` | Ignore cache and re-download every remote source |
| `--skip-checksums` | Proceed on checksum mismatch with a warning |
| `--allow-unverified` | Proceed past unverified-source warnings (useful for CI) |

FTP transfers have no total-duration limit by default. Set
`MAKERPM_CURL_TIMEOUT` to a positive number of seconds to enforce one;
connection attempts always time out after 30 seconds.

## PKGSPEC.toml format

```toml
[package]
name = "hello-world"
version = "1.0"
summary = "A friendly greeting program"
license = "MIT"
description = "hello-world prints a friendly greeting to standard output."

# Local sources are resolved relative to this file.
# Remote sources are downloaded and cached automatically.
sources = ["hello-world-1.0.tar.gz"]
# sha256sums are parallel to sources; use "SKIP" for local/untracked files.
sha256sums = ["SKIP"]

[package.build]
system = "make"  # also: autotools, cmake, meson, cargo, python-pyproject, none

[package.files]
paths = ["%{_bindir}/hello-world"]
docs = ["README.md"]
licenses = ["LICENSE"]

[[package.changelog]]
version = "1.0-1"
date = "2026-07-26"
packager = "Your Name <you@example.com>"
entries = ["Initial package"]
```

For subpackages (`-devel`, `-doc`, etc.), remote source syntax, and
advanced build-system options, see the examples under `examples/`:

- `examples/hello-world/` — minimal local-source package
- `examples/remote-hello/` — package fetched from a remote URL (GNU Hello)

## License

GPL-3.0-or-later — see [LICENSE](LICENSE).
