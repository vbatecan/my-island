# Remembered pages and Codex Usage 0.2

The expanded notch cycles through Home, Media, Timer, Volume, then enabled integration views. Wheel movement is accumulated for trackpads and limited to one page per 160 ms. Page changes use a finite content fade/translation while the existing velocity-preserving spring handles geometry. Animation speed and disabled animations are respected.

Leaving collapses to the selected page's compact summary unless a higher-priority activity is present; reentry opens the remembered page. The selection is saved locally. Pinning is preserved across page changes. Disabled or removed integrations fall back to Home. Lyrics, player lists, volume controls, and overflowing integration content retain their wheel behavior; the header remains available for page changes.

Codex Usage 0.2 displays large quota percentages, remaining capacity, reset captions, snapshot age, and restrained progress colors. Its compact summary uses the most constrained reported window. Unavailable windows are not fabricated; stale data is labeled. Generic optional SDK metadata supplies these summaries and semantic typography without hardcoding Codex styling in the host.

## Verification on Windows

- Workspace unit tests: 182 passed, one optional live lyrics-provider test ignored.
- `cargo check --workspace --locked` and `cargo build --release --locked`: passed.
- Workspace Clippy with all targets and warnings denied: passed.
- Formatting: passed.
- All four static-CRT plugin ZIP packages built; Codex package version is 0.2.0.
- Offline plugin process smoke: passed, including quota metadata, stale/error states, request deadlines, and owned-child cleanup.
- Native Slint page regression: all 36 checks passed in both debug and release builds. The suite uses an isolated settings/plugin profile and an offline Codex app-server fixture through actual SDK IPC. Final release log: `target/pages-probe/faa42b88cc844f98b5c5af2379562050/verification.txt`.
- Existing release regressions for hover/timer interactions, layout, lyrics, and media: all four suites passed. Logs and counts: `target/polish-probe/638ab2cd4e5746f3a68d2d09c1e018f8/results.csv`. The interaction fixture now enters the notch before opening Timer and leaves after starting it, matching the page's hover lifetime.

The page suite checks all five destinations, protected wheel regions, compact quota summary, reentry, pinning, 140% text, disabled animations, rapid page changes, stale transition rejection, leave during a transition, disabled-plugin fallback, finite geometry, and stopped rendering/animation timers after settling. Snapshots are written to `verification/pages-codex-{default,text140,compact}.bmp`.

```powershell
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --release --locked
./scripts/package-plugins.ps1
./scripts/test-pages.ps1 -Binary target/release/still-island.exe -Package target/plugin-packages/org.stillisland.codex-usage-0.2.0-windows-x64.zip
./scripts/test-polish.ps1
```

## Installed plugin

The existing enabled Codex Usage installation was upgraded by adding a sibling 0.2.0 directory and updating its version pointer. Plugin preferences and enablement were preserved. Version 0.1.0 was retained for rollback.

## Limits

The page regression uses synthetic pointer/wheel input and deterministic quota data; it is not a physical trackpad, multi-monitor DPI, or live account quota measurement. Screenshots were inspected at the default size and enlarged text size. Large text can require scrolling inside integration content. No new RAM/CPU/GPU benchmark was performed for this change. Existing hardware/fullscreen limitations remain documented in the other verification reports.
