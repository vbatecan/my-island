# Reliability and customization polish

This report covers the current hover/fullscreen, media/lyrics, and customization work. The older reports describe earlier builds.

## Implemented

- Hover/collapse deadlines recheck the native cursor and current window hit target. External foreground/z-order/location changes also reconcile a stationary pointer, including unchanged fullscreen policy; these events do not rebuild desktop capture. DPI change events schedule a deferred pointer reconciliation. There is no hover polling timer at rest.
- Per-executable fullscreen rules use limited-information process queries only when rules exist. Names are validated and normalized once, limited to 64 per list, persisted locally, and matched against the relevant foreground/top app on the selected monitor. Always hide takes precedence; Always show bypasses automatic fullscreen suppression. Session-lock hiding is unconditional.
- Compact media uses available contour width, finite metadata/artwork feedback and a finite play/pause crossfade. Timeline-only updates do not restart decorative motion. Long expanded titles wrap within a bounded region; unavailable/live seeking has an explicit explanation and disabled hit target.
- Lyrics keep seven stable slots with a continuous cursor. Current text grows as it enters focus, previous text shrinks upward, and stable row spacing reserves two wrapped lines without changing pitch per track. Long text is bounded with ellipsis. Blank timed rows show an instrumental/waiting hint. Seeking rebases immediately, normal line changes animate, and motion follows the saved speed/reduced-motion settings.
- Appearance has a fitted live contour preview, finite cancelable motion preview, Balanced/Roomy/Larger text presets, independent 80–140% text size, existing overall zoom, advanced width/height/corner controls, and independent material tint. Presets preserve unrelated integrations and preferences. Invalid app names retain the user's edit for correction.

## Build and automated evidence

- `cargo check --workspace --locked`: passed.
- `cargo test --workspace --locked --quiet`: 156 passed across the workspace; one optional live-provider test ignored. The deliberate invalid-artwork test prints a decoder error while passing its fallback assertions.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed after correcting a collapsible conditional.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- `cargo build --release --locked`: passed on the final sources.
- `scripts/test-polish.ps1`: all four offline regressions passed on the final release, with 145 assertions (18 interaction, 59 layout/settings, 55 lyrics, 13 media polish). The runner writes isolated profiles and logs under ignored `target/polish-probe/` and rejects missing completion markers, failed assertions, nonzero exits, or timeouts.
- Screenshots were inspected for settings, 480px media at 140% text size, compact media, volume, battery, and timer. Review caught and corrected overlong lyrics losing their first words, text-row clipping, and stale validation feedback. The final narrow lyric snapshot begins at the first words and uses a trailing ellipsis.
- Final solid-material media/lyrics checks confirm no additional renderer callbacks or morph/playback timers after settling. Live glass desktop capture is not covered by that idle assertion. No synthetic test is presented as proof of real game or video-player compatibility.

## Verification scope

| Requirement | Evidence to collect |
| --- | --- |
| Rapid pointer entry/exit, child controls, delayed collapse | Deterministic interaction regression and native pointer fixture |
| DPI changes and window movement beneath a stationary pointer | Coordinate regression at 96/120/144/192 DPI; native window fixture; physical scaling separately reported |
| Fullscreen hiding and restoration | Native normal/maximized/borderless fixture, attempted backend re-show, restore |
| Per-application visibility overrides | Persisted settings tests and native fixture in automatic/hide/show modes |
| Exclusive games, video players, multiple physical monitors | Report only runtime cases actually exercised; fixture geometry is not proof for these applications |
| Adaptive compact media and unavailable seeking | Synthetic media snapshots, finite animation settling checks, rendered screenshots |
| Lyrics wrapping, gaps, seek/track replacement and speed | Timeline unit tests, synthetic UI regression and rendered screenshots |
| Preview, presets, independent text size and transparency | Persistence/migration tests, settings callbacks and rendered preview |

## Native runtime evidence

On Windows with an NVIDIA GeForce RTX 2050 and one 1920×1080 display at 96 DPI:

- `scripts/test-fullscreen.ps1 -TraceHover`: all 19 stages passed. Three visits shorter than the configured 200ms dwell remained compact; a sustained visit expanded; six child-control crossings stayed expanded; pointer exit collapsed. A captioned window moved beneath the stationary notch retained hover. A topmost window covering the cursor cleared hover and collapsed; moving it away reopened after dwell.
- Normal and maximized fixture windows kept the island visible. A borderless full-monitor fixture hid it. An attempted backend re-show remained hidden. Restoring the fixture restored the island.
- `-FullscreenOverride Hide`: all five stages passed, including windowed hiding with automatic fullscreen detection disabled.
- `-FullscreenOverride Show`: all four stages passed, including keeping the island visible over the fixture's fullscreen surface.
- These runs had no detected external pointer interference. CSV artifacts are under ignored `verification/fullscreen-*.csv`, including monitor inventory. This supersedes the earlier report's inconclusive physical hover repetition.

The initial 19-stage native run preceded the final lyric-row clipping and test-fixture corrections. The subsequent monitor-aware run below exercised the final release again.

### Monitor-aware probe follow-up

The probe now accepts `-MonitorId` and sequential `-AllMonitors` runs. Both the isolated island and owned fullscreen fixture target the same selected device. Unavailable device names are rejected instead of silently falling back to primary; output files are suffixed per device. An added native assertion checks the actual overlay's top-center position against that display's bounds.

`scripts/test-fullscreen.ps1 -AllMonitors` passed all 20 stages on the final release on `\\.\DISPLAY1` at 96 DPI. Expected and actual physical horizontal centers were both 960. This repeated all hover/occlusion/fullscreen checks after the final UI changes. Windows PowerShell 5 runtime testing caught and corrected a single-result array-unwrapping bug in monitor selection.

The wrapper's `verification/fullscreen-all-monitors.csv` records DISPLAY1 as Passed and the secondary-monitor case as **Unavailable**, because only one monitor is connected. It does not count that entry as multi-monitor or mixed-DPI success. Per-display results are in `verification/fullscreen-hide-DISPLAY1.csv`. Windows display configuration and scaling were not altered.

### Actual video playback

`scripts/test-video-fullscreen.ps1` passed all three stages on the final release with installed VLC 3.0.24. It generated a local silent AVI and used isolated VLC/application preferences, disabled metadata network access, and a short-lived loopback control interface. No user recordings or existing player sessions were used.

| Actual VLC video state | Client bounds | Island |
| --- | --- | --- |
| Windowed | (264,239)–(424,329) | Visible |
| Fullscreen Direct3D11 output | (0,0)–(1920,1080) | Hidden |
| Windowed restored | (264,239)–(424,329) | Visible |

Playback time advanced from 0 to 2 to 4 seconds. This exercises VLC's actual video-output window using its dummy/control interfaces; it does not cover every Qt frontend interaction or exclusive presentation mode. Both owned processes and the control listener exited. Evidence is in ignored `verification/video-fullscreen.csv`. The user subsequently waived game testing.

No memory or GPU benchmark is implied by a successful build or a synthetic UI test. The prior release memory measurements remain historical until a fresh measurement is performed.

## Remaining physical compatibility matrix

These cases require runtime evidence beyond the automated WinForms borderless fixture:

| Case | Pass condition |
| --- | --- |
| Game modes | Game testing waived by the user; no game compatibility claim |
| Other video players/browser video | VLC video-output case passed above; other frontends and players remain unverified |
| Two physical monitors | Selected monitor fullscreen coverage wins even when focus moves to the other; ordinary selected-monitor windows prevent false suppression |
| 100/125/150/200% display scaling | Physical pointer boundary agrees with the silhouette; moving displays/scaling while expanded does not retain stale hover or miscenter the island |

Only one active physical display was found, and no installed game with confirmed exclusive-fullscreen support was identified. Record the application, actual mode, monitor arrangement and Windows scaling for each additional run. App rules are a compatibility fallback, not evidence that the underlying automatic detection worked.

## Completion audit

The current release and saved fixture results were inspected again after implementation. This is the status against each requested polish item:

| Requested item | Current evidence | Remaining gap |
| --- | --- | --- |
| Dependable hover | 18 callback checks; real rapid visits, six child-control crossings, stationary-pointer occlusion/unocclusion and window movement all passed | Physical display-scale changes have not been exercised |
| Fullscreen compatibility and per-app rules | Automatic/hide/show fixtures passed; actual VLC Direct3D11 fullscreen playback passed; rules persisted and validated | Game testing waived; second physical monitor unavailable |
| Adaptive media | Compact and narrow expanded screenshots inspected at 140% text; finite metadata feedback and unsupported seek state passed | No known implementation gap from the audited fixtures |
| Lyrics | 55 lyric checks plus media gap/seek/settling checks; long-line clipping corrected and final screenshot inspected | Coverage remains dependent on the selected player/provider, as described in README |
| Customization | 59 layout/settings checks; preview, presets, independent typography/geometry/tint and validation exercised | No known implementation gap from the audited fixtures |

The physical scaling and multi-monitor items remain unverified, rather than being inferred from the coordinate unit tests or single-display native fixtures. Existing positive results do not need another release build unless production sources change.
