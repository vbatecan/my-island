# Still Island

The launcher also searches Windows settings and tools (try `Bluetooth`, `uninstall programs`, or `Task Manager`). Type `(50 + 20) / 2` or `5 ft to cm` for local fend calculations, or `100 USD to SGD` for currency conversion. A selected answer copies with Enter and contracts the notch. Currency uses Frankfurter reference rates cached for six hours; only the currency pair is sent, never the amount. Settings → Behavior contains independent provider toggles. Expressions are not saved.


Press **Alt + Space** to morph the notch into a native application launcher. Search Start Menu shortcuts and installed Windows apps, select with Up/Down, launch with Enter, and dismiss with Escape or an outside click. Settings → Behavior lets you disable the shortcut or choose Ctrl + Space / Alt + Shift + Space. Shortcut conflicts are reported in Settings. Ctrl + Alt + Space retains its existing island-panel toggle.

Search ranks exact names before prefixes and fuzzy matches. Blank search shows recent successful launches, followed by alphabetical suggestions. Recent IDs stay locally in `launcher-recent.json` next to settings; clear them in Settings. Windows resolves shortcut arguments and working directories. Native icons are loaded for the current DPI and held in a bounded cache. Discovery and icon work run on demand; reopening after a minute refreshes the catalog without background polling.

The launcher shares the persistent overlay and its interruptible spring geometry. It receives keyboard focus only when explicitly invoked. Notifications wait while you type, and dismissal restores normal presentation priority without overwriting the remembered page. The resource budget for this two-mode application is now **150 MB maximum**, with release private bytes and working set measured separately; this is a target, not a universal guarantee.

The latest polish adds native hover deadline checks, per-app fullscreen rules, finite media feedback, clearer lyric gaps, and a live settings preview with presets and independent text sizing. See [current verification and runtime limits](POLISH-VERIFICATION.md); [earlier spacing and hover results](SPACING-HOVER-VERIFICATION.md) remain historical.

In **Settings → Appearance**, choose Balanced, Roomy, or Larger text, then apply the preset. Text size changes typography separately from overall zoom and the advanced width/height controls. The fitted preview can show contracted, expanded, and media contours; Preview motion runs locally without leaving settings. Material tint remains independent. Acrylic/glass preview is an approximation of transparency; actual desktop composition depends on Windows.

In **Settings → Behavior**, enter executable names under the per-app fullscreen rules and press Enter or **Apply app rules**. Always hide also hides over a windowed instance of that app, even with automatic fullscreen hiding disabled. Always show overrides automatic fullscreen hiding for that app. Clear a name to restore automatic behavior. Hide takes precedence, and session-lock hiding always applies.

After building, `powershell -File scripts/test-polish.ps1` runs isolated, offline UI regressions. Exit the normal island first. `scripts/test-fullscreen.ps1` exercises native hover and borderless fullscreen; use `-FullscreenOverride Hide` or `Show` for rule tests. Native hover tests briefly move the pointer and report external cursor movement as inconclusive. Synthetic fixtures do not establish compatibility with every game, video player, or physical monitor arrangement.

Use `scripts/test-fullscreen.ps1 -MonitorId '\\.\DISPLAY2'` to target a connected display, or `-AllMonitors` to run the matrix sequentially on each connected display. The probe sets only its isolated island profile, validates top-center placement, and writes per-display CSVs. It refuses an unavailable device instead of silently testing primary. It does not change Windows scaling; rerun after manually choosing the scale you want to validate. A single-display run explicitly records that physical multi-monitor and mixed-DPI coverage is unavailable.

With VLC already installed, `scripts/test-video-fullscreen.ps1` generates a silent clip and verifies actual video windowed/fullscreen/restored behavior using isolated profiles. It requires Python 3; use `-VlcPath` for a custom installation location. It never opens your recordings.

The latest [optimization measurements](OPTIMIZATION-VERIFICATION.md) still miss the original RAM targets. Cache/allocation optimizations are implemented; measured savings are modest and the remaining baseline requires allocation/graphics profiling.

## User-created integrations

The **Integrations** hub supports trusted external Windows plugins. Import a package in Settings → Plugins, explicitly enable it, configure its fields, then open its native view or pin an action. Importing never executes a plugin. Editable plugin forms use a focusable manager window; the notch stays nonactivating.

Authors describe rows, columns, text, images, buttons, progress, toggles, inputs, and lists through a versioned JSON-lines protocol. The host owns native rendering, clipping, springs, and activity priority. Plugins can request brief activity previews; they cannot displace an open/pinned panel. Protocol compatibility, bounded messages/layouts, package paths, process timeouts, and cleanup are validated. These are trusted programs with your Windows account access, not sandboxed extensions.

The core works without plugins enabled. Online lyrics and configured integrations use optional network access. Secret configuration uses Windows protection and is excluded from ordinary configuration files and diagnostics. Reference ZIPs are separate from the installer and are never silently installed or enabled.

All bundled plugin sources live under [examples/](examples/README.md). Start authoring with the [SDK tutorial](crates/plugin-sdk/README.md). Build the Codex Usage, developer, Obsidian, and GitHub Actions packages using `powershell -File scripts/package-plugins.ps1`; packages and SHA-256 checksums are written under `target/plugin-packages/`. The Obsidian example uses local URI actions; GitHub Actions uses an explicitly configured read-only token for private repositories and opt-in background monitoring.

[Codex Usage](examples/codex-usage/README.md) displays server-reported used/remaining percentages, quota windows, reset timing, and an explicit compact notch preview. It uses your existing Codex sign-in through the documented app-server interface. Open its Usage view or choose Refresh for a new snapshot; it does not poll in the background or run inference. Import `org.stillisland.codex-usage-0.2.0-windows-x64.zip` from `target/plugin-packages/` through Settings → Plugins, then enable it.

Scroll the **expanded** notch to cycle **Home → Media → Timer → Volume → enabled integration views**. The header shows the current page and its position. Hovering or clicking reopens the remembered page. The selection is remembered across restarts. Scrollable lyrics, open player lists, volume controls, and overflowing integration content keep their own wheel behavior; scroll the header to change pages from those views. Compact wheel volume and middle-click mute remain configurable. Disabled or removed integration pages fall back to Home.

The **contracted** notch prioritizes **Notifications → urgent/temporary events → direct feedback → playing media → active countdown → remembered page → idle**. Temporary events include countdown completion, battery previews, and explicit integration activity previews. Direct feedback is volume/mute feedback. Playing media takes priority only while playback is active and compact now-playing is enabled; paused or unavailable media yields. Interruptions never replace the remembered expanded destination. Open/pinned panels remain under user control, and existing notification privacy/fullscreen settings still apply.

Codex Usage 0.2 adds large quota metrics, aligned reset labels, understated progress colors, and a persistent compact summary for the most constrained reported window. Plugins can provide backward-compatible semantic text roles and optional compact summary metadata; no host code depends on the Codex plugin's ID for its visual styling.

The default shape is a top-edge **Notch**, with 8px curved shoulders and separate **contracted and expanded bottom corner settings**, both defaulting to 8px. Change either radius from 0–24 logical pixels under Settings → Appearance. Contracted corners apply to idle and compact states and are capped at half the current height; choose 24px to restore their fully rounded bottoms. Choose **Island** to restore the floating capsule with 12px expanded corners; its saved top offset is preserved. Materials and all existing media, timer, notification, hover, pin, and volume features work with either shape. `--form=island` provides a session-only override for testing.

Battery activities use Windows power events without polling. Charging changes and low-battery crossings at 20% and 10% show quiet, three-second compact previews. The launcher offers battery details when a system battery is present. Disable previews in Settings → Behavior. Startup status is silent, unknown values are never shown as fabricated percentages, and desktop PCs without a system battery hide this feature. Preview arbitration protects open panels and higher-priority activity.

`--verify-battery --appearance=solid` runs a synthetic battery UI regression without changing Windows power settings or requiring a physical charger transition. See [NOTCH-VERIFICATION.md](NOTCH-VERIFICATION.md) for verification boundaries.

A native Windows 11 island written in Rust and Slint, with interruptible spring motion, real Windows volume and media controls, and selectable materials. No browser engine, network service, telemetry, or audio playback engine.

In the media panel, click **Automatic** (or the chosen player's name) to select a Windows media session. Automatic follows playing sessions. The compact playback line updates once per second while playing; expanded seeking supports click or drag, previews locally, and sends one seek on release. Players must expose Windows media controls and seeking support.

The media panel expands to **640×216 logical pixels**, with lyrics on the left and playback controls on the right. On narrow displays it reduces its width; below 480px it prioritizes the playback controls. Automatic lyrics lookup uses [LRCLIB](https://lrclib.net/docs) when the media panel is open. It sends track title, artist, album, and duration over HTTPS; it does not record audio. Disable it under Settings → Behavior. Timed lyrics follow the selected player's timeline, including pause and seek; untimed lyrics are scrollable. Scrolling that lyrics viewport leaves system volume unchanged. Instrumental, unavailable, and unmatched tracks have explicit states. Coverage and synchronization depend on the provider and the media publisher; no title guessing or audio recognition is used. Results are cached only in memory, with no idle network polling or additional playback timer.

See [lyrics and notch verification](LYRICS-VERIFICATION.md) for tests, native boundary checks, screenshots, resource measurements, and remaining limits.

Timed lyrics glide upward over 300ms as a line finishes, with fading context and smooth emphasis. Interrupted steps preserve the rendered position; track changes and large/backward seeks rebase directly. Animation settings and Windows reduced motion apply. The in-panel sync/provider footer has been removed; lookup details remain in settings.

In **Settings → Behavior**, tune expansion and collapse delays. Moving across the island resets expansion dwell; reentering cancels pending collapse. Use the header pin to hold a panel open. The close control and Escape still close a pinned panel. Scroll-to-volume and middle-click mute can be enabled independently; wheel sensitivity is adjustable. Scrolling inside the player chooser scrolls its list instead of changing volume.

Open **Countdown timer** from the tray or the launcher footer. Choose a preset or adjust minutes, then start. The countdown returns to compact status; click it to pause, resume, reset, or change duration. Completion briefly expands for 2.5 seconds, then restores the previous media/idle base. Paused timers keep their remaining time. Timers are local to the current run and stop when the application exits.


## Notification-first release

The next milestones are implemented: presentation arbitration, opt-in Windows notifications, automatic volume feedback, optional compact media, a real settings window, monitor selection, fullscreen hiding, and a global shortcut. See [MILESTONES-VERIFICATION.md](MILESTONES-VERIFICATION.md) for current evidence and remaining installation/device tests.

Open **Settings…** from the tray. Settings is a normal focusable window; the island remains nonactivating. Appearance, privacy/app filters, behavior, and display preferences save locally. Ctrl+Alt+Space toggles the panel by default; registration conflicts are reported and disable that shortcut. Outside clicks and Escape close an open island panel without swallowing the original input. Hover expansion is optional (off by default,350ms delay); Windows reduced-motion settings are respected unless overridden.

Notifications require the identity-bearing MSIX build, Windows consent, and the in-app opt-in. The portable build deliberately disables access-request controls and explains how to obtain identity. Package output and signing/install instructions are in [packaging/README.md](packaging/README.md). Build with `powershell -File scripts/package.ps1`; no certificate is trusted and no package is installed by the script.

After installing a properly signed/trusted package, exit any portable instance, launch Still Island, open Settings → Notifications, and choose Enable access. Compact previews show app/title; expand to read the body. Open app activates the registered source application, not necessarily the original message. Hide preview never clears Windows Notification Center. Notifications are opt-in; pause, per-app filtering, and identity-only privacy are available. Existing notifications are baselined without replay. Pending notifications expire after60s; dwell defaults to5s and pauses while hovered. At most20 notification identities are retained in the presentation queue, with same-app bursts grouped. Windows banners are unchanged.

User-opened panels are protected. Hover previews contract after the configured exit delay; hovering opens active countdown controls, media controls, or the launcher. External volume/mute changes show1.5s feedback, or a small chip inside an open panel; local slider writes do not create duplicate popups. Compact now-playing defaults on and shows artwork, track title, artist, and play/pause; paused sessions remain accessible. Existing saved opt-outs are respected. The coordinator restores the current timer/media/idle base after temporary events. Source metadata remains authoritative; no playback engine or fake notification feed is used during normal operation.

Monitor selection falls back to primary if a saved display is absent. Fullscreen/lock suppression is event-driven and stops Live glass capture while hidden. Shell notification eligibility is checked before automatic previews; this is not a claim of complete modern Windows Do Not Disturb synchronization. No message bodies or notification history are persisted.

Development checks: `--verify-notifications` uses synthetic records and never asks for permission; `--verify-settings` captures the actual settings window and focus/style readback. `--verify-interactions --appearance=solid` exercises hover callback timing, reentry, pin/unpin, and countdown pause/resume/completion with renderer snapshots; it never synthesizes desktop input or changes the user's volume or player selection. Real notification listening is disabled in every verification/benchmark mode to prevent screenshots of private notifications. Renderer metrics are opt-in and measure pre-swap render callbacks, not displayed FPS.

## Build and run

Install stable Rust (MSVC) and Visual Studio Build Tools with Desktop development with C++ and the Windows SDK. Minimum Rust1.92; built here with1.99.1, Slint1.18.1, windows0.62.2.

```powershell
cargo run --release
# Optional state selection for development:
cargo run --release -- --demo --demo-state=media
# Temporary overrides; tray choices are persisted instead:
cargo run --release -- --appearance=glass --opacity=0.55 --bounce=lively
```

Executable: `target/release/still-island.exe`. One instance per Windows session. Click the capsule to open; click the black background or close button to collapse. Media and Volume tiles open their respective panels. The island uses a persistent transparent base canvas of 800×300 units, scaled by the selected island size and Windows DPI, with a native contour region for desktop click-through outside its silhouette.

## Customization

Open the tray icon, then Settings:

- **Material:** Solid black, Transparent, Acrylic blur, Eco liquid glass, Live liquid glass.
- **Transparency:** light25%, balanced55%, or dark80% black tint. Solid black stays opaque.
- **Spring response:** Soft, Balanced bounce, Lively bounce. Balanced is the new default; expansion has a visible but restrained overshoot, and collapse uses more damping to avoid excessive squashing. Retargeting preserves momentum.
- **Notch corners:** separate contracted and expanded radii, each 0–24 logical pixels with an 8px default. Changes retarget the current contour without restarting its content animation.
- **Island size:** 75–150%, default 100%. Scales the entire island, including contracted geometry, text, artwork, and controls. Try 125% for a larger island; narrow monitors retain responsive media layouts. Windows DPI and desktop top offset keep their independent meanings.
- **Width and height:** independently adjust contracted, expanded, and media layouts without enlarging text. Compact status cards keep a 220×48 minimum. Zoom now rasterizes fonts at their final sizes instead of stretching the parent surface.
- **Animation speed:** 50–200% under Appearance → Motion. Changes morph and lyric motion while preserving real playback/countdown timing and spring bounce. Settings uses native navigation tabs. See [layout verification](LAYOUT-VERIFICATION.md).
- Animations, always-on-top, and top offset0/6/12 logical pixels.

Preferences are stored in `%LOCALAPPDATA%\StillIsland\settings.ini`. Numeric opacity can also be edited there between0.15 and0.95. The tray has Show Island, Reset Island, Settings, and Exit. `--demo` exposes state selection; it does not replace real services with mock data.

**Settings → Behavior → Start with Windows** controls native startup registration. Portable and installer builds use a per-user Run entry with the quoted executable path; keep portable copies in a stable location. MSIX uses its opt-in Windows StartupTask. The checkbox reads actual Windows state, respects user/policy restrictions, and refreshes when settings reopen. Merely loading a saved preference never enables startup. See [size and startup verification](SIZE-STARTUP-VERIFICATION.md).

## Real controls

Volume controls the **default Windows multimedia output device**, including mute. Drag the slider, or click the speaker to mute/unmute. The compact state also offers volume. External changes and default-device replacement use Core Audio notifications, not polling. A missing output device is shown explicitly.

Media uses Windows **Global System Media Transport Controls**. Start playback in a player that publishes a Windows media session, then open Media. The panel displays actual track/artist/artwork and playback state; play/pause, previous, next, and seeking are sent to that publisher. Unsupported actions are disabled. No session shows an honest empty state. The application does not create playback or invent tracks. A four-times-per-second display clock runs only while the Media panel is open and the selected session is playing; it stops on pause/collapse. Provider timeline events remain authoritative. Seek is committed on release to avoid queuing remote commands during a drag.

Not every player exposes SMTC or every transport capability. If a player does not publish a session, this app cannot control it through GSMTC. No global keyboard-media-key fallback is used, because that could control a different application.

## Materials and glass mathematics

Transparent material uses the desktop composition surface directly. Acrylic uses a sampled, blurred desktop texture clipped to the exact island contour and the selected tint. The whole-window Windows backdrop is deliberately disabled: it can paint the persistent transparent canvas gray outside the notch. Acrylic refreshes on desktop/window events rather than continuously.

Both glass modes have actual **backdrop refraction**, rather than only a shiny border. The overlay is excluded from capture to prevent feedback. Pixels are filtered and refracted into a Slint bitmap; Slint continues to render the UI and spring morphs.

- **Eco glass** (`--appearance=glass`) samples the desktop through GDI on hover/click/state transitions and foreground/top-level-window changes. It has no capture worker or refresh timer. Its backdrop stays frozen between these events, so an underlying video does not update continuously.
- **Live glass** (`--appearance=live-glass`) uses borderless Windows Graphics Capture, a low-memory copy device (with hardware fallback and a developer `--capture-hardware` comparison option), one buffer, dirty-region checks, and a30FPS backdrop ceiling. Only the canvas crop is read back. Significant pixel changes are coalesced; tiny compositor dithering is ignored. This30FPS ceiling applies to the backdrop, independently of spring animation scheduling.

For a point `p` relative to the shape center, half-size `b`, radius `r`:

```
q = abs(p) - b + r
d = length(max(q, 0)) + min(max(q.x, q.y), 0) - r
```

The outward gradient of this signed distance defines the direction of the bezel slope. A circular height profile across the inside edge yields a normalized3D surface normal `N`. Refraction uses Snell's law in vector form, with incident ray `I=(0,0,-1)` and air/glass ratio `eta=1/1.45`:

```
k = 1 - eta² (1 - dot(N,I)²)
T = eta I - (eta dot(N,I) + sqrt(k)) N
sample_offset = T.xy / -T.z * optical_depth
```

Bilinear backdrop sampling follows that offset. Schlick's approximation adds grazing-angle reflection: `F=F0+(1-F0)(1-N.z)^5`, with `F0=((1.45-1)/(1.45+1))²`. A separable soft prefilter reduces competing backdrop text. The implementation is in `src/material.rs`, with finite-ray, normal-incidence, SDF-gradient, interpolation, and bounce tests. This is a screen-space lens approximation, not full scene ray tracing.

Research references: [Windows backdrop types](https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwm_systembackdrop_type), [Khronos GLSL refraction specification](https://registry.khronos.org/OpenGL/specs/es/3.1/GLSL_ES_Specification_3.10.pdf), [Microsoft's native composition/capture samples](https://github.com/microsoft/Windows.UI.Composition-Win32-Samples), and [capture exclusion semantics](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity).

Live glass is an optional higher-cost material. See [current material verification](MATERIALS-VERIFICATION.md) for release memory and CPU measurements; older MVP figures do not represent the launcher-sized scenes. Its capture resources exist only while selected. Eco glass avoids that capture stack; unchanged sampled backdrops do not cause UI updates. The WinEvent hooks exclude the overlay's own process and ignore child-control movements, preventing refresh feedback. **Acrylic and glass are omitted from normal screen captures/recordings** because capture exclusion is required to prevent feedback. Internal `--verify` captures can still render it. Backdrop pixels stay in RAM; they are never sent over the network or written to disk during normal use. Explicit verification captures write local BMPs. If borderless capture is unsupported, live glass falls back to acrylic with an error message, without requesting a permission prompt or changing Windows security settings.

## Architecture and verification

`src/animation` holds exact damped-spring integration and choreography gates. `src/app.rs` coordinates startup, presentation, and geometry; `src/app/` separates media, artwork, lyrics, volume, interaction, materials, settings, notifications, and verification orchestration. `src/state.rs` defines shared geometry and expanded-state classification. `ui/island.slint` composes the island using reusable controls from `ui/components/`. `src/platform` owns Win32 styles, DPI/monitor events, tray, native regions, and optional capture. `src/services` owns event-driven Core Audio and GSMTC workers plus the lazy lyrics lookup worker. All platform unsafe calls document their invariants. Workers sleep on channels when idle; no service polls for audio or track data.

```powershell
cargo fmt --all -- --check
cargo check --locked
cargo test --locked
cargo build --release --locked
powershell -File scripts/test-media.ps1
powershell -File scripts/measure.ps1
powershell -File scripts/measure.ps1 -Appearance glass
powershell -File scripts/measure.ps1 -Appearance live-glass
```

The media probe publishes its own silent native SMTC fixture, refuses commands unless that fixture is current, verifies play/pause/previous/next/seek, and closes it. It never plays audio or controls a user's player. `--verify` exercises real states and rapid reversals, then exits; live pointer input is disabled only in verification/benchmark modes. `--benchmark` adds no orchestration timer. See `POLISH-VERIFICATION.md` for current results. `VERIFICATION.md` is the original MVP baseline.

## Remaining limits

Primary-monitor support and native startup registration are implemented; additional physical DPI scales, live monitor topology changes, actual next-sign-in launch, and packaged StartupTask enablement still require device testing. No media playback engine, cloud, or telemetry. Installed notification consent and fullscreen/hotplug behavior require the device tests recorded in the current milestone report. Acrylic and glass require supported desktop capture/exclusion behavior. Actual presentation FPS and exhaustive manual rounded-edge clicking have not been measured. Material captures and native hit-routing readback are verified separately.

The original source is MIT licensed. Dependencies retain their own licenses, including Slint's applicable GPL/commercial/royalty-free options.

## Automated Windows releases

Repository: https://github.com/vbatecan/my-island (private).

Pull requests and pushes to main run formatting, type checks, Clippy,40unit tests, and release-tool tests. Workflows and compiler downloads are pinned; workflow definitions are validated with actionlint.

To publish a version, update `[package].version` in Cargo.toml and refresh Cargo.lock, commit the change, then push the matching stable tag:

```powershell
git tag v0.2.0
git push origin v0.2.0
```

The release workflow validates the tag, reruns checks, builds a self-contained x64 executable, compiles a per-user Windows Setup EXE, builds MSIX, assembles a portable ZIP, and publishes all artifacts with SHA256 checksums. It creates a draft first and publishes only after all assets upload. Already-published tags are not overwritten.

Optional signing uses repository Actions secrets `WINDOWS_CERTIFICATE_PFX` (base64), `WINDOWS_CERTIFICATE_PASSWORD`, and `WINDOWS_SIGNING_PUBLISHER`. Without them, releases remain explicitly unsigned; the setup installer still supports ordinary per-user installation, while MSIX needs signing/trust for notification-enabled installation. No certificates are generated or trusted by the pipeline.

See [release operations](packaging/RELEASING.md). Linux application builds are deliberately not enabled yet: native overlay, audio/media and notification backends need Linux implementations before packaging or Linux CI would be meaningful. The Ubuntu workflow jobs only validate workflows and publish artifacts.


## Plugin examples

The remembered page carousel and Codex Usage 0.2 checks are documented in [page and Codex verification](PAGES-CODEX-VERIFICATION.md).

All runnable plugin samples live under [`examples/`](examples/): the developer UI showcase, Obsidian quick capture, GitHub Actions, and Codex usage. They use the native SDK in `crates/plugin-sdk`; the application host remains in `src/plugins`.

```powershell
cargo test --locked --workspace --exclude still-island
./scripts/package-plugins.ps1
```

The package script discovers `examples/*/manifest.json`, builds each sibling Cargo package, checks Windows executable imports, and writes separate plugin ZIPs and checksums to `target/plugin-packages`. New example crates are included through the workspace's `examples/*` member pattern.
