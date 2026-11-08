# Plugin platform v1 verification

Implemented a trusted executable plugin host, versioned JSON-lines contract, Rust SDK, native declarative layouts, focusable configuration/editor window, package management, protected credentials, quick-action pins, and coordinated activity previews. Reference packages are separate from the installer and disabled on import. No embedded runtime, browser UI, marketplace, or sandbox is provided.

## Checks performed on Windows

- Workspace formatting/type checks and warning-denied Clippy pass.
- 101 tests pass: 96 application tests, three protocol tests, one SDK error-acknowledgement test, and one GitHub repository-validation test.
- Actual compiled process fixture verifies handshake, views, action results, unsolicited URI rejection, activity throttling, graceful shutdown, descendant cleanup, crashes, malformed messages, hung-action timeout, restart, and direct removal of a running executable.
- Isolated package tests cover unsafe paths/versions/reserved namespaces, disabled import, protected credential storage, redaction boundaries, and configuration/pin lifecycle.
- Release `--verify-plugins --appearance=solid --verify-plugin-package=<developer ZIP>` passes disabled import, custom layout rendering, progress updates, editor input round-trip, manager capture, proactive-preview arbitration, disablement, and settled scheduling. Test storage is under ignored `target/plugin-ui-verification`, separate from the user's plugins.
- The exact static-CRT distribution executables pass protocol smoke checks. Obsidian URI encoding is verified without opening Obsidian or creating notes. A live public read-only GitHub workflow query succeeds; private-repository credentials and OAuth are not exercised.
- PE import audits confirm that all three example packages avoid VCRUNTIME/MSVCP/CONCRT/MSVCR redistributable dependencies. SHA256SUMS accompanies the ZIP files under `target/plugin-packages/`.
- Existing Windows CI checks the workspace; the additional plugin workflow builds, smoke-tests, and uploads separate plugin ZIP artifacts. Hosted execution of these newly edited workflows has not been triggered in this change.

## Release memory samples

These are single Windows process samples, not peak measurements or sustained idle/animation benchmarks. Private bytes use `Process.PrivateMemorySize64`; working set uses `Process.WorkingSet64`. MiB means 1,048,576 bytes.

| Scenario/process | Private bytes | Working set |
|---|---:|---:|
| Solid-material host during plugin GUI verification, approximately 3.5 seconds after launch | 90.39 MiB | 89.86 MiB |
| Developer plugin child in that run | 0.55 MiB | 4.03 MiB |
| Combined private bytes for that host and child | 90.95 MiB | — |
| Normal release with saved Live glass preferences, no plugins installed, three seconds after launch | 92.64 MiB | 94.95 MiB |

The scenarios use different materials/windows and should not be treated as a before/after comparison. They exceed the preferred idle/expanded memory goals and are close to the original 100 MB ceiling. The existing rendering/capture stack contributes to host usage; exact heap contributors have not been profiled. Arbitrary third-party plugins can add their own resource costs. No new CPU/GPU or long-duration memory benchmark is claimed.

## Remaining boundaries

Plugins have the user's Windows access. Import validation, process ownership, and protocol bounds are not a security sandbox. Real third-party application actions, private GitHub credentials, additional physical DPI scales, and manual native file-picker interaction remain user/device checks. Author/API documentation is in `crates/plugin-sdk/README.md`.

Automatic approval review rejected local staging-folder deletion with only “blocked by policy.” Packaging was completed through the non-deleting `-KeepStaging` option; ignored staging directories remain under `target/plugin-packages/`.
