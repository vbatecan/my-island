# Plugin examples

Every bundled plugin example lives in this folder. The host implementation remains in `src/plugins/`; the shared SDK and protocol remain in `crates/`.

| Folder | Integration |
| --- | --- |
| [codex-usage](codex-usage/README.md) | Reported Codex quota percentages, reset windows, and optional notch preview |
| developer | Native UI/protocol demonstration with inputs, progress, toggles and activities |
| obsidian | Daily-note and note-capture URI actions |
| github-actions | Read-only workflow status and optional background monitoring |

Build and package all examples from the repository root:

```powershell
powershell -File scripts/package-plugins.ps1
```

The packager discovers `examples/*/manifest.json`, builds their sibling Cargo packages, checks the Windows executables for static CRT linkage, and writes ZIPs plus `SHA256SUMS` into `target/plugin-packages/`. Import a ZIP through **Settings → Plugins**, then enable it. Example packages are separate from the island installer.

New Rust examples can be added as `examples/<name>/Cargo.toml`, `manifest.json`, and `src/main.rs`. Workspace membership and packaging are discovered automatically. See the [SDK guide](../crates/plugin-sdk/README.md) for the protocol and package contract.
