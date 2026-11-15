# Folder activity verification

Implemented without browser extensions. Enable Settings → Downloads → Monitor folder activity, choose Windows Downloads and/or custom folders, then open Downloads through the existing page navigation. Entries describe filesystem observations, not confirmed download completion. Folder preferences are stored in folders.json alongside settings.ini; activity stays in memory.

Validation:

- `cargo check --quiet` passed.
- `cargo test --quiet --bin still-island`: 205 passed, 2 ignored. Includes native Windows watcher baseline, duplicate registration, temporary-file filtering, final rename, and shutdown; presentation tests cover finite dwell, protected panels and remembered-page restoration.
- `cargo build --quiet` passed.
- `target/debug/still-island.exe --verify-downloads --isolated-material-probe --appearance=solid` ran alongside the user's existing instance without desktop input. It created an owned fixture folder, exercised the production watcher and GUI delivery handler, and reported `downloads PASS real watcher: final file only, silent baseline`.
- Renderer captures: verification/downloads-empty.bmp and verification/downloads-active.bmp. Reviewed the expanded empty/active page and long filename at the current 130% island scale. Initial review identified row alignment and the reused integration icon; corrected the row's explicit text origin and supplied a folder icon.

Limits: native folder-picker interaction and Explorer reveal were not manually exercised. Physical filesystem overflow, disconnected network paths, every Windows text/DPI combination, and reduced-motion/error screenshots were not covered. Excessive event bursts are bounded and reported as skipped changes; this feature does not promise a complete filesystem audit. Folder changes cannot establish transfer percentages, successful download completion, or browser pause/resume controls. Existing entries are historical observations; moved/deleted files may no longer be revealable.

The isolated fixture does not save preferences, listen to real Downloads folders, request notification permission, replace the running instance, or synthesize global input.
