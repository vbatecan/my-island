# Still Island

A native Windows11 island written in Rust and Slint, with interruptible spring motion, real Windows volume and media controls, and selectable materials. No browser engine, network service, telemetry, or audio playback engine.


## Notification-first release

The next milestones are implemented: presentation arbitration, opt-in Windows notifications, automatic volume feedback, optional compact media, a real settings window, monitor selection, fullscreen hiding, and a global shortcut. See [MILESTONES-VERIFICATION.md](MILESTONES-VERIFICATION.md) for current evidence and remaining installation/device tests.

Open **Settings…** from the tray. Settings is a normal focusable window; the island remains nonactivating. Appearance, privacy/app filters, behavior, and display preferences save locally. Ctrl+Alt+Space toggles the panel by default; registration conflicts are reported and disable that shortcut. Outside clicks and Escape close an open island panel without swallowing the original input. Hover expansion is optional (off by default,350ms delay); Windows reduced-motion settings are respected unless overridden.

Notifications require the identity-bearing MSIX build, Windows consent, and the in-app opt-in. The portable build deliberately disables access-request controls and explains how to obtain identity. Package output and signing/install instructions are in [packaging/README.md](packaging/README.md). Build with `powershell -File scripts/package.ps1`; no certificate is trusted and no package is installed by the script.

After installing a properly signed/trusted package, exit any portable instance, launch Still Island, open Settings → Notifications, and choose Enable access. Compact previews show app/title; expand to read the body. Open app activates the registered source application, not necessarily the original message. Hide preview never clears Windows Notification Center. Notifications are opt-in; pause, per-app filtering, and identity-only privacy are available. Existing notifications are baselined without replay. Pending notifications expire after60s; dwell defaults to5s and pauses while hovered. At most20 notification identities are retained in the presentation queue, with same-app bursts grouped. Windows banners are unchanged.

User-opened panels are protected. External volume/mute changes show1.5s feedback, or a small chip inside an open panel; local slider writes do not create duplicate popups. Compact now-playing is optional and off by default. The coordinator restores the current media/idle base after temporary events. Source metadata remains authoritative; no playback engine or fake notification feed is used during normal operation.

Monitor selection falls back to primary if a saved display is absent. Fullscreen/lock suppression is event-driven and stops Live glass capture while hidden. Shell notification eligibility is checked before automatic previews; this is not a claim of complete modern Windows Do Not Disturb synchronization. No message bodies or notification history are persisted.

Development checks: `--verify-notifications` uses synthetic records and never asks for permission; `--verify-settings` captures the actual settings window and focus/style readback. Real notification listening is disabled in every verification/benchmark mode to prevent screenshots of private notifications. Renderer metrics are opt-in and measure pre-swap render callbacks, not displayed FPS.

## Build and run

Install stable Rust (MSVC) and Visual Studio Build Tools with Desktop development with C++ and the Windows SDK. Minimum Rust1.92; built here with1.99.1, Slint1.18.1, windows0.62.2.

```powershell
cargo run --release
# Optional state selection for development:
cargo run --release -- --demo --demo-state=media
# Temporary overrides; tray choices are persisted instead:
cargo run --release -- --appearance=glass --opacity=0.55 --bounce=lively
```

Executable: `target/release/still-island.exe`. One instance per Windows session. Click the capsule to open; click the black background or close button to collapse. Media and Volume tiles open their respective panels. The island uses a persistent transparent 420x220 logical canvas and a native rounded region for desktop click-through outside its silhouette.

## Customization

Open the tray icon, then Settings:

- **Material:** Solid black, Transparent, Acrylic blur, Eco liquid glass, Live liquid glass.
- **Transparency:** light25%, balanced55%, or dark80% black tint. Solid black stays opaque.
- **Spring response:** Soft, Balanced bounce, Lively bounce. Balanced is the new default; expansion has a visible but restrained overshoot, and collapse uses more damping to avoid excessive squashing. Retargeting preserves momentum.
- Animations, always-on-top, and top offset0/6/12 logical pixels.

Preferences are stored in `%LOCALAPPDATA%\StillIsland\settings.ini`. Numeric opacity can also be edited there between0.15 and0.95. The tray has Show Island, Reset Island, Settings, and Exit. `--demo` exposes state selection; it does not replace real services with mock data. Startup registration is omitted.

## Real controls

Volume controls the **default Windows multimedia output device**, including mute. Drag the slider, or click the speaker to mute/unmute. The compact state also offers volume. External changes and default-device replacement use Core Audio notifications, not polling. A missing output device is shown explicitly.

Media uses Windows **Global System Media Transport Controls**. Start playback in a player that publishes a Windows media session, then open Media. The panel displays actual track/artist/artwork and playback state; play/pause, previous, next, and seeking are sent to that publisher. Unsupported actions are disabled. No session shows an honest empty state. The application does not create playback or invent tracks. A four-times-per-second display clock runs only while the Media panel is open and the selected session is playing; it stops on pause/collapse. Provider timeline events remain authoritative. Seek is committed on release to avoid queuing remote commands during a drag.

Not every player exposes SMTC or every transport capability. If a player does not publish a session, this app cannot control it through GSMTC. No global keyboard-media-key fallback is used, because that could control a different application.

## Materials and glass mathematics

Transparent material uses the desktop composition surface directly. Acrylic requests the documented Windows11 `DWMSBT_TRANSIENTWINDOW` backdrop and adds the selected tint. Windows can choose a solid fallback under accessibility, transparency, power, or inactive-window policies. API acceptance is checked; unsupported acrylic falls back to transparency.

Both glass modes have actual **backdrop refraction**, rather than only a shiny border. The overlay is excluded from capture to prevent feedback. Pixels are filtered and refracted into a Slint bitmap; Slint continues to render the UI and spring morphs.

- **Eco glass** (`--appearance=glass`) samples the desktop through GDI on hover/click/state transitions and foreground/top-level-window changes. It has no capture worker or refresh timer. Its backdrop stays frozen between these events, so an underlying video does not update continuously.
- **Live glass** (`--appearance=live-glass`) uses borderless Windows Graphics Capture, a WARP Direct3D device, one buffer, dirty-region checks, and a30FPS backdrop ceiling. Only the canvas crop is read back. Significant pixel changes are coalesced; tiny compositor dithering is ignored. This30FPS ceiling applies to the backdrop, independently of spring animation scheduling.

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

Live glass is an optional higher-cost material. Its initial hardware-device implementation exceeded the original100MB ceiling; the optimized release sampled about80MiB private/95MiB working set on this machine, close to that ceiling. Its capture resources exist only while selected. Eco glass avoids that capture stack; unchanged sampled backdrops do not cause UI updates. The WinEvent hooks exclude the overlay's own process and ignore child-control movements, preventing refresh feedback. **The glass island is omitted from normal screen captures/recordings** because capture exclusion is required to prevent feedback. Internal `--verify` captures can still render it. Backdrop pixels stay in RAM; they are never sent over the network or written to disk during normal use. Explicit verification captures write local BMPs. If borderless capture is unsupported, live glass falls back to acrylic with an error message, without requesting a permission prompt or changing Windows security settings.

## Architecture and verification

`src/animation` holds exact damped-spring integration and choreography gates. `src/app.rs` separates logical destinations from current geometry and manages temporary animation/display timers. `ui/island.slint` contains the UI. `src/platform` owns Win32 styles, DPI/monitor events, tray, native regions, and optional capture. `src/services` owns event-driven Core Audio and GSMTC workers. All platform unsafe calls document their invariants. The two service workers sleep on channels when idle; no service polls for audio or track data.

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

Primary-monitor support is implemented; additional DPI scales and live monitor topology changes still require physical-device testing. No startup registration, media playback engine, cloud, or telemetry. Installed notification consent, additional physical DPI scales, and fullscreen/hotplug behavior require the device tests recorded in the current milestone report. The native acrylic material can be altered by Windows policies. Actual presentation FPS and exhaustive rounded-edge clicking have not been measured. Desktop UI automation is unavailable in this environment, so controlled manual pointer/tray tests are not claimed. Material captures and native hit-routing readback are verified separately.

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
