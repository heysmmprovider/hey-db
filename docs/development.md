# Development guide

[Back to the README](../README.md)

## Run locally

Install Node.js 22.12+ (or a supported newer release), a current stable Rust toolchain, and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your operating system. On macOS, install Xcode Command Line Tools.

```sh
npm ci
npm run desktop
```

`npm run dev` opens the frontend on `http://127.0.0.1:1420`. Browser mode is a demo preview; real database connections and credential storage require the desktop application.

## Build

```sh
# macOS .app and .dmg
APPLE_SIGNING_IDENTITY=- CI=true npm run tauri -- build --bundles app,dmg

# Windows installer, on Windows
npm run tauri -- build --bundles nsis

# Linux package, on Linux
npm run tauri -- build --bundles deb
```

Outputs are under `src-tauri/target/release/bundle/`. The included installer is for Apple Silicon on macOS 13 or later. Intel Macs require an x86_64 build or a universal build. Windows and Linux use the same codebase but have not yet received manual platform QA.

Local development builds do not require Apple signing credentials. Public macOS distribution should use a Developer ID certificate and notarization. Signing credentials belong in the release environment's secret store, never in source control.

## Release packaging

The ready-to-install preview lives in [`downloads/`](../downloads/README.md), alongside its SHA-256 checksum. The stable installer filename is `hey-db-macos-arm64.dmg`. Use that same file when publishing a download on [heymydb.com](https://heymydb.com).

On an Apple Silicon Mac with native Node.js 22.12+, Rust installed through rustup, and Xcode Command Line Tools, run:

```sh
npm run release -- 0.1.2
```

Use the version you want to release. `npm run release` without a version increments the patch number automatically; specifying the current version rebuilds it. Older versions and prerelease suffixes are rejected. Run `npm run release -- --help` for usage.

The script installs dependencies with `npm ci`, synchronizes the versions in npm, Tauri, Cargo, and both lockfiles, and runs the frontend, Rust, and temporary PostgreSQL tests plus formatting and Clippy checks. The UI reads its displayed version from `package.json`. It builds the Apple Silicon `.app` and `.dmg`, verifies the app version, architecture, signature, and disk image, then replaces these files:

- `downloads/hey-db-macos-arm64.dmg`
- `downloads/SHA256SUMS`

Build outputs are also available under `src-tauri/target/aarch64-apple-darwin/release/bundle/`. A failed build restores the previous version metadata and leaves the published download unchanged. Historical installers are preserved. If the process was forcibly killed, confirm it has stopped before removing `.local/release.lock` and retrying.

**The script does not stage, commit, tag, push, publish a GitHub release, or change the website.** Review and commit your source changes, version metadata, installer, and checksum together, then push to `main` yourself. The website's existing [download URL](https://github.com/heysmmprovider/hey-db/blob/main/downloads/hey-db-macos-arm64.dmg?raw=true) always targets that fixed path on `main`; pushing the replacement updates what it serves without changing the website. Merely building locally does not update the remote download.

To verify the prepared download yourself:

```sh
cd downloads
shasum -a 256 -c SHA256SUMS
```

The current preview is ad-hoc signed, not Apple-notarized. For a verified developer identity and standard macOS distribution, use a Developer ID certificate and Apple notarization instead of the ad-hoc signing override above. Keep signing credentials outside source control.

## Validate

```sh
npm test
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npm run test:database
```

The last command starts a temporary PostgreSQL server bound to localhost on a free port. Its credentials are generated for that run, its data is synthetic, and its directory is removed afterward. It tests real updates, composite keys, exact large integers, NULL, SQL-looking values, stale-write conflicts, rollback, read-only results, cancellation, and result limits. It never connects to your own database. PostgreSQL test binaries are development dependencies and are not included in the desktop app.
