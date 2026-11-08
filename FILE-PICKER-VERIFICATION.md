# Plugin package picker correction

The reported screenshot showed a blank white Shell content area and unreadable controls. The previous integration synchronously called `IFileOpenDialog::Show(None)` inside the Slint callback, ignored COM apartment initialization failures, and treated every Show failure as Cancel.

The picker now runs on a short-lived STA thread with balanced COM initialization. The existing Integrations HWND owns the modal dialog; the Slint event loop remains free. Repeated Browse clicks are disabled while it is active. The dialog has a plugin ZIP filter, requires an existing filesystem file, and does not change the working directory or add the package to Windows recent files. Cancel preserves the previous path. Other errors are displayed instead of silently disappearing. Selecting a path never imports or enables a plugin.

Native code is isolated in `src/platform/file_dialog.rs`. No UI framework, file-dialog dependency, undocumented theme hook, or global dark-mode override was added. The Windows Shell Common binding feature supplies the filter structure.

## Evidence

- `cargo check --workspace --locked`: passed.
- Workspace tests: 166 passed; one optional live lyrics-provider test ignored. The new unit test distinguishes cancellation from access/COM errors.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- Debug and release builds passed. The release build was followed by `scripts/test-plugin-picker.ps1 -Binary target/release/still-island.exe`; all six native checks passed again.
- Integrated native picker test passed six checks: Cancel retains the previous package path, cancellation is distinct from errors, Slint dispatches timer callbacks while each of two dialogs is open, the exact selected ZIP returns to the manager, and no plugin was imported/enabled.
- The actual in-app picker screenshot was inspected: the generated package list, filename entry, ZIP filter, Select, and Cancel are readable with the system's dark appearance. The user also confirmed ZIP selection works.

Standalone plain-STA, MTA-conflict, and Common Controls v6 comparison dialogs all rendered normally. Therefore a missing v6 manifest was **not established as the cause**, and none was added speculatively. The exact original paint failure was not reproduced in that isolated comparison. The evidence supports the corrected in-app integration; it does not identify a unique Windows theming defect.

`scripts/test-plugin-picker.ps1` starts an isolated instance, navigates only to generated package files, screenshots only its owned picker, drives Cancel/Select, and requires the six checks and clean exit. It uses a scoped native control fallback when UI Automation does not expose Value/Invoke patterns. Existing user dialogs and installed plugins are not test inputs. Diagnostic images and logs live under ignored `target/plugin-picker-probe/`; the final release evidence is in run `b916383eaab1440faaf1efb31ea98f15`. Both final screenshots show generated package files. The normal release application was relaunched after verification.

Microsoft documents [STA initialization for common dialogs](https://learn.microsoft.com/en-us/windows/win32/shell/common-file-dialog) and [the owner parameter and cancellation result](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-imodalwindow-show). Desktop automation access was unavailable in this session, so the controlled native regression and screenshot fixture supplied runtime evidence.
