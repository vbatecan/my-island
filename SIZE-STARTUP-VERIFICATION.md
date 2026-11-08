# Island size and Windows startup — 2026-11-08

Settings → Appearance offers **Island size, 75–150%** (default 100%). It scales the entire overlay, including idle/compact shapes, media panels, typography, artwork, and controls. Settings itself stays at normal desktop size. Preferences persist as `island_scale`; 1.25 means 125%.

Native Slint transforms scale an 800×300 design canvas. The physical window, rounded region, glass sampling, and raw pointer conversion use Windows DPI × user size. Windows DPI is never spoofed, and child controls retain Slint's inverse pointer transforms. Desktop top offset remains an independent desktop logical-pixel setting. Monitor-fit geometry accounts for user size and spring overshoot; narrow media layouts remain responsive. Changing size clears stale pointer/wheel state and refreshes glass capture bounds. No resize polling or extra rendering timer was introduced.

Settings → Behavior offers **Start with Windows**. Its checked state reflects native registration, refreshed at launch and when settings reopen. Reading a saved INI boolean never enables autorun. Native operations are serialized, the checkbox disables while a request is pending, and failures roll back to actual readback.

- Portable/installer builds use the per-user `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` value `StillIsland`, containing a quoted absolute executable path. Keep portable executables in a stable location. Enabling explicitly replaces an older copy's registration; disabling and installer removal only delete this copy's matching command.
- MSIX declares the initially disabled `StillIslandStartup` desktop StartupTask and uses native `GetAsync`, `RequestEnableAsync`, and `Disable`. WinRT initialization remains on the initiating GUI thread across awaits.
- Windows user/policy restrictions are respected. The portable branch reads Windows startup approval flags conservatively; it never writes them to override a Startup Apps choice. The UI explains disabled/unsupported states.

## Validation

Formatting, workspace type checks, Clippy with warnings denied, release compilation, and **139 workspace tests** passed. The optional live lyrics-provider test remains excluded from ordinary tests; its earlier explicit result is recorded in [lyrics verification](LYRICS-VERIFICATION.md).

Four native startup tests passed: Unicode/quoted paths and command limits, packaged task states, real read-only status without registration changes, and actual registry enable/readback/disable using a unique app-local test subtree. Ownership preservation and Windows-disabled startup were covered. No real autorun entry was enabled by development tests. MSIX MakeAppx pack/unpack schema validation and Inno Setup 6.7.4 compilation passed with an existing static CRT executable; these packaging checks do not establish installed startup behavior.

Release `--verify-size --appearance=solid` passed at 75%, 100%, 125%, and 150% on the primary 1920×1080, 96-DPI display. It checked actual HWND dimensions, logical shape geometry, native region membership, `WindowFromPoint` routing inside/outside both edges, plain-lyrics wheel routing, idle settling, and unchanged saved preferences. Renderer screenshots were reviewed: `verification/size-75.bmp`, `size-100.bmp`, `size-125.bmp`, and `size-150.bmp`. Mathematical tests cover combined DPI/size and glass sample alignment; additional physical DPI scales and runtime glass capture at each size were not tested.

The same release passed contracted-corner and lyric-motion regressions, including interrupted lyric movement, reduced/disabled-animation handling, and no further renderer frames after settling.

```powershell
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --release --locked
.\target\release\still-island.exe --verify-size --appearance=solid
.\target\release\still-island.exe --verify-contracted-corners --appearance=solid
.\target\release\still-island.exe --verify-lyrics --appearance=solid
```

Actual launch at the next Windows sign-in and installed MSIX enable requests were not performed. No reboot, logoff, package installation, certificate trust, or real startup enablement was used as a test. Resource usage was not rebenchmarked for enlarged islands; the earlier memory-target gap remains documented in the lyrics report, and larger surfaces can increase rendering memory.
