# Install hey db

Download from **[heymydb.com](https://heymydb.com)** or get the ready-to-install file here:

**[hey db — macOS, Apple Silicon (.dmg)](hey-db-macos-arm64.dmg?raw=true)**

Requires an M-series Mac running macOS 13 Ventura or later. No Node.js, Rust, PostgreSQL server installation, or source build is needed on your Mac to run the app and connect to an existing database.

1. Open the disk image.
2. Drag **hey db** into **Applications**.
3. Launch hey db and add a connection, or try the demo.

This preview is ad-hoc signed and is **not Apple-notarized**. If macOS blocks the first launch and you trust the download, follow [Apple’s instructions for opening an unnotarized app](https://support.apple.com/en-us/102445).

## Verify the download

Download [SHA256SUMS](SHA256SUMS?raw=true) into the same folder as the installer, then run this command from that folder:

```sh
shasum -a 256 -c SHA256SUMS
```

The result should be `hey-db-macos-arm64.dmg: OK`. This checks the file against the published checksum; it does not replace Apple notarization.

The disk image contains the app, including the synthetic demo. It does not include saved connections, credentials, or a database server. Intel Mac, Windows, and Linux installers are not included in this release.

[Request a feature](https://github.com/heysmmprovider/hey-db/issues/new?template=feature_request.yml) · [Project README](../README.md) · [Build from source](../docs/development.md)
