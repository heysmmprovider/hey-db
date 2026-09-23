# hey db

A focused, local-first PostgreSQL desktop workspace. Built with Tauri 2, Rust, React, TypeScript, and CodeMirror.

![hey db workspace with synthetic sample data](docs/workspace.png)

## What works in 0.1

- Saved PostgreSQL connections, verified TLS, optional system credential storage, and read-only connections.
- Schema explorer with table/view discovery, columns, primary keys, and index definitions.
- SQL tabs, PostgreSQL highlighting, basic schema completion, selected-statement execution, and cancellation.
- A result grid that virtualizes both rows and columns. Integers, decimals, timestamps, and other values remain strings so JavaScript cannot round your data.
- Inline cell edits, pending-change highlights, discard, parameterized UPDATE previews, and transactional Apply.
- CSV export of the loaded result, with spreadsheet-formula protection.
- System, light, and dark appearance; a resizable SQL pane; a collapsible sidebar.
- An explicit, in-memory demo with synthetic data. The demo only executes its supplied sample queries.

## Edit a result

```sql
SELECT id, name, status, stock
FROM public.products
WHERE status = 'active'
ORDER BY id
LIMIT 100;
```

Double-click `Monitor stand`, type `Monitor Holder`, and press Enter. The cell is marked as pending. Click **Apply changes**, review the generated statements and parameter values, then click **Apply update**.

The generated statement uses the original primary key and checks the originally loaded value:

```sql
UPDATE ONLY "public"."products"
SET "name" = $1
WHERE "id" = $2
  AND "name"::text IS NOT DISTINCT FROM $3;
```

Parameters are `Monitor Holder`, `1003`, and `Monitor stand`. They are sent separately using PostgreSQL's text parameter format; values are never interpolated into SQL. NULL and empty strings are distinct. Multi-column primary keys are supported.

All staged rows commit in one transaction. If an edited value changed externally, a row was deleted, a constraint fails, or any update affects a number of rows other than one, the batch rolls back. Table identity and projected column metadata are checked again before applying. Database triggers and rules retain their normal PostgreSQL behavior.

Results are editable only for direct columns from one ordinary table and when the entire primary key is selected. Views, joins, expressions, aggregates, DISTINCT, CTEs, inherited/partitioned tables, missing keys, and duplicate projections remain read-only. Primary keys, identity columns, and generated columns cannot be edited in this release.

## Develop

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

Outputs are under `src-tauri/target/release/bundle/`. The current local build is for Apple Silicon on macOS 13 or later. Intel Macs require an x86_64 build or a universal build. Windows and Linux use the same codebase but have not yet received manual platform QA.

Local development builds do not require Apple signing credentials. Public macOS distribution should use a Developer ID certificate and notarization. Signing credentials belong in the release environment's secret store, never in source control.

## Validate

```sh
npm test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npm run test:database
```

The last command starts a temporary PostgreSQL server bound to localhost on a free port. Its credentials are generated for that run, its data is synthetic, and its directory is removed afterward. It tests real updates, composite keys, exact large integers, NULL, SQL-looking values, stale-write conflicts, rollback, read-only results, cancellation, and result limits. It never connects to your own database. PostgreSQL test binaries are development dependencies and are not included in the desktop app.

## Local data and privacy

There is no account, telemetry, hosted backend, or remote UI. The application connects directly to the PostgreSQL host you select.

- Connection metadata is stored in Tauri's per-user application-data directory (`~/Library/Application Support/app.heydb.desktop/` on macOS).
- Remembered passwords use macOS Keychain, Windows Credential Manager, or Linux Secret Service. Without “Remember password,” the password is used for the connection and is not written to settings.
- SQL tabs, loaded results, and pending edits stay in memory. Closing the application discards them; pending edits prompt before closing. There is no persistent SQL history in 0.1.
- CSV files are written only to a destination you choose. Exports include loaded values, excluding staged edits; leading spreadsheet formula characters are prefixed with an apostrophe.
- The application does not log SQL text, credentials, or result contents. Error messages are shown locally.
- Use synthetic data in issues, tests, screenshots, and contributions. Never commit connection settings, database dumps, credentials, certificates, or `.env` files.

TLS defaults to certificate and hostname verification. The unencrypted option is intended for local development. Custom CA selection and SSH tunnels are not implemented yet.

## Current scope

Each Run executes one statement in an application-managed transaction. Explicit transaction/session commands, COPY, and procedures are not supported. Commands that PostgreSQL forbids inside a transaction (such as VACUUM) are therefore not supported yet. There is a 120-second statement timeout and a five-second lock timeout.

Results are limited to 1,000 rows and 8 MiB of retained values. When the limit is reached, the remainder is canceled and that query's transaction is rolled back; oversized write results return an error without committing. A single server row can still require transient memory before its size is checked. Use WHERE, ORDER BY, and LIMIT to browse larger datasets. CSV exports include only loaded rows. Queries are serialized per connection; run separate sessions for independent work.

There is no persistent tab restore, full SQL semantic analysis, grid insertion/deletion, SQL file management, or support for other database engines yet. The Rust database boundary keeps future adapters separate from the interface.

## License

MIT. See [LICENSE](LICENSE).
