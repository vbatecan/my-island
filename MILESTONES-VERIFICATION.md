# Notification-first milestone verification

## Delivered implementation

Presentation arbitration protects user-opened panels, pauses/resumes temporary notifications around volume feedback, restores the current media/idle base, coalesces bursts, bounds retained IDs to20, and expires pending records after60seconds. Existing spring velocity/geometry are retained on interruption.

Windows notification service is opt-in, event-driven, package-identity-gated, and checks/revokes consent. It suppresses startup replay, handles add/update/remove, skips malformed individual records, bounds text/icon caches, and never writes message bodies. UI shows app/title compactly, body on expansion, source-app activation when registered, preview hiding, app filters, and pause/privacy preferences. No Windows notification deletion or reply forwarding is implemented.

Also delivered: origin-aware external-volume feedback, compact media option, focusable settings window, backward-compatible INI settings, optional350ms hover expansion, reduced-motion handling, Ctrl+Alt+Space with conflict reporting, pass-through outside-click/Escape observation, monitor selection/fallback, and event-driven fullscreen/session-lock suppression. Live capture stops while hidden and rebinds to selected monitor/offset. Native regions update only when geometry changes.

## Build/package evidence

- Formatting and `cargo check` pass without warnings; **40 unit tests pass**. Tests include spring physics, coordinator priority/restoration/late deadlines, grouped identity bounds/removal, hover/suppression expiry, pinned snapshots, filtering, privacy/configuration compatibility, safe source activation inputs, and icon/text bounds.
- Normal optimized release build succeeds. Static-CRT MSIX build succeeds with no direct VC runtime, Windows App SDK GUI, or browser-engine dependency. Windows/graphics-driver dependencies remain.
- Unsigned artifact: `target/msix/StillIsland_0.2.0.0_x64_unsigned.msix` (5,896,189bytes).
- SHA256: `65AB5A586D9A4D6D0E34965D004AEA7E977239C0896C659BE76FD119F2293AA1`.
- Windows SDK MakeAppx validates manifest, capability declaration, assets and unpack/block map. Two repacks of the final executable produce identical unsigned package bytes. Source fingerprint remained unchanged during packaging; evidence is in `packaging/final-source-freeze.json`, `final-repeatability.json`, and `target/msix/package-audit.json`.
- Packaging is unsigned: **not installed, not signed, no certificates trusted, and no notification permission requested**. Installation/upgrade/uninstall and real consent/listener delivery must be completed after deliberate signing/trust/install and user consent. See `packaging/README.md`.

## Runtime evidence

Normal release `--verify` and synthetic `--verify-notifications` sequences complete with `active=false`, no startup/runtime errors, correct region/style readbacks, and transparent rendered corners. The notification fixture proves compact/expanded presentation, protected Media panel with pending count, volume interruption, bounded bursts, and return to idle; it does **not** prove receiving private Windows notifications. Real listening is disabled in verification/benchmark modes regardless saved opt-in.

`--verify-settings` captured the actual settings window and read back `focused=true`, `noactivate=false`; the overlay retained its nonactivating/toolwindow styles. Settings and notification snapshots were visually reviewed. No controlled manual pointer/tray test is claimed.

The portable notification probe remains explicitly `UnavailablePortable`, emits no content, and does not ask for permission. The service's four parser/cache/activation-input tests pass. Existing real Core Audio and silent GSMTC fixture probes are retained; no audio playback engine or user-player commands were introduced by the notification tests.

An isolated native fixture exercised both attached monitors at96DPI, including secondary negative coordinates. Explicit selection moved the window correctly; missing IDs fell back to primary. Hotkey registration/unregistration and deliberate conflict reporting passed; input-hook installation/removal and focus preservation passed. Actual user keyboard/pointer, fullscreen game, lock/unlock, physical DPI changes and hotplug scenarios remain unverified.

## Release measurements

Windows11 Pro26300, RTX2050, two attached monitors at96DPI; primary1920x1080. Values are sampled process private committed bytes and resident working set, reported separately in MiB. These runs used the portable release; installed notification provider overhead is not measured.

The full overlay harness sampled a maximum **89.70MiB private /93.73MiB working set**. Idle phase sampled0.95% one-core CPU, rapid reversal stress45.16%, and settled return0.50%. A separate no-capture steady run sampled idle **63.45MiB private /75.18MiB working set /0% CPU**; expanded81.07/90.13/0.28%, Media79.47/89.09/6.23%, Volume81.11/87.68/3.28%. Real media/service events and compilation activity were present; these are observed runs, not isolated laboratory guarantees. The preferred30/45/60MB targets remain missed. These particular sampled maxima stayed below100decimalMB, with limited margin; other materials/providers/hardware can exceed it.

Current GPU sampling found13 valid in-process intervals, with maximum individual-engine utilization **1.516%**. Missing counters after process exit remain missing, not zero. Raw measurements: `milestones-performance.csv`, `milestones-steady-performance.csv`, and `milestones-gpu-performance.csv`.

Opt-in renderer instrumentation measured274 animated draw callbacks with mean draw time1.349ms and p95 inter-frame interval21.736ms in the overlay harness; the notification fixture measured236 callbacks,0.928ms mean draw,18.955ms p95 interval. These callbacks occur **before buffer swap**, so they are not actual presented FPS. PresentMon was unavailable; presentation-rate acceptance remains unverified. Normal operation has no instrumentation callback or reporting loop. Logs: `milestones-overlay.log`, `milestones-notification.log`, and `verification.log`.

## Remaining acceptance work

- Sign/trust/install MSIX deliberately; enable notification consent from the focused settings window; test actual applications, denial/revocation, upgrade and uninstall. Portable builds intentionally lack notification access.
- Verify100/150/200% physical scaling, display reconnection, fullscreen/exclusive games, lock/unlock, and real keyboard/pointer handling. Hooks forward input; no input was synthesized to claim these tests passed.
- Modern Windows Do Not Disturb mirroring is not guaranteed; shell notification eligibility and the app's pause/privacy policy are implemented.
- Eco glass freezes between interaction/window events. Both glass modes exclude the island from ordinary captures to prevent feedback. Capture-provider policies and native acrylic fallback behavior remain Windows-dependent.
- Inline replies, Windows notification dismissal, notification-history persistence, startup registration, cloud and automatic updating remain deferred as planned.
