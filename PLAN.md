# makerpm — Implementation Plan (v2)

Builds on `PLAN-v1.md`. v1 delivered `makerpm build`/`fetch`/`spec`/
`validate`/`init`. v2 restructures the CLI around four subcommands and adds
two importers that produce a *draft* `RPMSPEC.toml` from an existing
packaging format, for manual review before building.

```
makerpm build  RPMSPEC.toml
makerpm lint   RPMSPEC.toml
makerpm import deb ./path/to/debian/source/ -o RPMSPEC.toml
makerpm import arch ./PKGBUILD -o RPMSPEC.toml
```

---

## 1. Scope and non-goals for v2

### In scope

1. CLI restructure: positional `<PATH>` argument instead of v1's
   `--spec-file` flag, matching the shape in the prompt above.
2. `makerpm lint` — v1's `validate` renamed and extended with best-practice
   warnings (not just structural correctness), with severity levels.
3. `makerpm import deb <dir> -o <path>` — parses an **already-extracted**
   Debian source package directory (i.e. `dpkg-source -x` has already been
   run; `debian/` sits next to the upstream source tree) and emits a draft
   `RPMSPEC.toml` (or another path passed to `-o`).
4. `makerpm import arch <PKGBUILD> -o <path>` — parses a `PKGBUILD` file and
   emits a draft `RPMSPEC.toml` (or another path passed to `-o`).

### Explicit non-goals for v2

- **No execution of untrusted input.** Neither importer sources, evals, or
  runs any part of the input (no `bash -c "source PKGBUILD"`, no running
  `debian/rules`). Both are implemented as static parsers over a restricted
  grammar. This is a hard security requirement, not a style preference —
  see §4.1 for the reasoning.
- **No guarantee of a working spec on first import.** Cross-distro
  translation (dependency names, file paths, build systems) is inherently
  lossy — Debian and Fedora don't agree on package names, library paths,
  or build tooling conventions. The importers' job is to produce a
  well-organized *starting point* with every uncertain field clearly
  flagged, not a build-ready spec. `makerpm lint` should still be run (and
  will likely still warn) after every import.
- **No dynamic `pkgver()` evaluation** and **no `debian/rules` execution**
  — both are effectively arbitrary code; where either is detected, the
  importer emits a placeholder value plus a `# TODO` comment rather than
  attempting to compute the real one.
- **No multi-architecture dependency merging beyond x86_64 + generic.**
  Arch's `depends_aarch64`-style per-arch arrays and Debian's
  architecture-conditional `Build-Depends` are out of scope for v2;
  document this as a known limitation.
- Does not touch `makerpm build`/`fetch`/`spec` behavior beyond the CLI
  argument shape — v1's pipeline logic is otherwise unchanged.

---

## 2. CLI restructure (breaking change from v1)

v1 used the filename `PKGSPEC.toml`, passed through `--spec-file` and
defaulting to `./PKGSPEC.toml` on every subcommand. v2 renames the canonical
project file to `RPMSPEC.toml` so its purpose is immediately recognizable as
RPM-specific; the TOML schema itself is unchanged by this naming transition.
At the same time, v2 switches to a positional path argument to match the shape
requested for v2 and to read more naturally as a verb-object command, the way
`makepkg`/`dpkg-buildpackage`-adjacent tools typically don't need a flag for
the one file they obviously operate on.

```
makerpm build <PATH> [--output-dir DIR] [--offline] [--refetch]
              [--skip-checksums] [--allow-unverified] [-v|-vv]
makerpm lint  <PATH> [--strict]
makerpm fetch <PATH> [--offline] [--refetch]
makerpm spec  <PATH> [--output FILE]
makerpm init  [--name NAME] [-o PATH]
makerpm import deb <SOURCE_DIR> -o <PATH> [--force]
makerpm import arch <PKGBUILD_PATH> -o <PATH> [--force]
```

- `<PATH>` defaults to `./RPMSPEC.toml` when omitted; only the canonical
  filename changes from v1's `./PKGSPEC.toml`.
- `validate` is renamed to `lint`; keep `validate` as a hidden alias for one
  release (`#[command(alias = "validate")]` in `clap`) so v1 muscle memory
  and any scripts aren't broken outright.
- `import`'s `-o <PATH>` is required (no sensible default — importers
  should never silently overwrite `./RPMSPEC.toml`); `--force` is required
  to overwrite an existing file at `-o`'s path, otherwise error out.
- This is the first task of Phase 5 (§7) since every other v2 piece hangs
  off subcommand structure.

---

## 3. `makerpm lint` — severity levels and new rules

v1's `validate` already implemented hard structural checks (§5 of v1's
plan: missing fields, array-length mismatches, duplicate suffixes, and so
on). v2 reframes these as `Severity::Error` and adds a second tier of
`Severity::Warning` best-practice checks — the same relationship `clippy`
has between hard compile errors and lints.

```rust
pub enum Severity { Error, Warning }

pub struct LintFinding {
    pub severity: Severity,
    pub field_path: String,   // e.g. "package.license", "subpackage[1].files"
    pub message: String,
    pub suggestion: Option<String>,
}
```

- `makerpm lint <PATH>` runs all checks, prints every finding grouped by
  severity, and exits non-zero only if any `Error`-severity finding exists.
- `--strict` makes `Warning` findings exit non-zero too — useful for CI.
- All of v1's §5 rules become `Error`-severity findings, unchanged in
  behavior. New `Warning`-severity rules to add in v2:
  - `license` doesn't parse as a plausible SPDX expression (v1 already
    flagged this as a warning — keep it here as-is).
  - `changelog` is empty.
  - A subpackage's `summary` is character-for-character identical to the
    main package's `summary` (common copy-paste mistake).
  - `release` doesn't end in `%{?dist}` (Fedora convention, not a hard
    RPM requirement).
  - `description` (package or any subpackage) is shorter than ~10
    characters — likely a placeholder left unedited.
  - Any `# TODO` comment is present in the raw TOML text (see §4.3) —
    surfaces leftover import-review items directly in `lint` output so
    `makerpm import` → `makerpm lint` is a natural two-step workflow.

---

## 4. Shared import infrastructure

Both importers produce the same output shape (an `RPMSPEC.toml` draft) and
share the same core design constraints, so build the shared pieces first
and let each importer be a thin frontend over them.

### 4.1 Why static parsing only, never execution

Both `PKGBUILD` and `debian/rules` are, formally, executable shell/Make
scripts — sourcing or running either one runs arbitrary code with the
importing user's privileges. A tool whose explicit purpose is to ingest
*other people's packaging files*, often sight-unseen before review, cannot
safely do this. This is a hard constraint carried over from the v1 QA
prompt's security stance and applies with more force here:

- `import arch` parses `PKGBUILD` as **text**, using a restricted grammar
  (§6.1) that recognizes only variable assignments and named function
  bodies as opaque text blocks. It never invokes `bash`.
- `import deb` never executes `debian/rules`. Build-system detection is
  heuristic, based on `debian/control`'s `Build-Depends` and the presence
  of marker files in the source tree (`CMakeLists.txt`, `meson.build`,
  `configure.ac`, `Cargo.toml`), not by interpreting the Makefile (§5.3).

If a future release wants higher-fidelity extraction via actual execution,
that must be sandboxed (container/`bwrap`) and opt-in, and is out of scope
for v2.

### 4.2 Draft model and confidence annotations

Reuse v1's `PkgSpecFile`/`Package`/`Subpackage` structs directly — do not
create a parallel "draft" struct. Instead, thread a side-channel of
per-field annotations through the import process:

```rust
pub struct ImportDraft {
    pub spec: PkgSpecFile,
    pub notes: Vec<ImportNote>,
}

pub struct ImportNote {
    pub field_path: String,       // matches LintFinding's field_path convention
    pub note: String,             // human-readable explanation
    pub confidence: Confidence,
}

pub enum Confidence {
    Confident,     // e.g. name/version/license direct field copy
    BestEffort,    // e.g. heuristic build-system detection, name translation
    Unsupported,   // e.g. dynamic pkgver(), unparsed debian/rules logic
}
```

`Confident` fields are written to the TOML with no annotation.
`BestEffort` and `Unsupported` fields get a `# TODO:` comment immediately
above them in the written file, sourced from `ImportNote::note`. This is
the single most important UX property of both importers: **a reviewer
should be able to `grep TODO RPMSPEC.toml` and know exactly what needs
manual attention**, rather than having to diff every field against the
original source by hand.

### 4.3 TOML writer with comments (`toml_edit`, not `toml`)

v1's `toml`/`serde` combination is fine for *reading* — but plain
`serde`-based serialization has no concept of comments, so it cannot
produce the `# TODO:` annotations from §4.2. Use the `toml_edit` crate for
import output specifically: build a `toml_edit::Document` field by field
(or serialize via `serde` first, then walk the resulting `Document` to
inject comments at the right keys), attaching each `ImportNote` as a
comment directly above its field.

`makerpm build`/`lint`/`spec` continue to use `serde`+`toml` for *reading*
`RPMSPEC.toml` as before — this only affects the write path used by
`import` (and, if useful, `init`, which could similarly comment its
scaffolded template rather than leaving fields silently unexplained).

### 4.4 Common CLI behavior for `import`

- Both subcommands print a summary at the end: counts of `Confident` /
  `BestEffort` / `Unsupported` fields, and a reminder to run `makerpm lint
  <output>` next.
- Both refuse to overwrite an existing file at `-o` without `--force`.
- Both exit non-zero only on unrecoverable parse failure (e.g. `PKGBUILD`
  missing entirely, `debian/control` absent) — a successful-but-heavily-
  annotated import is still exit code 0, since "produced a draft that
  needs review" is the expected happy path, not a failure.

---

## 5. `makerpm import deb` design

### 5.1 Expected input shape

A directory that already looks like an unpacked Debian source package:

```
./path/to/debian/source/
├── debian/
│   ├── control
│   ├── changelog
│   ├── rules
│   ├── copyright
│   ├── compat  (or debhelper-compat in control)
│   ├── patches/
│   │   └── series
│   └── *.install, *.dirs, etc. (optional)
└── <upstream source files>
```

If `debian/control` or `debian/changelog` is missing, fail immediately
with a clear "this doesn't look like an extracted Debian source package"
error rather than attempting a partial import.

### 5.2 `debian/control` → package + subpackage identity

`debian/control` is RFC822-style stanzas separated by blank lines, with
continuation lines (leading whitespace) belonging to the previous field.
Check crates.io for an existing, well-maintained parser for this format
(search terms: `deb822`, `debian-control`) before hand-rolling one; if
nothing suitable is found or it pulls in more than needed, a minimal
stanza parser for this specific format is small (a few dozen lines) and
worth writing directly rather than adopting a heavy dependency.

| `debian/control` field | Maps to |
|---|---|
| `Source:` stanza's `Build-Depends:` | `package.deps.build_depends` (each entry passed through §5.4's name-translation heuristic) |
| `Source:` stanza's `Homepage:` | `package.url` |
| `Source:` stanza's `Maintainer:` | used for `changelog[].packager` if `debian/changelog` doesn't have a clearer per-entry maintainer |
| Each `Package:` stanza | one `Subpackage` (or the base `Package` if its name matches the source package name) |
| `Description:` (short + long form) | stanza's `summary` (first line) + `description` (remaining lines) |
| `Depends:` per binary stanza | that subpackage's `deps.depends` |
| `Architecture: all` | that subpackage's `noarch = true` |

The **base package vs. subpackage** distinction: Debian doesn't have a
"main package matches source name" convention as strictly as Fedora does —
pick the `Package:` stanza whose name matches the `Source:` field (or, if
none matches, the first stanza) as `package`, and every other stanza
becomes a `[[subpackage]]` with `suffix` derived by stripping the source
name prefix (e.g. source `libfoo`, binary `libfoo-dev` → `suffix = "dev"`,
noting Fedora convention would call this `-devel` — flag the suffix as
`BestEffort` and suggest the Fedora-conventional rename in the TODO note).

### 5.3 `debian/rules` → best-effort build system detection

Do not parse `debian/rules` as a Makefile in any structural sense (it's
often just `#!/usr/bin/make -f` + `%:\n\tdh $@` with debhelper doing
everything implicitly, or, in the worst case, deeply custom). Instead:

1. Look for marker files in the source tree root: `CMakeLists.txt` →
   `BuildSystem::Cmake`; `meson.build` → `Meson`; `configure.ac` or
   `configure` → `Autotools`; `Cargo.toml` → `Cargo`; `pyproject.toml` →
   `PythonPyproject`; a bare `Makefile` with none of the above → `Make`;
   nothing recognized → `None`.
2. Set this as `BestEffort` confidence unconditionally — even an
   unambiguous `CMakeLists.txt` match doesn't guarantee the Debian
   packaging didn't do something unusual in `debian/rules` (e.g.
   `override_dh_auto_configure` with extra flags) that a marker-file
   heuristic can't see.
3. If `debian/rules` contains any `override_dh_*` targets, emit an
   `Unsupported`-confidence note on `build.steps.*` fields listing which
   overrides were detected by name (just the target names, e.g.
   `override_dh_auto_install`) without attempting to translate their
   contents, so the reviewer knows exactly where to go look at the
   original `debian/rules` by hand.

### 5.4 Dependency name translation (best-effort, always annotated)

Debian and Fedora package names frequently differ for the same underlying
library. Implement a small, explicitly-limited heuristic table plus
pattern rules — never claim confidence beyond what it deserves:

| Debian pattern | Heuristic Fedora guess | Confidence |
|---|---|---|
| `libfoo-dev` | `libfoo-devel` (or `foo-devel` if `libfoo` looks like it's just `foo`'s soname-prefixed form) | `BestEffort` |
| `libfoo1`, `libfoo2` (soname-versioned) | `libfoo` (strip trailing version digits) | `BestEffort` |
| `python3-foo` | `python3-foo` (usually identical) | `BestEffort` |
| version constraints `foo (>= 1.2)` | `foo >= 1.2` (RPM `Requires` syntax) | `Confident` (syntax translation only, not name translation) |
| anything not matching a pattern | passed through unchanged | `Unsupported` — flag explicitly as "package name not translated, verify on Fedora" |

Every single dependency line gets an `ImportNote`, even the ones marked
`BestEffort` — this is a case where over-annotating is correct, since
cross-distro package names are exactly the kind of thing that's silently
wrong in a way that only fails at `dnf builddep`/`rpmbuild` time, often
confusingly.

### 5.5 `debian/changelog` → `package.version`/`release`/`changelog`

Format is regular and well-documented: repeated blocks of
`pkgname (version) distribution; urgency=...` header, `  * entry` lines,
and a `-- Maintainer <email>  date` trailer, blank-line separated. Write a
small dedicated parser (again, check for an existing crate first, but this
format is simple and stable enough that hand-rolling is reasonable if
nothing lightweight fits).

- Most recent entry's `version` → split on Debian's `epoch:upstream-revision`
  grammar: `Epoch:` (if a colon-prefixed epoch is present) → `package.epoch`,
  the upstream portion → `package.version` (sanitized — RPM disallows `-`
  in `Version`, so a Debian revision-embedded `-` needs collapsing;
  `~` in Debian versions has no changed meaning in modern RPM version
  comparison but is unusual — flag with a note rather than assuming it's
  fine, since this is a place a wrong guess directly breaks upgrade
  ordering).
- All entries (not just the latest) → `package.changelog[]`, each mapped
  to `version`/`date`/`packager`/`entries` directly — this is a case where
  Debian's format maps almost 1:1 onto v1's `ChangelogEntry` shape, so
  confidence here should be `Confident` for well-formed entries.

### 5.6 `debian/copyright` → `package.license`

Modern `debian/copyright` files (DEP-5 machine-readable format) have a
`License:` field per-stanza, sometimes multiple distinct licenses for
different files. For v2:

- Extract the **first** `License:` value found (typically the
  whole-package license in the first stanza) as `package.license`,
  `BestEffort` confidence.
- Map common Debian license short names to SPDX via a small static table
  (`GPL-2+` → `GPL-2.0-or-later`, `GPL-3+` → `GPL-3.0-or-later`,
  `LGPL-2.1+` → `LGPL-2.1-or-later`, `Expat`/`MIT` → `MIT`,
  `BSD-3-clause` → `BSD-3-Clause`, `Apache-2.0` → `Apache-2.0`) — anything
  unrecognized is passed through verbatim with an `Unsupported` note
  ("license string not recognized, verify SPDX expression manually").
- Do not attempt to reconstruct a full per-file SPDX expression from a
  multi-stanza `debian/copyright` — that's a materially harder problem
  than v2's scope; a single best-guess top-level license plus a loud
  `Unsupported` note is the right level of effort here.

### 5.7 File lists — explicitly not attempted

Debian's `debian/*.install`/`.dirs` files describe *build output* paths in
Debian's filesystem layout conventions, which frequently don't match
Fedora's (e.g. differing `/usr/lib` vs `/usr/lib64` conventions, Python
site-packages paths, docdir naming). Rather than emit confidently-wrong
`files.paths` entries:

- Emit each subpackage's `files` section with a single placeholder entry
  and an `Unsupported`-confidence note: `# TODO: file list not imported —
  Debian and Fedora filesystem layouts differ; populate manually after a
  test build`.
- This is a deliberate scope cut, not an oversight — flag it as such in
  the CLI's final summary output too, so it's not missed.

---

## 6. `makerpm import arch` design

### 6.1 Restricted `PKGBUILD` parser

Grammar recognized (everything else is either ignored or captured as an
opaque, unexecuted text blob):

- Scalar assignment: `name=value` or `name="value"` or `name='value'`.
- Array assignment: `name=(a b "c d" 'e f')`, including multi-line arrays
  (parenthesized, so track paren depth across lines).
- Named function bodies: `build()`, `package()`, `prepare()`, `check()`,
  `pkgver()` — captured as raw opaque text between the matching `{`/`}` of
  each (track brace depth; do not attempt to parse their *contents* as
  shell beyond finding the boundaries).
- Anything else (comments, other functions, top-level command
  invocations, conditionals) is not parsed and does not block the import,
  but top-level statements that aren't recognized assignments are noted as
  `Unsupported` at a whole-file level ("PKGBUILD contains additional logic
  outside recognized fields; review the original file for anything not
  reflected below").
- **No command substitution is evaluated.** If a scalar or array value
  contains `$(...)` or `` `...` ``, do not attempt to resolve it — treat
  the field's value as `Unsupported`, keep the literal unresolved text as
  a placeholder, and note that it depends on a shell computation that
  wasn't evaluated.

### 6.2 Field mapping

| PKGBUILD field | Maps to | Confidence |
|---|---|---|
| `pkgname` | `package.name` | `Confident` (unless it's an array — split-package PKGBUILDs are a v2 gap; import the first name, note the rest as unsupported) |
| `pkgver` | `package.version` | `Confident`, *unless* a `pkgver()` function is also present in the file — in that case `Unsupported`, keep the static `pkgver` value as a placeholder, and note "PKGBUILD defines a dynamic pkgver(); this value may be stale, verify manually" |
| `pkgrel` | contributes to `package.release` (rendered as `<pkgrel>%{?dist}`) | `Confident` |
| `pkgdesc` | `package.summary` (and reused for `package.description` if no better long-form text exists) | `Confident` |
| `arch` | `package.noarch = true` if the only entry is `any`, else `false` | `Confident` |
| `url` | `package.url` | `Confident` |
| `license` | `package.license`, via the same small static mapping approach as §5.6 (`GPL2` → `GPL-2.0-or-later`, `GPL3` → `GPL-3.0-or-later`, `MIT` → `MIT`, `Apache` → `Apache-2.0`, `BSD` → `BSD-3-Clause`, `ZLIB` → `Zlib`, `custom:*` → passed through as `Unsupported`) | `BestEffort` |
| `depends` | `package.deps.depends`, each entry's version-constraint operator translated to RPM's spaced form (same rule as §5.4's table row) | `Confident` for syntax, name is passed through unchanged (Arch and Fedora package names for common libraries are close enough more often than Debian/Fedora, but still not guaranteed — annotate as `BestEffort`) |
| `makedepends` | `package.deps.build_depends` | `BestEffort`, same reasoning |
| `optdepends` | `package.deps.recommends` — each entry has the form `"pkgname: reason"`, split on the first `: ` — the reason text is dropped (RPM `Recommends` has no per-entry description field) but preserved in the field's `ImportNote` so it isn't silently lost | `BestEffort` |
| `provides` / `conflicts` / `replaces` | `package.deps.{provides,conflicts,obsoletes}` | `BestEffort` |
| `source` / `sha256sums` (or `b2sums`/`md5sums` if `sha256sums` absent) | `package.sources` / `package.sha256sums` | `Confident` — v1's source grammar (§4.4 of plan v1) was deliberately modeled on makepkg's, so this is close to a direct copy. If only `md5sums`/`b2sums` are present (no `sha256sums`), copy the array as-is into a *comment* rather than `sha256sums` (wrong hash algorithm in the real field would silently break verification), and note that re-hashing is needed — do not put an MD5 sum into a field named `sha256sums`. |
| `depends_x86_64`, `makedepends_x86_64` (if present) | merged into `depends`/`build_depends` in addition to the generic arrays | `BestEffort`; other `_<arch>` suffixed arrays are ignored per §1's stated limitation |
| `build()` function body | `package.build.steps.build`, with `$pkgdir` occurrences replaced by `%{buildroot}` | `BestEffort` — flag remaining `$srcdir` references (usually unneeded under `rpmbuild`, since `%prep` already lands in the right directory) as `Unsupported` if any survive the substitution |
| `package()` function body | `package.build.steps.install`, same `$pkgdir` → `%{buildroot}` substitution | `BestEffort` |
| `prepare()` function body | `package.build.steps.prep` | `BestEffort` |
| `check()` function body | `package.build.steps.check` | `BestEffort` |
| (no PKGBUILD equivalent) | `package.build.system` | Always set to `"none"` — PKGBUILD's `build()`/`package()` are raw shell by construction, so there's no macro-driven build system to detect the way §5.3 does for Debian; the imported shell is the entire `%build`/`%install` content |

### 6.3 What's structurally impossible to import well

Be explicit in both the tool's output and this plan about a few things
that are Arch-specific enough that no amount of heuristics closes the gap
well — these should always be `Unsupported`-flagged, never guessed at:

- Split-package PKGBUILDs (`pkgname=(a b c)` with per-package
  `package_a()`/`package_b()` functions) — v2 imports only the first name
  as `package.name` and notes the rest need to become subpackages manually.
- `%files` equivalents — PKGBUILD's `package()` function moves files via
  arbitrary `install`/`cp` commands rather than declaring a list, so unlike
  even Debian's best-effort `.install` files, there's no structured list to
  read from at all. `package.files` should be left with the same
  placeholder-and-TODO treatment as §5.7, always.
- `.install` scriptlet files (`pkgname.install`, referenced via
  `install=pkgname.install` in the PKGBUILD) — if referenced, note its
  filename and that `scriptlets.*` need manual population; don't attempt
  to parse the referenced file's shell functions automatically in v2.

---

## 7. Crate additions for v2

| Concern | Crate | Notes |
|---|---|---|
| Commented TOML writing | `toml_edit` | Used only by `import` (and optionally `init`); v1's read path keeps using `toml`+`serde` |
| Debian control/changelog parsing | check crates.io first (`deb822`/`debian-control`-family search); fall back to a small hand-rolled parser if nothing fits without pulling in unwanted transitive dependencies | Both formats are simple enough that hand-rolling is a reasonable fallback, not a last resort to be avoided at all costs |

No new crate is needed for `PKGBUILD` parsing — it's a small enough
restricted grammar (§6.1) to hand-roll directly, and pulling in a real
shell parser/interpreter would work against the "never execute untrusted
input" constraint from §4.1 anyway.

---

## 8. Module layout additions

```
src/
├── ... (v1 modules unchanged)
├── lint.rs                     # renamed/extended from v1's validate.rs;
│                                # Severity, LintFinding, all rules from
│                                # v1 §5 plus v2 §3's new warning rules
├── import/
│   ├── mod.rs                  # ImportDraft, ImportNote, Confidence (§4.2);
│   │                            # shared toml_edit writer (§4.3)
│   ├── deb/
│   │   ├── mod.rs
│   │   ├── control.rs          # debian/control stanza parser
│   │   ├── changelog.rs        # debian/changelog parser
│   │   ├── build_detect.rs     # §5.3 marker-file heuristic
│   │   └── deps.rs             # §5.4 name-translation heuristics
│   └── arch/
│       ├── mod.rs
│       ├── pkgbuild_parser.rs  # §6.1 restricted grammar parser
│       └── deps.rs             # PKGBUILD-side version-constraint translation
```

`lint.rs` replacing `validate.rs` is a rename plus additive change — v1's
existing validation tests should be preserved as `Severity::Error` cases,
not rewritten.

---

## 9. Testing strategy additions

- **`pkgbuild_parser.rs`**: unit tests against real, varied PKGBUILDs —
  include at least one with a `pkgver()` function, one with `depends_x86_64`,
  one with command substitution in a field (to confirm it's correctly
  flagged `Unsupported` rather than mis-parsed), and one minimal/well-formed
  one that should come through almost entirely `Confident`.
- **`deb/control.rs` and `deb/changelog.rs`**: unit tests against real
  `debian/control`/`debian/changelog` samples, including multi-binary-stanza
  control files and multi-entry changelogs, plus malformed/truncated input
  to confirm parse failures are clear errors rather than panics.
- **Import integration tests**: for each importer, one full fixture
  (a real small Debian source package directory; a real small PKGBUILD)
  run end-to-end through `import` → assert the output TOML parses cleanly
  via v1's `parse.rs`, and assert specific fields land with the expected
  `Confidence` level (spot-check a handful, not every field).
- **`lint.rs`**: extend v1's per-rule fixture pattern (§5 of plan v1) with
  one fixture per new `Warning`-severity rule from §3, plus a test
  confirming `--strict` changes the exit code appropriately.
- **Round-trip sanity test**: `makerpm import arch` on a fixture PKGBUILD →
  `makerpm lint` on the result → confirm it exits 0 (no `Error`-severity
  findings) even though `Warning`-severity TODO-related findings are
  expected and fine. This is the actual end-to-end promise of the
  `import` → `lint` → manual edit → `build` workflow described in §1.

---

## 10. Phased execution plan — prompts and exit checklists

### Phase 5 — CLI restructure and `lint`

**Covers:** §2 (positional-argument CLI), §3 (`lint.rs`, severity levels,
new warning rules). No importer code yet.

**Prompt:**
> Implement Phase 5 of the makerpm v2 plan: restructure the CLI (§2) so
> `build`, `lint`, `fetch`, and `spec` take a positional `<PATH>` argument
> defaulting to `./RPMSPEC.toml`, and rename `validate` to `lint` while
> keeping `validate` as a hidden `clap` alias. Rework the old `validate.rs`
> into `lint.rs`: introduce `Severity`/`LintFinding` types (§3), migrate
> every v1 validation rule to `Severity::Error` unchanged in behavior, and
> add the new `Severity::Warning` rules listed in §3. Implement `--strict`
> on `makerpm lint` to make warnings exit non-zero too. Do not touch
> `import` in this phase — it doesn't exist yet.

**Exit checklist:**
- [ ] `makerpm build RPMSPEC.toml`, `makerpm lint RPMSPEC.toml`, etc. all
      work with the positional argument; `makerpm validate RPMSPEC.toml`
      still works as a silent alias.
- [ ] Every v1 validation-rule test still passes, now asserting
      `Severity::Error` findings instead of the old pass/fail boolean.
- [ ] Each new `Severity::Warning` rule from §3 has a fixture and test.
- [ ] `--strict` changes exit code behavior in a dedicated test.
- [ ] `lint` output is grouped/readable (errors before warnings, or
      clearly labeled), not an unsorted dump.

### Phase 6 — Shared import infrastructure

**Covers:** §4 in full (`import/mod.rs`: `ImportDraft`, `ImportNote`,
`Confidence`, the `toml_edit`-based commented writer). No format-specific
parsing yet — build and test this against a hand-constructed `ImportDraft`
value, not real Debian/PKGBUILD input.

**Prompt:**
> Implement Phase 6 of the makerpm v2 plan: the shared import
> infrastructure in `import/mod.rs` (§4). Define `ImportDraft`,
> `ImportNote`, and `Confidence` per §4.2, reusing v1's `PkgSpecFile`/
> `Package`/`Subpackage` structs unchanged — do not create parallel draft
> structs. Implement the `toml_edit`-based writer (§4.3) that serializes an
> `ImportDraft` to a `.toml` file, inserting a `# TODO:` comment above any
> field with a `BestEffort` or `Unsupported` `ImportNote`, and no comment
> for `Confident` fields. Implement the common CLI behaviors from §4.4
> (overwrite protection via `--force`, the Confident/BestEffort/Unsupported
> summary printed at the end). Test this entirely against hand-built
> `ImportDraft` values in unit tests — no real Debian or PKGBUILD parsing
> exists yet, so construct test fixtures directly in Rust.

**Exit checklist:**
- [ ] A hand-built `ImportDraft` with a mix of `Confident`/`BestEffort`/
      `Unsupported` fields round-trips through the writer to a `.toml` file
      that (a) parses cleanly via v1's `parse.rs`, and (b) visibly contains
      `# TODO:` comments only above the non-`Confident` fields.
  - [ ] Overwrite protection: writing to an existing path without
      `--force` errors; with `--force` it overwrites.
- [ ] The end-of-run summary counts match the actual `ImportNote` tally in
      a test.

### Phase 7 — `import deb`

**Covers:** §5 in full, `import/deb/*`. Depends on Phase 6.

**Prompt:**
> Implement Phase 7 of the makerpm v2 plan: `makerpm import deb <dir> -o
> <path>` (§5), building on Phase 6's shared infrastructure. Implement
> `control.rs` (RFC822 stanza parsing for `debian/control` — check
> crates.io for an existing well-maintained parser first per §7's crate
> table; hand-roll a minimal one if nothing fits well), `changelog.rs`
> (§5.5's parser for `debian/changelog`), `build_detect.rs` (§5.3's
> marker-file heuristic — explicitly do not parse or execute
> `debian/rules` as a Makefile), and `deps.rs` (§5.4's name-translation
> heuristic table). Follow §5.7 exactly for file lists: never emit
> `files.paths` entries from Debian `.install`/`.dirs` data, always emit
> the placeholder-plus-TODO instead. Every heuristic-derived field must
> carry the confidence level specified in §5's tables — do not mark
> anything `Confident` unless it is a direct, unambiguous field copy.

**Exit checklist:**
- [ ] A missing `debian/control` or `debian/changelog` produces a clear
      "not a Debian source package" error, not a panic or silent partial
      import.
- [ ] `control.rs` correctly handles multi-line continuation fields
      (wrapped `Build-Depends:`, multi-paragraph `Description:`) — test
      against a real multi-binary-package `control` file.
- [ ] `changelog.rs` correctly extracts epoch/version/revision and the
      full changelog history, not just the latest entry.
- [ ] Build-system detection never inspects or runs `debian/rules` beyond
      checking for `override_dh_*` target *names* as plain text.
- [ ] Every dependency line gets an `ImportNote`, per §5.4's closing
      paragraph — verify with a test asserting note count matches
      dependency count.
- [ ] `files` sections always contain the placeholder-and-TODO from §5.7,
      never a guessed real path.
- [ ] Running the full importer against a real (small, permissively
      licensed) Debian source package directory produces a `.toml` that
      passes v1's `parse.rs` cleanly and whose `Confident`-vs-not field
      counts look reasonable on manual inspection.

### Phase 8 — `import arch`

**Covers:** §6 in full, `import/arch/*`. Depends on Phase 6; independent of
Phase 7 (can run in parallel with it if desired).

**Prompt:**
> Implement Phase 8 of the makerpm v2 plan: `makerpm import arch
> <PKGBUILD> -o <path>` (§6), building on Phase 6's shared infrastructure.
> Implement `pkgbuild_parser.rs` as a restricted, non-executing grammar
> parser per §6.1 — it must recognize scalar/array variable assignments
> and capture named function bodies (`build`, `package`, `prepare`,
> `check`, `pkgver`) as opaque unparsed text, and must never invoke `bash`
> or evaluate command substitution (`$(...)`/backticks); any field
> containing unevaluated command substitution must be flagged
> `Unsupported`, never guessed at. Implement the field mapping table from
> §6.2 exactly, including the `$pkgdir` → `%{buildroot}` substitution in
> extracted function bodies and the sha256sums-only rule (never write a
> non-SHA256 checksum into the `sha256sums` field). Explicitly handle the
> §6.3 cases (split-package PKGBUILDs, absent file lists, `.install`
   scriptlet references) with the specified placeholder-and-TODO treatment,
> not a best-effort guess.

**Exit checklist:**
- [ ] Parser tests cover: a well-formed single-package PKGBUILD (mostly
      `Confident` output), one with a `pkgver()` function (version marked
      `Unsupported`), one with `depends_x86_64` (merged in, `BestEffort`),
      and one with command substitution in a field (flagged, not evaluated).
- [ ] No code path in `pkgbuild_parser.rs` shells out to `bash` or any
      command interpreter — confirm by inspection, this is a hard
      requirement from §4.1/§6.1, not a nice-to-have.
- [ ] `optdepends`'s `"pkgname: reason text"` split is correct and the
      dropped reason text ends up in the field's `ImportNote`, not
      silently discarded.
- [ ] If only `md5sums`/`b2sums` are present (no `sha256sums`), the output
      never writes those values into the `sha256sums` field — verify with
      a dedicated test.
- [ ] Split-package PKGBUILDs (`pkgname=(...)`) produce a clear
      `Unsupported` note about the untranslated additional package names,
      not a silent single-package import that drops them.
- [ ] Running the full importer against a real (small, permissively
      licensed) Arch PKGBUILD produces a `.toml` that passes v1's
      `parse.rs` cleanly.
- [ ] The round-trip sanity test from §9 (`import arch` → `lint` → exit 0
      despite warnings) passes for at least one real fixture.

Once Phase 8's checklist is green, v2's four-subcommand surface (§1) is
complete and ready for the v1-style final QA pass (§11) before release.

---

## 11. Final QA pass addendum — prompt for an independent reviewing LLM

Reuse v1's §15 QA prompt (security / idiomatic Rust / conformance) in
full for the v2 codebase, with the following v2-specific additions to its
security section:

> In addition to the checks already listed for makerpm's security review,
> specifically verify for the v2 `import` subcommands:
>
> - **No execution of input files.** Grep the entire `import/` module tree
>   for any invocation of a shell/process (`Command::new`, `std::process`,
>   any crate that shells out) — there should be none. `import deb` must
>   never execute or `source` `debian/rules`; `import arch` must never
>   execute or `source` the `PKGBUILD`. This is the single most important
>   security property of this release; treat any violation as critical
>   severity regardless of how contained or "just for detection" it might
>   look.
> - **Path traversal in extracted-directory parsing.** `import deb` reads
>   from an arbitrary user-supplied directory tree; confirm no path
>   constructed from its contents (e.g. a crafted `debian/patches/series`
>   entry) can cause a read or write outside the intended input/output
>   locations.
> - **Resource exhaustion on malformed input.** Both parsers process
>   plain-text input with hand-rolled grammars (§5.2/§6.1) — confirm
>   there's no unbounded recursion or quadratic-blowup path on adversarial
>   input (e.g. deeply nested parentheses in a `PKGBUILD` array, an
>   absurdly long `debian/changelog`).
> - **TOML output injection.** Confirm the `toml_edit`-based writer (§4.3)
>   correctly escapes any TOML-special characters in values pulled from
>   the input files (Debian package descriptions, PKGBUILD string values)
>   — a crafted upstream field should not be able to break out of its
>   TOML string context and inject additional keys/tables into the
>   generated file.
