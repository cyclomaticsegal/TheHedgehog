# GitHub Actions Release Plan — The Hedgehog

**Status:** **IMPLEMENTED** — see [`.github/workflows/release.yml`](../.github/workflows/release.yml) and [`dist-workspace.toml`](../dist-workspace.toml).
**First release:** [`v0.1.0-preview.1`](https://github.com/cyclomaticsegal/TheHedgehog/releases/tag/v0.1.0-preview.1), cut 2026-04-12. Four target archives, all green. (The v0.1.0-preview tag was published first and then deleted after smoke testing surfaced an env_path bug; v0.1.0-preview.1 is the working release.)
**Closes:** issue [#1](https://github.com/cyclomaticsegal/TheHedgehog/issues/1).

## What we actually shipped

We went with `cargo-dist` rather than the hand-rolled YAML matrix originally sketched in this document. The reasoning and tradeoff are documented on issue #1; the short version is that for a non-Rust-native maintainer, `cargo dist init` abstracts away the target triples, glibc gotchas, and cross-compilation plumbing that would otherwise be hand-written.

The implemented pipeline matches the resolved decisions on #1 exactly:

| Decision | Outcome |
|---|---|
| Apple signing | Unsigned. macOS first-run requires right-click → Open. Documented in `INSTALL.txt`. |
| Targets | macOS aarch64, macOS x86_64, Linux x86_64 glibc, Windows x86_64 MSVC. Four archives. Linux aarch64 and musl deliberately excluded. |
| Trigger | Tag-triggered on `v*`. Cutting a release is a deliberate `git tag` + `git push`, never automatic. |
| Hosting | GitHub Releases only. |
| Tooling | `cargo-dist` v0.31.0. |

The original proposal below is preserved for historical context.

---

**(Original proposal — superseded by the implementation above)**

**Context:** We want a distributable executable for Windows, macOS, and Linux so users can run The Hedgehog without installing Rust and building from source.

## Short answer

**No, we don't currently have a distributable executable — and what we do have is macOS-only.**

Our earlier `cargo build --release` produced `target/release/the-hedgehog`, which is a ~30-50 MB self-contained binary *for this Mac*. It won't run on Windows or Linux, and depending on which Mac it was built on, it may not even run on both Intel and Apple Silicon. Rust gives you one target triple per build.

## Why cross-platform distribution isn't a single-button thing

1. **Rust binaries are per-platform.** `x86_64-apple-darwin` ≠ `aarch64-apple-darwin` ≠ `x86_64-pc-windows-msvc` ≠ `x86_64-unknown-linux-gnu`. To cover the obvious targets we need **four separate builds**.

2. **macOS Gatekeeper** will block an unsigned binary on anyone else's machine — they have to right-click → Open and dismiss a warning, or we need an Apple Developer account ($99/year) to code-sign and notarize the app.

3. **Windows SmartScreen** shows an "unrecognized publisher" warning on unsigned `.exe`s. Dismissable but scary-looking for end users.

4. **Linux glibc drift.** A binary built on Ubuntu 24 won't run on Ubuntu 20 because of glibc version skew. The fix is building in an older Docker base, or statically linking against musl (`x86_64-unknown-linux-musl`).

5. **Runtime configuration.** Users still need a `.env` file for API keys — the app won't run without at least FRED + Alpha Vantage configured. Any release bundle needs a `.env.example` and clear first-run instructions.

## The standard path

A GitHub Actions workflow with a matrix `[macos-latest, ubuntu-latest, windows-latest]` that runs `cargo build --release` on each runner, bundles the binary plus a sample `.env.example`, and attaches them to a GitHub Release. This is about 40 lines of YAML and requires zero extra build system.

Tools like `cargo-dist` automate it further — auto-generated install scripts, universal macOS binaries via `lipo`, Homebrew tap, etc. — but a hand-rolled matrix is fine for a preview release.

### Minimal matrix (sketch)

```yaml
name: release
on:
  push:
    tags: ['v*']
jobs:
  build:
    strategy:
      matrix:
        include:
          - os: macos-latest
            target: x86_64-apple-darwin
            archive: the-hedgehog-macos-x86_64.tar.gz
          - os: macos-latest
            target: aarch64-apple-darwin
            archive: the-hedgehog-macos-aarch64.tar.gz
          - os: ubuntu-latest
            target: x86_64-unknown-linux-gnu
            archive: the-hedgehog-linux-x86_64.tar.gz
          - os: windows-latest
            target: x86_64-pc-windows-msvc
            archive: the-hedgehog-windows-x86_64.zip
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}
      - run: cargo build --release --target ${{ matrix.target }}
      # Bundle binary + .env.example + README into the archive
      # Upload as release asset
```

### What a user downloads

Each release artifact is a `.tar.gz` (or `.zip` on Windows) containing:

```
the-hedgehog/
├── the-hedgehog(.exe)     # the binary
├── .env.example           # template for API keys
└── README.txt             # first-run instructions
```

User flow:
1. Download the archive for their OS from the GitHub Releases page
2. Extract
3. Copy `.env.example` to `.env` and fill in their FRED / Alpha Vantage keys
4. Double-click the binary (or run from terminal)

## Decisions needed before I implement

1. **Apple signing** — do we have (or will we get) an Apple Developer account for code-signing and notarization? If no, macOS users will see a "cannot be opened because developer cannot be verified" dialog and need to right-click → Open the first time. Acceptable for Preview 0.1, painful for general distribution.

2. **Target list** — is `[macOS x86_64, macOS aarch64, Linux x86_64, Windows x86_64]` the right shape? Should we also cover:
   - **Linux aarch64** (Raspberry Pi, ARM servers)?
   - **Linux musl** for maximum portability (statically linked, no glibc dependency)?
   - **Universal macOS binary** (one binary that runs on both Intel and Apple Silicon via `lipo`)?

3. **Trigger** — do we build on:
   - Every push to `main` (continuous release candidates)?
   - Only on version tags like `v0.1.0-preview` (manual, curated releases)?
   - Manual workflow dispatch (push-button release when we're ready)?

4. **Release hosting** — GitHub Releases is the default. Alternatives: dedicated S3/R2 bucket, Homebrew tap (macOS), Winget manifest (Windows), AUR PKGBUILD (Arch Linux). None of these need to block Preview 0.1 — start with GitHub Releases and add distribution channels later.

5. **Use `cargo-dist` or hand-roll?**
   - **Hand-rolled** (~40 lines of YAML): full control, easy to read, small maintenance surface.
   - **cargo-dist**: generates the workflow for us, handles platform packaging, auto-generates install shell scripts (`curl … | sh`), supports updates. More featureful but another dependency to keep current.

My recommendation for Preview 0.1: hand-rolled workflow, unsigned builds, tag-triggered releases, target `[macOS universal, Linux x86_64 glibc, Windows x86_64]`. Ship it, see what breaks, add signing and extra targets later.
