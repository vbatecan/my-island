# Launcher activation paint correction

The user confirmed the launcher surface no longer flashes white after the activation-paint changes.

- Disable DWM-managed transitions for the overlay HWND only. Slint spring animation remains enabled.
- Forward WM_NCACTIVATE to Winit with LPARAM(-1), preserving backend activation/focus handling while preventing DefWindowProc from painting native active/inactive frame colors over the frameless surface.
- Do not re-show an already-visible overlay when requesting keyboard focus.
- Consume WM_ERASEBKGND because Slint paints the full surface; continue forwarding WM_PAINT.
- Suppress the overlay's unused SC_KEYMENU and Alt+Space system-character path.
- Reconcile focus loss once after16ms against actual foreground ownership, guarded by an activation epoch so an old callback cannot close a new launcher session.

References: https://learn.microsoft.com/en-us/windows/win32/winmsg/wm-ncactivate and https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwmwindowattribute.

Type check, formatting and Clippy with warnings denied passed. Interactive visual confirmation came from the user. Earlier center-pixel probes were inconclusive: desktop interaction moved foreground to other applications and sometimes exposed their pixels. The probes were stopped and must not be treated as proof of all-frame correctness. A concurrent GUI verification run passed19checks but its2answer checks failed after the launcher closed during external interaction; full unattended GUI regression for this patch remains unverified. No global-input probes are left running.
