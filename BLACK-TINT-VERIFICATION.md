# Liquid Glass black tint

The Liquid Glass shader now applies the existing opacity control as a black overlay in display space, before bezel reflections. Refraction, blur, contour coverage, and edge highlights remain in the existing GPU pipeline. Text contrast is selected from the tinted backdrop, and a tint change immediately resets luminance filtering. The appearance preview uses the same shader.

Workspace tests passed (202 application tests, two ignored, plus workspace crate tests); Clippy and formatting passed. The added contrast regression covers changing from untinted white to a 55% black tint on the same frame.

The isolated `scripts/test-liquid-gpu.ps1 -Executable target/debug/still-island.exe` run passed through the actual renderer. At the existing default 55% opacity, interior samples were RGB 110/110/112 on a white backdrop, 9/11/16 on a dark backdrop, and 81/11/43 on a magenta backdrop. Snapshots of the battery and media states were visually inspected for tint, retained color, readable text, and contour edges.

Evidence: `target/liquid-gpu-verification/5fa9fb7fdd1843e3b64f3f5ae3b7ccee/`. The fixture used its own profile and mutex, did not replace the running application, and sent no global keyboard or mouse input. These checks establish fixture rendering, not compatibility with every live capture source or display. Partner Center upload remains separate from local packaging.

The final optimized executable extracted from `target/store/StillIsland_1.3.1.0_x64_store.msix` also passed the same renderer checks; evidence is `target/liquid-gpu-verification/200f8ce7cfb541af9e7c1ebdabdbff08/`. MakeAppx pack/unpack validation and explicit Store identity/version/SHA-256 checks passed. The package is unsigned for Microsoft Store submission, not installed or uploaded. Computer Use reports that its native pipe is unavailable.
