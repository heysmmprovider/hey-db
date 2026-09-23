# Contributing

Keep changes focused on PostgreSQL workflows and include a short explanation of the user-facing behavior. Follow the development and validation commands in the README.

Use synthetic fixtures only. Never attach a real connection string, password, certificate, database dump, private host name, or customer data to an issue or pull request. Redact screenshots before sharing. Run `git diff --cached` before committing.

For changes to editing or query execution, extend the isolated PostgreSQL integration test. In particular, preserve key-based row identification, parameter binding, the NULL/empty-string distinction, rollback on errors, and concurrent-edit checks. UI checks should include keyboard editing, dark mode, and the minimum 820 × 580 window size.

The UI lives under `src/`. Tauri commands and PostgreSQL sessions live under `src-tauri/src/`. `edits.rs` owns SQL eligibility and update planning; the Rust side is the authority for editable metadata and snapshots. Never trust client-provided table names or primary keys for Apply.
