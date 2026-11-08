# Dimensions, crisp rendering, tabs, and motion speed — 2026-11-08

Settings → Appearance now separates geometry from zoom:

| Setting | Default | Range |
| --- | --- | --- |
| Contracted width / height | 126×32 | 100–320 / 28–64 |
| Expanded width / height | 360×160 | 320–900 / 160–360 |
| Media width / height | 640×216 | 480–1000 / 216–400 |
| Island size (zoom) | 100% | 75–150% |
| Animation speed | 100% | 50–200% |

Dimensions are base layout units. Width/height changes preserve typography size; zoom changes typography and controls as well. Compact status cards retain a minimum 220×48 layout so their content remains usable. Monitor-fit constraints and overshoot room can reduce requested width. All settings persist locally with finite/range validation and backward-compatible defaults.

The blurry parent `transform-scale` has been removed. Layout lengths, fonts, icons, and control sizes now use their actual final sizes, including shared media/plugin controls. Windows DPI remains independent. Pointer/hit coordinates convert back to the same base layout space. The transparent native region, glass sampling, and persistent window use the dynamic canvas center and dimensions. Default dimensions retain the existing 800×300 minimum canvas; larger requested layouts allocate a larger surface only when needed. Shrinking waits for morph settlement, so the previous shape cannot be clipped mid-transition.

Animation speed scales elapsed simulation time for the existing exact spring integrator, preserving its trajectory and bounce characteristics. Lyric movement duration is 300ms divided by the selected speed. Media playback and countdown time remain real time. Animation-disable and reduced-motion behavior still apply.

Settings navigation uses Slint's native `TabWidget`/`Tab` with four pages. The former independently checkable buttons and private page switch have been removed. The selected tab survives preference-model refreshes.

## Verification

- Formatting, workspace checks, warning-denied Clippy, **146 workspace tests**, and release compilation passed. The live provider test remains opt-in.
- Release `--verify-layout --appearance=solid` passed custom contracted 220×52, expanded 600×240, and media 900×300 layouts, monitor fit, independent text sizing, explicit font-size readback at 150% zoom, 50%/200% speed bindings, all four native tabs, unchanged preferences on disk, and stopped animation scheduling.
- Pure tests cover size persistence/clamps, dynamic canvas/overshoot room, time-scaled spring trajectories, native DPI/canvas dimensions, custom-canvas pointer alignment, and glass-cache/sample alignment.
- The release also passed `--verify-size` at 75/100/125/150% and `--verify-lyrics`, including interrupted movement, pause/seek, transparent edge routing, and no further renderer frames after settling.
- Renderer captures were reviewed: `verification/layout-media-150.bmp`, `layout-settings-tab0.png`, `layout-settings-tab2.png`, plus all native tab pages and size captures. Original synthetic content was used; the layout and lyric fixtures do not control a real media player or request online lyrics.

Physical checks used the primary 1920×1080 display at 96 DPI. Additional physical DPI scales, exhaustive keyboard/pointer interaction, and glass capture at every custom dimension were not tested. Resource usage was not rebenchmarked after this rendering change; the prior RAM measurements remain in [OPTIMIZATION-VERIFICATION.md](OPTIMIZATION-VERIFICATION.md).

```powershell
cargo test --workspace --locked --quiet
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --release --locked
.\target\release\still-island.exe --verify-layout --appearance=solid
.\target\release\still-island.exe --verify-size --appearance=solid
.\target\release\still-island.exe --verify-lyrics --appearance=solid
```
