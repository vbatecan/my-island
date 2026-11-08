# Contracted notch priority

Implemented order:

1. Notifications.
2. Urgent/temporary events: countdown completion, battery activity, explicit plugin activity previews.
3. Direct feedback: volume/mute.
4. Playing media (when compact now-playing is enabled).
5. Active countdown.
6. Remembered page.
7. Idle.

The presentation coordinator owns the order. Open or pinned panels retain user ownership. Privacy, notification enablement, suppression and fullscreen policy are preserved. Paused or unavailable media no longer counts as automatic playing-media activity. A specifically remembered Media page can still show its paused summary at the remembered-page tier.

Notifications immediately preempt volume feedback and keep their normal visible dwell; repeated volume changes cannot extend a notification indefinitely. Battery/plugin preview dwell can begin while lower-priority feedback exists. Events keep bounded existing lifetimes and are not replayed after becoming stale. No new polling or recurring timers were added.

Automatic activity never changes the saved page: for example, Codex → playing music → notification → music → pause → Codex. Hover or click still reopens the selected expanded page.

## Checks

- Workspace tests: 183 passed, one optional live lyrics-provider test ignored.
- Type check, Clippy with warnings denied, and formatting: passed.
- The priority regression asserts the complete ordered unwind, notification preemption/dwell, media-versus-countdown order, remembered-page preservation, and no remaining deadline after events clear.
- The page GUI regression additionally sends synthetic playing/paused changes through the normal `media-updated` callback with Codex selected. It uses an offline isolated profile and does not control a real media player.

- `cargo build --release --locked`: passed.
- All 38 release carousel/UI assertions passed, including playing media overriding Codex, pause restoring Codex, remembered page reopening, interrupted page switches, and stopped renderer/animation scheduling. Log: `target/pages-probe/fe7a1905b691482a90f7ea67cd99da57/verification.txt`.

No new memory/GPU benchmark, real notification-consent test, or physical fullscreen/display test was performed for this policy change.
