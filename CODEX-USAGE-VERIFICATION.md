# Codex Usage plugin verification

Implemented under `examples/codex-usage/`. All bundled plugin example sources now live in `examples/`; the runtime host and shared SDK remain in `src/plugins/` and `crates/`. Workspace membership, documentation and package CI use the new locations. The packager discovers each example manifest and builds all four Windows ZIPs automatically.

## Verified on Windows

- `cargo check --workspace --locked`: passed.
- `cargo test --workspace --locked --quiet`: 165 passed, one optional live lyrics-provider test ignored. Nine tests belong to Codex Usage, covering bounded protocol handling, cancellation, response sanitization, quota parsing, absent windows, invalid percentages, expired reset handling, and host manifest/layout validation.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- `cargo build --release -p still-island-plugin-codex-usage --locked`: passed.
- `scripts/package-plugins.ps1`: all four static-CRT Windows x64 packages built successfully. The Codex ZIP contains only its manifest and executable. Its SHA-256 matches `target/plugin-packages/SHA256SUMS`.
- `scripts/plugin-smoke.py --binary-dir target/plugin-build/x86_64-pc-windows-msvc/release`: passed developer, Obsidian, recoverable configuration errors, and the new offline Codex process suite. GitHub's optional live API smoke was not requested or run.

## Actual account read

`target/release/still-island-plugin-codex-usage.exe --check` succeeded with installed Codex CLI 0.166.0 and the existing ChatGPT sign-in. The service returned one 7-day window at **22% used**, with a reset timestamp. No primary window was returned, and the plugin did not invent one. This was a point-in-time account read, not a default value or future guarantee. Only normalized quota information was printed; no credentials or raw account responses were displayed.

The read uses the documented [Codex app-server account rate-limit method](https://learn.chatgpt.com/docs/app-server#6-rate-limits-chatgpt). No inference/thread was started. The temporary child was terminated and reaped after the request; existing Codex instances were not terminated.

## Offline process coverage

The test fixture is a separate disposable Rust executable under ignored `target/`, built from `scripts/fixtures/codex-app-server.rs`. It is not shipped in the plugin, and the production binary contains no fake quota mode. Its only observed methods were `initialize`, `initialized` and `account/rateLimits/read`.

- Ready/initial view is immediate; no account child starts until a view or refresh requests data.
- Primary/secondary fixture windows report 25.5%/63%, displayed as normalized progress and actual 5-hour/7-day durations.
- Ten rapid refresh requests acknowledge promptly and share one child request.
- Show in notch emits a declared activity using reported snapshot values.
- A failed refresh retains and labels the previous snapshot. Remote secret-like error text is absent from stdout/stderr, including trailing output.
- Unknown actions fail recoverably; missing executables show unavailable data rather than zero usage.
- Changing executable configuration during a pending request cancels/reaps the old child and rejects its stale result.
- Output exceeding 64 KiB is rejected. The eight-second deadline leaves IPC responsive and supports a subsequent successful refresh.
- Last measured timeout appeared 7.973 seconds after observing child startup; hung-child shutdown and reaping took 0.061 seconds. These are fixture observations, not general performance guarantees.

The synthetic process suite never starts real Codex, accesses a real auth file, or makes a network request. The separate live check above intentionally performs a real account lookup.

## Usage and limits

Import `target/plugin-packages/org.stillisland.codex-usage-0.1.0-windows-x64.zip` in Settings → Plugins, then enable Codex Usage. See [setup instructions](examples/codex-usage/README.md).

Updates are on demand, with an explicit brief notch preview; this is not an always-running usage polling service. API-key-only or unsupported quota accounts show an unavailable state. The GUI plugin was not silently installed/enabled into the user's existing profile; process/manifest/layout tests and the live account read establish the evidence above.

The old repository-root `plugins/` directory is empty. Its removal was rejected by automatic approval review (the tool reported only “blocked by policy”), so it was left in place. It contains no plugin sources and is not a workspace member.
