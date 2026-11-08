# Native application launcher

Alt + Space enters the keyboard-focusable launcher on the existing overlay HWND. It uses the existing interruptible geometry springs and content gates; dismissal restores WS_EX_NOACTIVATE and ordinary priority arbitration. Open launcher sessions retain user ownership while notifications queue. Outside click, Escape, repeated shortcut invocation, focus loss, settings/reset, and desktop suppression dismiss safely. The saved carousel destination is preserved.

## Implementation

- `src/platform/launcher.rs`: per-user/common Start Menu discovery plus Shell AppsFolder enumeration; Shell-native launching and physical-size native icons. COM interfaces and GDI handles stay on the worker and are released there.
- `src/launcher.rs`: exact/prefix/word-prefix/substring/fuzzy ranking, deterministic tie-breaking, recent-first suggestions, bounded local history with atomic replacement.
- `src/app/launcher.rs`: on-demand discovery/icon/launch tasks, six displayed results, bounded 128-entry icon cache keyed by app ID and physical size. No periodic application scan or idle launcher worker. Catalog refresh occurs on reopen after 60 seconds.
- `ui/components/app-launcher.slint`: keyboard search, native artwork, selection, empty/error states, and readable Enter/Escape hints. Geometry and text sizing remain separate.
- Settings → Behavior: shortcut enablement, Alt + Space / Ctrl + Space / Alt + Shift + Space, and clear recent launches.

## Verified

- Workspace tests: 192 passed, one optional live lyrics-provider test ignored. New tests cover ranking tiers/Unicode, recency, missing and duplicate IDs, persistence/corruption/bounds, shortcut filtering, and RGBA conversion.
- Native Shell probe on this machine: 217 entries (183 Start Menu shortcuts, 34 additional AppsFolder entries), no warnings.
- Native icons: 12 shortcut and 12 packaged-app extractions across 32/48/64 pixels.
- An owned marker executable launched via a temporary shortcut under `target/`; spaced arguments and working directory were preserved. No user application was launched and no Start Menu entries were changed.
- Debug native GUI: shortcut delivered through WM_HOTKEY, real foreground focus, text input, Down Arrow selection, Escape, Enter launch, persisted successful history, queued notifications, and stopped spring frames all passed. Screenshot inspection caught and corrected search-field positioning and a missing Enter glyph.

```powershell
cargo fmt --all -- --check
cargo check --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release --locked
./scripts/test-launcher.ps1 -Configuration release
./scripts/test-pages.ps1 -Binary target/release/still-island.exe
./scripts/test-polish.ps1
```

The launcher test uses an isolated local profile, registers its own shortcut, injects that finite registered chord, and launches only its own marker fixture. Native tests return fixture manifests for reuse.

## Release results

The optimized release build passed all 18 launcher UI assertions, including actual WM_HOTKEY activation/focus, typed filtering, keyboard selection, Enter launching the owned fixture, persisted recency, notification protection, settings chord changes, and clean shortcut disablement. Log: `target/launcher-ui/4f280d1330a74f6a8c918e301187a7b8/verification.txt`.

All 38 release page-carousel assertions also passed after integration, including media priority, compact Codex restoration, protected wheel regions, interrupted morphs, and stopped rendering after settling. Log: `target/pages-probe/eb0f6694c2ed4559b2f23792f5b7e660/verification.txt`.

All 145 existing release UI assertions passed across hover/timer interactions, layout, lyrics, and media polish. Logs: `target/polish-probe/142b5073bea74bfd8b77bd8950e4fec3/results.csv`. Formatting, workspace type check, Clippy with warnings denied, and release-tool validation passed. The final default-size launcher screenshot was independently inspected after the layout corrections.

Measured with Windows `Process.PrivateMemorySize64` (private bytes/commit) and `Process.WorkingSet64` (resident working set including shared pages), in a compiled release build using a fresh isolated profile, solid material, no enabled plugins, and synthetic playback disabled:

| Point-in-time sample | Private bytes | Working set |
| --- | ---: | ---: |
| Idle, 1.3 seconds after process launch | 70,832,128 B (70.83 MB) | 74,723,328 B (74.72 MB) |
| Launcher open with catalog and icons, 3.2 seconds after launch | 110,108,672 B (110.11 MB) | 116,211,712 B (116.21 MB) |

Raw measurements: `target/launcher-ui/4f280d1330a74f6a8c918e301187a7b8/memory.csv`. Both sampled metrics were below the revised 150 MB budget. This is not a peak, long-duration, CPU/GPU, or every-material/plugin benchmark. Native font/renderer caches and the larger persistent transparent surface contribute to resident/committed memory; live glass and third-party plugins can change totals.

## Limits

Shell launch success means Windows accepted the request, not that a third-party application finished starting. Explicit shortcut conflicts and Windows foreground restrictions are reported instead of bypassed. Reopening refreshes app installation changes; there is no background catalog watcher. Physical multi-monitor/DPI transitions, every third-party shortcut and IME, and graphics-driver variations still require device coverage. No system-wide file search, web search, executable command input, telemetry, or network service was added.
