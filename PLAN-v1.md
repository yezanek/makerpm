# makerpm — Implementation Plan (v1)

## 1. Purpose and scope

`makerpm` is a CLI tool, written in idiomatic Rust, that:

1. Reads a single TOML file (working name: `PKGSPEC.toml`) describing a source
   package, using a schema that combines the spirit of Arch's `PKGBUILD` with
   the full field set of an RPM `.spec` file, reorganized logically.
2. Validates the file and produces clear, actionable errors.
3. Downloads any remote sources listed in the TOML, verifies their
   checksums, and caches them locally — following `makepkg`'s
   `source=()`/`sha256sums=()` convention (see §8).
4. Transpiles it into one or more idiomatic, Fedora-guideline-compliant
   `.spec` files (a package can define subpackages — e.g. `-devel`, `-doc`).
5. Sets up a standard `rpmbuild` tree, stages sources/patches into it.
6. Invokes the system's `rpmbuild` binary to actually build the RPM(s).
7. Collects the resulting `.rpm` (and optionally `.src.rpm`) files back into
   a predictable output location, analogous to how `makepkg` drops the built
   package next to the `PKGBUILD`.

### Explicit non-goals for v1

- No dependency resolution against repositories, no AUR-style search/install
  of *built packages* — this is a build tool, not a package manager.
  Networking is limited to fetching the source archives/patches a package
  declares, exactly as `makepkg` does; it does not touch `dnf`/repo metadata.
- No VCS-style sources (`git+`, `svn+`, `hg+`, `bzr+` prefixes in makepkg).
  v1 supports plain HTTP(S)/FTP downloads and local files only. VCS source
  support is a natural v2 addition and should be designed for but not
  implemented now (see §8 for the extension point).
- No package installation step (no `sudo dnf install` at the end). It stops
  once the RPM(s) exist on disk.
- No custom RPM binary implementation. `makerpm` assumes the official
  `rpm-build` / `rpmbuild` package is installed on the host and shells out
  to it. `makerpm` owns the TOML→spec transpilation and orchestration only.
- No GUI, no daemon mode, no config beyond the per-package TOML and minimal
  global CLI flags.

### Definition of done for v1

Given a directory containing only `PKGSPEC.toml` (plus any *local* patches
it references), running `makerpm build` downloads whatever remote sources
the TOML declares, verifies them, and produces valid `.rpm` files (main +
subpackages) in an output directory, using a correctly-formed, idiomatic
`.spec` file generated from the TOML, with no manual spec editing required
for the common cases (cmake/meson/autotools/cargo/make/pure-shell build
systems). This is the same "clone a PKGBUILD-style spec, run one command,
get built packages" experience `makepkg` provides — no separate `wget`
step required.

---

## 2. Toolchain and crate choices

| Concern | Crate | Notes |
|---|---|---|
| TOML parsing | `toml` + `serde` | Deserialize into strongly-typed structs |
| CLI | `clap` (derive API) | Subcommands: `build`, `spec` (spec-only, no build), `validate`, `init` |
| Templating | `tera` | Jinja2-like; used to render the final `.spec` file |
| Diagnostics | `miette` (+ `thiserror`) | Rich, source-span-aware error messages for TOML validation failures |
| Process execution | `std::process::Command` (or `duct` if convenient) | Invoke `rpmbuild`, stream stdout/stderr live |
| Filesystem ops | `std::fs`, `walkdir` if needed | Setting up the rpmbuild tree, copying sources |
| Logging | `tracing` + `tracing-subscriber` | `-v`/`-vv` verbosity flags |
| HTTP downloads | `ureq` (blocking, minimal deps) | Fetch remote `source`/`patch` entries; blocking is fine, `makerpm` is a sequential CLI tool like `makepkg`, not a server |
| URL parsing | `url` | Detect http/https/ftp schemes and derive default filenames in `source_spec.rs` |
| Checksums | `sha2` (+ `md-5` only if legacy `md5sums` compatibility is wanted) | Verify downloaded files against declared hashes before staging them |
| Progress display | `indicatif` | Per-file download progress bar, matching `makepkg`'s familiar `curl`-style download feedback |
| Testing | `insta` (snapshot testing) | Critical for verifying generated `.spec` output stays correct across refactors |

No crate is needed for RPM binary parsing in v1 (we never read `.rpm`
files ourselves — `rpmbuild` does that).

---

## 3. Repository / module layout

```
makerpm/
├── Cargo.toml
├── src/
│   ├── main.rs                 # CLI entry point, arg parsing, dispatch
│   ├── cli.rs                  # clap struct definitions
│   ├── model/
│   │   ├── mod.rs
│   │   ├── package.rs          # Package, Subpackage structs
│   │   ├── build.rs            # BuildSystem enum, BuildSteps
│   │   ├── files.rs            # FilesSpec (paths/docs/licenses/configs)
│   │   ├── deps.rs             # DependencySet (Requires/BuildRequires/...)
│   │   ├── scriptlets.rs       # Scriptlets struct
│   │   └── changelog.rs        # ChangelogEntry
│   ├── parse.rs                # TOML -> model, with miette diagnostics
│   ├── validate.rs             # Cross-field validation (post-parse)
│   ├── source_spec.rs          # parses "name::url" syntax, resolves local vs. remote sources
│   ├── fetch.rs                # downloads sources, verifies checksums, manages the cache dir
│   ├── spec_gen/
│   │   ├── mod.rs
│   │   ├── template.tera       # (or embedded via include_str!)
│   │   └── render.rs           # Model -> Tera context -> rendered .spec string
│   ├── build_tree.rs           # rpmbuild tree setup, source staging
│   ├── runner.rs               # invoke rpmbuild, stream output, collect artifacts
│   └── error.rs                # top-level error types
├── tests/
│   ├── fixtures/                # sample PKGSPEC.toml files + expected .spec snapshots
│   └── integration_test.rs
└── examples/
    └── hello-world/
        ├── PKGSPEC.toml
        └── hello-world-1.0.tar.gz
```

This mirrors the pipeline stages 1:1 with modules, which keeps each stage
independently testable: `parse` and `validate` never touch the filesystem
beyond reading the TOML; `spec_gen` is a pure function (model → string,
snapshot-testable with `insta` with zero I/O); `source_spec` is a pure
string-parsing module (no I/O, easily unit-tested); `fetch`, `build_tree`,
and `runner` are the only modules that touch the network, `rpmbuild`, and
the filesystem.

---

## 4. Data model

### 4.1 Shared building-block structs

These are reused by both the top-level package and each subpackage, so
define them once:

```rust
#[derive(Debug, Deserialize, Default)]
pub struct DependencySet {
    #[serde(default)] pub build_depends: Vec<String>, // BuildRequires
    #[serde(default)] pub depends: Vec<String>,        // Requires
    #[serde(default)] pub recommends: Vec<String>,     // Recommends
    #[serde(default)] pub suggests: Vec<String>,        // Suggests
    #[serde(default)] pub conflicts: Vec<String>,       // Conflicts
    #[serde(default)] pub provides: Vec<String>,        // Provides
    #[serde(default)] pub obsoletes: Vec<String>,       // Obsoletes
    #[serde(default)] pub supplements: Vec<String>,     // Supplements
    #[serde(default)] pub enhances: Vec<String>,        // Enhances
}

#[derive(Debug, Deserialize, Default)]
pub struct FilesSpec {
    #[serde(default)] pub paths: Vec<String>,             // plain %files entries
    #[serde(default)] pub docs: Vec<String>,               // %doc
    #[serde(default)] pub licenses: Vec<String>,           // %license
    #[serde(default)] pub configs_noreplace: Vec<String>,  // %config(noreplace)
    #[serde(default)] pub configs: Vec<String>,            // %config
    #[serde(default)] pub dirs: Vec<String>,               // %dir
}

#[derive(Debug, Deserialize, Default)]
pub struct Scriptlets {
    #[serde(default)] pub pretrans: Option<String>,
    #[serde(default)] pub pre: Option<String>,
    #[serde(default)] pub post: Option<String>,
    #[serde(default)] pub preun: Option<String>,
    #[serde(default)] pub postun: Option<String>,
    #[serde(default)] pub posttrans: Option<String>,
    // interpreter override, e.g. "/sbin/ldconfig" or "<lua>"
    #[serde(default)] pub interpreter: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChangelogEntry {
    pub version: String,   // e.g. "1.2.3-1"
    pub date: String,      // ISO 8601, reformatted to RPM's %changelog date format at render time
    pub packager: String,  // "Jane Doe <jane@example.org>"
    pub entries: Vec<String>,
}
```

### 4.2 Build system

```rust
#[derive(Debug, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BuildSystem {
    #[default]
    None,
    Make,
    Autotools,
    Cmake,
    Meson,
    Cargo,
    PythonPyproject,
}

#[derive(Debug, Deserialize, Default)]
pub struct BuildSteps {
    #[serde(default)] pub prep: Option<String>,    // raw shell appended after %autosetup
    #[serde(default)] pub build: Option<String>,   // raw shell appended after the system's %*_build macro (or the whole %build if system = none)
    #[serde(default)] pub install: Option<String>, // same pattern for %install
    #[serde(default)] pub check: Option<String>,   // %check body; presence implies run_tests = true unless explicitly false
}

#[derive(Debug, Deserialize, Default)]
pub struct BuildSpec {
    #[serde(default)] pub system: BuildSystem,
    #[serde(default)] pub extra_build_args: Vec<String>,
    #[serde(default)] pub extra_install_args: Vec<String>,
    #[serde(default)] pub run_tests: Option<bool>,
    #[serde(default)] pub steps: BuildSteps,
}
```

`BuildSystem` drives a lookup table (see §6) mapping each variant to:
its required `BuildRequires` (e.g. `cmake` → `BuildRequires: cmake`,
`gcc-c++` where relevant), the `%build` macro pair, and the `%install`
macro pair.

### 4.3 Package and Subpackage

```rust
#[derive(Debug, Deserialize)]
pub struct Package {
    pub name: String,
    pub version: String,
    #[serde(default = "default_release")]
    pub release: String,           // default: "1%{?dist}"
    #[serde(default)]
    pub epoch: Option<u32>,
    pub summary: String,
    pub license: String,           // SPDX expression, validated
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub noarch: bool,
    pub description: String,

    // Source0..N. Each entry follows makepkg's own convention:
    //   - a bare filename            -> must exist locally, next to the TOML
    //   - a bare URL                 -> downloaded, kept as the URL's basename
    //   - "filename::https://..."    -> downloaded, saved locally as `filename`
    //     (this is *exactly* makepkg's `source=()` renaming syntax)
    #[serde(default)] pub sources: Vec<String>,
    // Parallel array to `sources`, same index correspondence as makepkg's
    // sha256sums=(). Use the literal string "SKIP" for local/untracked
    // sources, matching makepkg exactly. If omitted entirely, all sources
    // are effectively "SKIP" (allowed, but validate.rs emits a warning for
    // every downloaded — not local — source left unchecksummed).
    #[serde(default)] pub sha256sums: Vec<String>,
    #[serde(default)] pub patches: Vec<String>,   // Patch0..N — same source syntax as `sources`
    #[serde(default)] pub patch_sha256sums: Vec<String>, // parallel to `patches`, same "SKIP" rules

    #[serde(default)] pub deps: DependencySet,
    #[serde(default)] pub build: BuildSpec,
    #[serde(default)] pub files: FilesSpec,
    #[serde(default)] pub scriptlets: Scriptlets,
    #[serde(default)] pub changelog: Vec<ChangelogEntry>,
}

#[derive(Debug, Deserialize)]
pub struct Subpackage {
    pub suffix: String,            // -> "<name>-<suffix>", not independently overridable in v1
    pub summary: String,           // required, no inheritance
    pub description: String,       // required, no inheritance
    #[serde(default)] pub noarch: Option<bool>,   // overrides package.noarch if set
    #[serde(default)] pub license: Option<String>, // overrides package.license if set
    #[serde(default)] pub url: Option<String>,

    #[serde(default)] pub deps: DependencySet,     // NOT inherited — must be explicit
    #[serde(default)] pub files: FilesSpec,        // required, no inheritance (validated non-empty)
    #[serde(default)] pub scriptlets: Scriptlets,  // independent from main package
}

#[derive(Debug, Deserialize)]
pub struct PkgSpecFile {
    pub package: Package,
    #[serde(default, rename = "subpackage")]
    pub subpackages: Vec<Subpackage>,
}
```

Design principle to preserve throughout implementation: internally,
treat `package` as "subpackage index 0" wherever the renderer loops over
`%package`/`%files`/`%description` blocks, so the Tera template has one
loop construct, not a special-cased main package plus a separate loop.
Build a small adapter/view type if that simplifies the template context
(e.g. `RenderablePackage { name, summary, description, deps, files,
scriptlets, is_base: bool }` built from both `Package` and `Subpackage`).

### 4.4 Source entry syntax (`source_spec.rs`)

Parses each raw string from `sources`/`patches` into a small enum, mirroring
`makepkg`'s own source-array grammar so PKGBUILD authors don't have to learn
a new one:

```rust
#[derive(Debug, PartialEq, Eq)]
pub enum SourceEntry {
    Local { filename: String },                       // "foo.patch"
    Remote { filename: String, url: String },          // resolved from either
                                                        // "https://.../foo.tar.gz"
                                                        // or "foo.tar.gz::https://.../v1.0.tar.gz"
}

pub fn parse_source_entry(raw: &str) -> SourceEntry {
    if let Some((filename, url)) = raw.split_once("::") {
        return SourceEntry::Remote { filename: filename.to_string(), url: url.to_string() };
    }
    if let Ok(url) = url::Url::parse(raw) {
        if matches!(url.scheme(), "http" | "https" | "ftp") {
            let filename = url.path_segments()
                .and_then(|segs| segs.last())
                .filter(|s| !s.is_empty())
                .unwrap_or(raw)
                .to_string();
            return SourceEntry::Remote { filename, url: raw.to_string() };
        }
    }
    SourceEntry::Local { filename: raw.to_string() }
}
```

This is pure and trivially unit-testable: feed it every shape from the
table below and assert the resulting variant.

| TOML `sources` entry | Resolves to |
|---|---|
| `"hello-1.0.tar.gz"` | `Local` — must already exist next to the TOML |
| `"https://example.org/hello-1.0.tar.gz"` | `Remote`, saved as `hello-1.0.tar.gz` |
| `"hello-1.0.tar.gz::https://example.org/v1.0.tar.gz"` | `Remote`, saved as `hello-1.0.tar.gz` (the makepkg rename form — needed when upstream URLs don't end in a sane filename, e.g. GitHub's `/archive/refs/tags/v1.0.tar.gz`) |

(`url` crate needed for robust parsing/scheme detection — add it alongside
`ureq` in §2's crate table.)

---

## 5. Validation rules (post-parse, pre-render)

Implement as a distinct `validate.rs` pass returning a `Vec<Diagnostic>`
(collect all errors, don't stop at the first one — this is much better UX
than one-error-at-a-time). Required checks:

- `package.version` must not contain a literal `-` (RPM restriction).
- `package.license` should be checked against a known SPDX identifier list
  (bundle a static list or a small crate) — warn, don't hard-fail, since
  SPDX expressions can be compound (`MIT OR Apache-2.0`) and full expression
  parsing is a nice-to-have, not a v1 blocker.
- Every `sources`/`patches` entry is parsed via `source_spec::parse_source_entry`
  (§4.4). `Local` entries must exist on disk relative to the TOML's
  directory — error immediately (nothing to download, so this is a hard
  failure, same as `makepkg` refusing to build when a local source file
  referenced in `source=()` is missing).
- `sha256sums`/`patch_sha256sums`, if present, must be the same length as
  their corresponding `sources`/`patches` array — this is the single most
  common `makepkg` PKGBUILD authoring mistake (arrays drifting out of sync
  after an edit) and should produce a precise "entry 3 of sha256sums has no
  matching source" style error, not a generic length mismatch.
- Every `Remote` source without a corresponding non-`"SKIP"` checksum
  produces a **warning**, not a hard error, at `validate` time — mirrors
  `makepkg`'s own default behavior (it warns loudly and lets you build
  anyway unless a flag like `--verifysource` implies otherwise). `makerpm
  build` should require `--allow-unverified` (or similar) to proceed past
  this warning without an interactive confirmation, to avoid silently
  building from unverified downloads by default.
- Every subpackage must have non-empty `summary`, `description`, and at
  least one entry across `files.{paths,docs,licenses,configs*,dirs}`.
- Subpackage `suffix` values must be unique and non-empty.
- If `build.system` is set to something other than `none`, and the
  corresponding `BuildRequires` isn't already present in `deps.build_depends`,
  auto-inject it (log at info level that this happened) rather than error —
  this is the "don't let it fail in a clean mock root" guarantee from the
  design discussion.
- If `build.steps.check` is set, and `build.run_tests` is unset, default it
  to `true`; if `run_tests = false` explicitly, `%check` is omitted even if
  `steps.check` has content (with a warning).
- `changelog` should be non-empty; warn (not error) if empty, since
  `rpmbuild`/`rpmlint` will separately flag this — don't duplicate rpmbuild's
  own error surface unnecessarily, just nudge early.
- Every file path in `files.*` across package + subpackages combined should
  not have unexplained overlaps (the same path claimed by two subpackages)
  — this is a common real-world spec bug (`rpmbuild` fails at build time
  with "file listed twice" but catching it in validation gives a much
  better error with TOML-level context).

---

## 6. Build-system macro lookup table

Central table (module `spec_gen` or `model/build.rs`) mapping
`BuildSystem` → `(required_build_requires: Vec<&str>, build_macro: &str,
install_macro: &str)`:

| `BuildSystem` | `BuildRequires` added | `%build` | `%install` |
|---|---|---|---|
| `None` | (none) | *entirely from `steps.build`* | *entirely from `steps.install`* |
| `Make` | (none) | `%make_build` | `%make_install` |
| `Autotools` | `autoconf`, `automake`, `libtool` (only if no `configure` script present — otherwise skip; detect at prep time or just always add and let it be a no-op) | `%configure <extra_build_args>` then `%make_build` | `%make_install` |
| `Cmake` | `cmake`, `gcc-c++` | `%cmake <extra_build_args>` then `%cmake_build` | `%cmake_install` |
| `Meson` | `meson`, `ninja-build` | `%meson <extra_build_args>` then `%meson_build` | `%meson_install` |
| `Cargo` | `rust-packaging` (provides cargo macros) | `%cargo_build <extra_build_args>` | `%cargo_install` |
| `PythonPyproject` | `python3-devel`, `pyproject-rpm-macros` | `%pyproject_build <extra_build_args>` | `%pyproject_install` |

Each row's macro invocation is followed immediately (same section) by the
raw shell from `build.steps.build` / `build.steps.install` if present, so
"macro handles the 90% case, TOML shell handles the rest" holds even when
`system` isn't `none`.

---

## 7. Spec rendering (`spec_gen`)

- Pure function: `fn render(spec: &PkgSpecFile) -> Result<String, RenderError>`.
- No filesystem access, no process execution — this makes it trivially
  snapshot-testable with `insta` against fixture TOMLs.
- Template written in Tera, structured to match spec-file section order:
  preamble → `%description` → per-subpackage `%package`/`%description`
  blocks → `%prep` → `%build` → `%install` → `%check` → per-package
  `%files` blocks (base + subpackages) → per-package scriptlet blocks →
  `%changelog`.
- `Release` field: always render as given, but default to `1%{?dist}` at
  the model layer (§4.3) rather than in the template, so the template
  logic stays simple string substitution wherever possible.
- Escape/guard against TOML string content that could break spec syntax
  (e.g. a `%` in a description should probably be escaped as `%%` — decide
  this explicitly and test it, since raw `%` in RPM spec text is a macro
  invocation attempt and a common source of confusing build failures).

---

## 8. Source fetching (`fetch.rs`) — makepkg-style downloads

This runs after validation and before the rpmbuild tree is populated. Goal:
reproduce the exact "just works" feel of `makepkg`'s download step, where a
user with a fresh checkout of a PKGBUILD-like directory runs one command and
never manually invokes `curl`/`wget` themselves.

### 8.1 Source cache directory

- Default cache location: `~/.cache/makerpm/sources/` — analogous to
  makepkg's `SRCDEST` (when set) or its default of downloading straight into
  the package build directory. A shared, name-spaced cache is friendlier for
  repeat builds of the same package across versions, so prefer a cache keyed
  by `<filename>-<sha256sums-entry-or-"nocsum">` (or just `<filename>` if no
  checksum, accepting a possible stale-cache edge case documented as a known
  v1 limitation).
- Respect a `MAKERPM_SRCDEST` environment variable to override the cache
  root, mirroring makepkg's `SRCDEST`.
- `--offline` CLI flag: never touch the network; any `Remote` source not
  already present (and checksum-valid, if a checksum is declared) in the
  cache is a hard error. This is the offline/CI-reproducibility escape
  hatch makepkg users expect from `--noextract`/pre-populated `SRCDEST`
  workflows.

### 8.2 Fetch algorithm (per source entry, run sequentially — matches
   makepkg's own sequential, one-download-bar-at-a-time UX)

1. Parse the entry via `source_spec::parse_source_entry`.
2. If `Local`, resolve the path relative to the TOML directory; nothing to
   fetch (already validated to exist in §5).
3. If `Remote`:
   a. Compute the cache path for `filename`.
   b. If present in cache **and** a checksum is declared **and** it matches
      → reuse, skip download entirely (this is the single biggest UX win
      over a naive implementation — repeat `makerpm build` runs during
      iterative packaging shouldn't re-download every time, exactly like
      makepkg's cache-and-reuse behavior).
   c. If present in cache but checksum doesn't match (or none declared and
      `--refetch`/`--force-download` was passed) → re-download.
   d. If absent → download via `ureq::get(url).call()`, streaming to a
      `.part` temp file in the cache dir with an `indicatif` progress bar
      keyed off `Content-Length` when present, then atomically rename to
      the final cache filename on success (never leave a half-downloaded
      file at the real cache path — a crashed download must not poison
      future "already cached" checks).
   e. After download (or on cache hit), verify against the declared
      `sha256sums` entry if present (skip if `"SKIP"` or absent, per §5's
      warning-not-error stance). Checksum mismatch is always a hard error,
      never downgraded — this is a security-relevant check and should never
      be silently bypassed except via the explicit `--skip-checksums` flag
      described below.
4. Once every source/patch entry is resolved to a real local path (cache
   entry or in-tree local file), pass that resolved-path list into
   `build_tree.rs`'s existing "copy into `SOURCES/`" step (§9) — `fetch.rs`'s
   only output is "here is where each declared source now lives on disk,"
   keeping `build_tree.rs` blissfully unaware of whether a file was local
   all along or just downloaded.

### 8.3 CLI flags introduced by this section

- `--offline` — never touch the network; fail if an uncached remote source
  is needed.
- `--skip-checksums` — proceed even on checksum mismatch, printing a loud
  warning per mismatched file (equivalent to makepkg's `--skipinteg`/
  `--skipchecksums`). Off by default; mismatches are hard errors otherwise.
- `--refetch` — ignore the cache and re-download every remote source
  unconditionally (equivalent to makepkg forcing a fresh download).
- `--allow-unverified` — proceed past the §5 "remote source with no
  declared checksum" warning without an interactive prompt (useful for
  CI); interactive terminals without this flag should prompt for
  confirmation once, listing which sources are unverified.

### 8.4 Extension point for v2 (VCS sources — not implemented in v1)

Keep `SourceEntry` as an enum specifically so a future `Vcs { kind: VcsKind,
url: String, reference: Option<String> }` variant (for makepkg-style `git+`,
`svn+`, etc. prefixes) can be added without reshaping `source_spec.rs`'s
callers — `fetch.rs`'s per-entry `match` is the only place that would need
a new arm.

---

## 9. Build orchestration (`build_tree.rs` + `runner.rs`)

1. Determine a working root, e.g. `~/.cache/makerpm/<pkgname>/` or a
   `.makerpm/` dir next to the TOML (pick one, document it — `makepkg`
   equivalent is `src/`/`pkg/` next to `PKGBUILD`, so a local `.makerpm/`
   sibling directory is probably the more familiar UX).
2. Create the standard rpmbuild tree layout inside it:
   `BUILD/ BUILDROOT/ RPMS/ SOURCES/ SPECS/ SRPMS/`.
3. Copy (not symlink, to avoid `rpmbuild` surprises with source
   permissions) every resolved source/patch path from §8's fetch step
   (whether it was a local file all along or just downloaded into the
   cache) into `SOURCES/`.
4. Write the rendered spec to `SPECS/<name>.spec`.
5. Invoke:
   ```
   rpmbuild --define "_topdir <working_root>" -ba SPECS/<name>.spec
   ```
   Stream stdout/stderr live (via `tracing` or direct passthrough) rather
   than buffering — build output is long and users expect to watch it, the
   same way `makepkg` streams `make` output.
6. On success, glob `RPMS/**/*.rpm` and `SRPMS/*.src.rpm`, copy them back
   to the invocation directory (or an `--output-dir` if specified), and
   print a summary list — mirroring makepkg's final "built package ...:
   `<name>-<version>-<release>.<arch>.rpm`" message.
7. On failure, surface `rpmbuild`'s exit code and tail of stderr clearly;
   do not attempt to parse/interpret `rpmbuild` errors in v1 — just pass
   them through faithfully. (A future version could pattern-match common
   `rpmbuild` failures into friendlier messages, but that's out of scope
   here.)

---

## 10. CLI surface

```
makerpm build [--spec-file PKGSPEC.toml] [--output-dir DIR]
              [--offline] [--refetch] [--skip-checksums] [--allow-unverified]
              [-v|-vv]
makerpm fetch [--spec-file PKGSPEC.toml] [--offline] [--refetch]  # download/verify sources only, no build
makerpm spec  [--spec-file PKGSPEC.toml] [--output FILE]   # render .spec only, no build, no download
makerpm validate [--spec-file PKGSPEC.toml]                # parse + validate, exit code only
makerpm init [--name NAME]                                 # scaffold a starter PKGSPEC.toml
```

`--spec-file` defaults to `./PKGSPEC.toml` in the current directory,
mirroring how `makepkg` defaults to `./PKGBUILD`. The standalone `fetch`
subcommand is the equivalent of `makepkg -o` (fetch/extract sources
without building) — useful for pre-populating the cache before an offline
build, or for CI layers that separate network access from the build step.

---

## 11. Testing strategy

- **Unit tests** for the `BuildSystem` macro table and `DependencySet`
  rendering (pure string logic, no fixtures needed).
- **Unit tests** for `source_spec::parse_source_entry` against every row of
  the §4.4 table (bare filename, bare URL, `name::url` rename form,
  malformed input).
- **Unit tests** for `fetch.rs`'s cache-hit/cache-miss/checksum-mismatch
  logic against a local mock HTTP server (e.g. spin up a tiny `ureq`-
  compatible test server, or abstract the download behind a trait so a
  fake implementation can be swapped in for tests — this keeps fetch logic
  testable without real network access, which matters if the LLM
  implementing this runs in a sandboxed environment).
- **Snapshot tests** (`insta`) for `spec_gen::render`: a handful of fixture
  TOMLs (simple no-subpackage package; cmake-based package; package with
  two subpackages sharing dependencies; package with custom scriptlets) →
  committed expected `.spec` output. Any template change that alters
  output requires reviewing and re-approving the snapshot — this is the
  main regression safety net for the project.
- **Integration test** (`tests/integration_test.rs`), gated behind a
  feature flag or `#[ignore]` by default (since it requires `rpmbuild` on
  the CI runner): build the `examples/hello-world` fixture end-to-end and
  assert an `.rpm` file is produced.
- **Validation tests**: fixture TOMLs deliberately containing each error
  class from §5, asserting the correct diagnostic fires.

---

## 12. Suggested implementation order (milestones)

1. **Model + parsing**: structs from §4, `toml`/`serde` deserialization,
   no validation yet. Get a TOML file loading into a `PkgSpecFile` with
   good `miette` errors on malformed TOML.
2. **`source_spec.rs`** (§4.4): pure parsing logic for the `sources`/
   `patches` grammar, fully unit-testable with zero I/O — good early
   milestone since it's small and self-contained.
3. **Validation pass** (§5) as a separate step, with tests for each rule.
4. **Spec rendering** (§7) as a pure function with the Tera template,
   covering the no-subpackage case first, then adding subpackage looping.
5. **Snapshot tests** locked in early, once rendering is stable enough to
   commit expected output — this protects all later refactors.
6. **Build-system macro table** (§6) wired into the renderer.
7. **`makerpm spec` subcommand** — CLI wrapper around steps 1–6, no build
   or download invocation yet. This alone is independently useful and
   testable.
8. **`fetch.rs`** (§8): cache directory handling, download-behind-a-trait
   for testability, checksum verification. Wire up `makerpm fetch` as a
   standalone subcommand once this works in isolation.
9. **Build tree setup + rpmbuild invocation** (§9), consuming `fetch.rs`'s
   resolved source paths.
10. **`makerpm build` subcommand**, wiring everything together (parse →
    validate → fetch → render → build tree → rpmbuild → collect).
11. **`makerpm validate` and `makerpm init` subcommands** (small, do last).
12. **End-to-end example package** (`examples/hello-world`) used both as
    documentation and as the integration test fixture — worth adding a
    second fixture here that uses a `Remote` source (pointed at a small,
    stable, versioned tarball URL) to exercise the fetch path end-to-end.

Each milestone should be independently mergeable. Milestones 1–7 require
neither `rpmbuild` nor real network access to develop or test (fetch logic
is tested against a mock/trait-based download backend per §11); only
milestones 9–10 need the real `rpmbuild` binary and network access —
useful if the LLM implementing this is working in a sandboxed environment
without RPM tooling or outbound network access.

---

## 13. Example `PKGSPEC.toml` (target shape, for the `init` template and first fixture)

```toml
[package]
name = "hello-world"
version = "1.0"
summary = "A friendly greeting program"
license = "MIT"
url = "https://example.org/hello-world"
description = """
hello-world prints a friendly greeting to standard output.
"""

# Downloaded automatically by `makerpm build` (or pre-fetched via
# `makerpm fetch`), cached locally, and checksum-verified before use —
# exactly like makepkg's source=()/sha256sums=() pair.
sources = ["https://example.org/releases/hello-world-1.0.tar.gz"]
sha256sums = ["9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a1"]

[package.build]
system = "make"

[package.files]
paths = ["%{_bindir}/hello-world"]
docs = ["README.md"]
licenses = ["LICENSE"]

[[package.changelog]]
version = "1.0-1"
date = "2026-06-29"
packager = "Jane Doe <jane@example.org>"
entries = ["Initial package"]
```

This should render to a spec using `%make_build`/`%make_install`, a single
`%files` section, and no `%package`/`%description` subpackage blocks —
the minimal end-to-end path the whole pipeline needs to support before
subpackages are layered on. A second fixture (per Phase 4 in §14) should
additionally exercise a `Local` source and a `name::url` rename-form
`Remote` source, to cover all three grammar shapes from §4.4 in one place.

---

## 14. Phased execution plan — prompts and exit checklists

Split implementation into four phases, each independently reviewable and
each ending at an objectively checkable point (tests pass, a specific
subcommand works) rather than "code was written." Hand each phase's prompt
to the LLM as a self-contained unit of work, only starting the next phase
once the current one's exit checklist is fully green. Each prompt below
assumes the LLM has access to this whole plan document as context.

### Phase 1 — Model & parsing

**Covers:** §4 (data model), `parse.rs`, `source_spec.rs` (§4.4),
`validate.rs` (§5). No filesystem access beyond reading the TOML/checking
paths exist, no network, no process execution, no spec rendering yet.

**Prompt:**
> Implement Phase 1 of the makerpm plan: the data model (§4), TOML parsing
> (`parse.rs`), the source-entry grammar parser (`source_spec.rs`, §4.4),
> and the validation pass (`validate.rs`, §5). Use `serde`/`toml` for
> deserialization and `miette`/`thiserror` for diagnostics. Do not implement
> spec rendering, fetching, or rpmbuild invocation in this phase — stub
> `main.rs` with just enough CLI wiring (via `clap`) to run `makerpm
> validate` and see parse/validation errors printed clearly. Write unit
> tests for every validation rule in §5 (one fixture TOML per rule,
> asserting the specific diagnostic fires) and every row of the §4.4
> source-syntax table. Do not write networking or `rpmbuild`-invoking code
> yet, even stubbed — keep this phase's surface area to parsing and
> validation only.

**Exit checklist:**
- [ ] `PkgSpecFile`, `Package`, `Subpackage`, and all shared structs from
      §4.1–4.3 exist and deserialize correctly from TOML.
- [ ] `source_spec::parse_source_entry` passes unit tests for all three
      grammar shapes in §4.4 (local filename, bare URL, `name::url` rename).
- [ ] Every validation rule in §5 has at least one fixture TOML that
      triggers it and one test asserting the correct diagnostic.
- [ ] Malformed TOML produces a `miette`-rendered error pointing at the
      offending line/field, not a raw serde error dump.
- [ ] `makerpm validate --spec-file <fixture>.toml` runs from the CLI and
      exits non-zero with a clear message on any fixture with a known
      validation problem, zero on a clean fixture.
- [ ] No code in this phase touches the network, spawns a process, or
      writes files other than reading the TOML/checking source paths exist.

### Phase 2 — Spec generation

**Covers:** §6 (build-system macro table), §7 (`spec_gen`, Tera template),
the `makerpm spec` subcommand. Consumes Phase 1's model directly; still no
network, no process execution.

**Prompt:**
> Implement Phase 2 of the makerpm plan: the build-system macro lookup
> table (§6) and the spec-rendering module (`spec_gen`, §7), using `tera`
> for templating. `spec_gen::render` must be a pure function (model in,
> `.spec` string out, no I/O) so it can be snapshot-tested with `insta`.
> Cover the no-subpackage case first, then extend the template to loop over
> `[package] + [[subpackage]]` uniformly per the "package as subpackage
> zero" design note in §4.3. Wire up the `makerpm spec` subcommand (parses
> + validates via Phase 1's code, then renders and prints/writes the
> `.spec`). Add `insta` snapshot tests for: a plain no-subpackage package,
> a `cmake`-based package, a package with two subpackages sharing
> dependencies, and a package with custom scriptlets. Do not implement
> fetching or rpmbuild invocation in this phase.

**Exit checklist:**
- [ ] `spec_gen::render` has zero filesystem/network/process access —
      confirm by inspection, not just by test behavior.
- [ ] All four `insta` snapshot fixtures listed in the prompt exist and
      pass, with the committed `.snap` files reviewed by hand at least once
      for idiomatic spec-file correctness (correct section order, correct
      macro usage per §6's table, correct `%files`/`%package` structure).
- [ ] Every `BuildSystem` variant from §4.2 has a working macro-table entry
      and at least one snapshot or unit test exercising it.
- [ ] `%` characters in free-text TOML fields (description, changelog
      entries) are correctly escaped in rendered output — add an explicit
      test for this.
- [ ] `makerpm spec --spec-file <fixture>.toml` runs end-to-end and
      produces a `.spec` file that is at least visually well-formed (worth
      manually eyeballing one full output against a real Fedora spec file).
- [ ] Missing `BuildRequires` auto-injection (§5's rule for `build.system`)
      is visible in rendered output — verify a `cmake`-system fixture's
      rendered spec actually contains `BuildRequires: cmake`.

### Phase 3 — Source fetching

**Covers:** §8 in full (`fetch.rs`, cache directory, checksum verification,
`--offline`/`--refetch`/`--skip-checksums`/`--allow-unverified`), the
`makerpm fetch` subcommand. This is the only phase touching the network.

**Prompt:**
> Implement Phase 3 of the makerpm plan: source fetching (`fetch.rs`, §8).
> Abstract the actual download behind a small trait (e.g. `Downloader`)
> so tests can substitute a fake/mock implementation instead of making real
> HTTP calls — real code should use `ureq` behind this trait. Implement the
> cache directory logic (§8.1, respecting `MAKERPM_SRCDEST`), the per-source
> fetch algorithm (§8.2, including atomic rename-after-download and
> cache-hit reuse when checksums match), and checksum verification with
> `sha2`. Wire up the `--offline`, `--refetch`, `--skip-checksums`, and
> `--allow-unverified` flags from §8.3 and the standalone `makerpm fetch`
> subcommand. Write unit tests against the mock `Downloader` covering:
> cache hit with matching checksum (no download attempted), cache miss
> (download attempted), checksum mismatch (hard error unless
> `--skip-checksums`), and offline mode with an uncached remote source
> (hard error). Do not implement rpmbuild invocation in this phase.

**Exit checklist:**
- [ ] `fetch.rs`'s download behavior is fully testable without real network
      access, via the `Downloader` trait abstraction.
- [ ] Cache-hit, cache-miss, checksum-mismatch, and offline-failure paths
      each have a passing unit test.
- [ ] A crashed/interrupted download never leaves a corrupt file at the
      final cache path (verify the `.part`-file-then-atomic-rename logic
      with a test that simulates a failure mid-download).
- [ ] `makerpm fetch --spec-file <fixture-with-a-real-small-stable-url>.toml`
      succeeds against one real URL (manual/CI-gated check, not a unit
      test) and populates the cache correctly.
- [ ] Re-running `makerpm fetch` on the same fixture is a no-op (cache hit,
      no re-download) — confirm via logging/output, not just absence of
      errors.
- [ ] `--skip-checksums` and `--allow-unverified` each visibly change
      behavior in a dedicated test/manual run; without them, the
      corresponding failure modes are hard errors.

### Phase 4 — Build orchestration & final wiring

**Covers:** §9 (`build_tree.rs`, `runner.rs`), the full `makerpm build`
subcommand tying Phases 1–3 together, `makerpm init`, and the end-to-end
example package (§13). This is the only phase needing a real `rpmbuild`
binary on the machine.

**Prompt:**
> Implement Phase 4 of the makerpm plan: the rpmbuild tree setup and
> invocation (`build_tree.rs` + `runner.rs`, §9), consuming Phase 3's
> resolved source paths and Phase 2's rendered spec. Implement the full
> `makerpm build` subcommand as the pipeline parse → validate → fetch →
> render → stage `SOURCES/`+`SPECS/` → invoke `rpmbuild --define
> "_topdir ..." -ba` → collect resulting RPMs into an output directory,
> streaming `rpmbuild`'s stdout/stderr live. Implement `makerpm init` to
> scaffold a starter `PKGSPEC.toml` matching §13's example. Build the
> `examples/hello-world` fixture end-to-end, including a second fixture
> that exercises a real downloaded `Remote` source per §13's note, and add
> an integration test (gated behind `#[ignore]` or a feature flag, since it
> needs `rpmbuild` installed) asserting a real `.rpm` file is produced.

**Exit checklist:**
- [ ] `makerpm build` on the `examples/hello-world` fixture produces a
      valid `.rpm` on a machine with `rpm-build` installed, with no manual
      spec editing.
- [ ] The second fixture exercising a real `Remote` source builds
      end-to-end, proving Phase 3 and Phase 4 are correctly wired together.
- [ ] Build failures from `rpmbuild` are surfaced with exit code and
      stderr tail intact — verify with a deliberately broken fixture.
- [ ] Output RPMs (and `.src.rpm` if requested) land in the expected
      output directory, named as `rpmbuild` names them, not renamed/altered.
- [ ] `makerpm init` produces a TOML that itself passes `makerpm validate`
      unmodified.
- [ ] The `#[ignore]`-gated integration test is documented (in the test
      file or a README) as requiring `rpmbuild` to be installed to run.

Once Phase 4's checklist is green, the project meets the "Definition of
done for v1" from §1.

---

## 15. Final QA pass — prompt for an independent reviewing LLM

Run this after all four phases are complete and merged, ideally with a
**different** LLM session/context than the one that implemented the code —
a reviewer that didn't write the code is less likely to rubber-stamp its
own assumptions. Give the reviewer the full codebase, this plan document,
and the prompt below. Ask for a written report, not just a pass/fail.

**Prompt:**
> You are performing an independent QA review of `makerpm`, a Rust CLI tool
> that transpiles a TOML package spec into an RPM `.spec` file, downloads
> and verifies declared sources, and invokes `rpmbuild` to build packages.
> You have access to the full source tree and the project's implementation
> plan (this document). Review the codebase — not the plan, the actual
> code — against the three categories below. For every issue found, report:
> the file and location, a description of the problem, its severity
> (critical / high / medium / low), and a concrete suggested fix. Do not
> limit yourself to the checklist items below — they are a floor, not a
> ceiling; flag anything else that looks wrong. Where you are uncertain
> whether something is actually a bug versus an intentional design choice
> from the plan, say so explicitly rather than asserting it's wrong.

### 15.1 Security

- **Command/shell injection**: every place `rpmbuild` (or any other
  process) is invoked — confirm it uses `Command::new(...).arg(...)` with
  separate arguments, never a shell string built via `format!`. Check
  specifically that no TOML-sourced value (package name, version, any
  `build.steps.*` shell content) is interpolated into a shell command line
  outside of the generated `.spec` file itself.
- **Path traversal**: TOML-sourced filenames (`sources`, `patches`,
  `files.*` paths, subpackage `suffix`, package `name`) must be checked
  before being used to construct filesystem paths — a malicious or
  careless `"../../etc/foo"` in any of these fields should not let files be
  written outside the intended working/cache/output directories. Check
  `fetch.rs`'s cache path construction and `build_tree.rs`'s staging step
  specifically.
- **SSRF / arbitrary URL fetch**: `fetch.rs` will fetch whatever URL a TOML
  file declares. Confirm this is treated as an accepted, documented
  capability (the tool's whole job is to download declared sources) rather
  than silently restricted — but check that redirects are handled sanely
  (no infinite redirect loops, a sane redirect count cap) and that the
  downloaded content size doesn't grow unbounded in memory (should stream
  to disk, not buffer the whole file).
- **TOCTOU on cache files**: confirm the "check cache, maybe download,
  verify checksum, then use" sequence in §8.2 can't be tricked by a file
  changing between the checksum check and the copy into `SOURCES/` —
  ideally checksum verification happens on the exact file that gets
  copied, not a separate read.
- **Checksum bypass paths**: confirm `--skip-checksums`/missing-checksum
  handling matches §5/§8.3 exactly — a checksum mismatch must be a hard
  error by default, and the only way to proceed is an explicit,
  user-supplied flag, never a silent fallback.
- **Atomic download writes**: confirm downloads write to a temp/`.part`
  file and are renamed only on full success (§8.2d) — a partial file at
  the real cache path due to a crash or interrupted download must not be
  treated as valid on a subsequent run.
- **Unsafe/`unwrap`/`expect` on untrusted input**: any `.unwrap()` or
  `.expect()` reachable from TOML content, filesystem content, or network
  responses (as opposed to internal invariants the code itself
  established) is a potential panic-as-DoS bug — flag every instance and
  assess whether it's reachable from attacker/user-controlled input.
- **Dependency audit**: check `Cargo.lock` / `cargo audit` (if runnable)
  for known-vulnerable crate versions, particularly in the networking
  (`ureq`, `url`) and archive-adjacent dependencies.

### 15.2 Idiomatic Rust

- `cargo clippy --all-targets --all-features` run clean, or every
  remaining lint explicitly and deliberately allowed with a comment
  explaining why.
- Error handling is consistent: library-ish modules (`parse`, `validate`,
  `source_spec`, `spec_gen`, `fetch`) return `Result<T, E>` with meaningful
  error types (`thiserror`-derived enums, not stringly-typed errors or
  `anyhow::Error` leaking out of internal APIs); only `main.rs`/CLI-facing
  code should collapse everything into a final user-facing report.
  `.unwrap()`/`.expect()` should be rare and justified, not a substitute
  for proper error propagation.
- Ownership is sensible: no unnecessary `.clone()` calls where a borrow
  would do, no `Rc<RefCell<...>>` reached for where a plain owned value or
  a straightforward borrow-checker-friendly restructuring would work.
- Module boundaries match §3's intended layout — confirm `spec_gen`
  genuinely has zero I/O (no `std::fs`, no `std::process`, no network
  crate usage reachable from `render()`), and that `fetch.rs`'s download
  behavior is truly behind the `Downloader` trait rather than leaking a
  concrete `ureq` call into code that's supposed to be testable via a mock.
  This is a correctness-of-testability check, not just a style nit.
  Confirm `source_spec.rs` has zero I/O.
- Naming and API shape follow Rust conventions (`snake_case` functions,
  `CamelCase` types, builder patterns or `Default`+struct-update where
  appropriate for the many-optional-field structs in §4, no stringly-typed
  enums where a real `enum` was called for in the plan, e.g. `BuildSystem`).
- No needless `pub` — internal helper functions/types shouldn't be part of
  the public API surface unless there's a reason for downstream consumers
  (e.g. if `makerpm` is ever used as a library) to reach them.
- Tests are meaningful, not just present: spot-check that `insta` snapshot
  tests actually assert on rendered content (not just "doesn't panic"), and
  that mocked `Downloader` tests in Phase 3 actually exercise the
  cache-hit/miss/mismatch branches distinctly rather than one test loosely
  covering all of them.

### 15.3 Conformance to the plan

- **Field mapping completeness**: cross-check every TOML field named in §4
  against the spec-rendering code — confirm every field actually reaches
  the generated `.spec` file somewhere, and nothing is silently dropped
  (a field that parses successfully but has no effect on output is a
  common class of bug in generator tools like this).
- **Validation rules**: every rule listed in §5 has corresponding
  implemented logic — not just a test fixture that happens to pass, but an
  actual check in `validate.rs` doing the stated work.
- **Build-system macro table**: every row of §6's table is correctly
  implemented — for each `BuildSystem` variant, confirm the exact macro
  invocations and injected `BuildRequires` match the table, not just "some
  plausible-looking cmake incantation."
- **Source grammar**: all three shapes in §4.4's table parse to the
  correct `SourceEntry` variant, including edge cases (a URL that happens
  to contain `::` in a query string, a filename that looks like it could be
  parsed as a URL but isn't, an empty string).
- **CLI surface**: every flag listed in §10 exists, is wired to the
  behavior described in §8.3/§9, and un-set defaults match what's
  documented (e.g. checksum mismatches are hard errors *by default*,
  `--offline` truly never touches the network under any code path).
- **Definition of done (§1)**: actually attempt the end-to-end scenario —
  a directory with only a `PKGSPEC.toml` (no pre-fetched sources) — and
  confirm `makerpm build` produces valid `.rpm` files with zero manual
  intervention, on a machine with `rpm-build` installed.
- **Non-goals honored**: confirm nothing in the codebase attempts
  dependency-repository resolution, VCS-style source fetching, or package
  installation — any code reaching toward those is scope creep beyond what
  the plan called for, not a bonus feature, and should be flagged as such.

Deliver the review as a single report grouped by category (15.1/15.2/15.3),
each finding tagged with severity, so the issues can be triaged and fixed
in priority order rather than addressed in whatever order they were found.

