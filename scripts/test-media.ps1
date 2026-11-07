param()
$ErrorActionPreference='Stop'
$probeRoot=Join-Path $env:TEMP 'still-island-media-probe'
New-Item -ItemType Directory -Force -Path $probeRoot|Out-Null
$probeSource=(Resolve-Path "$PSScriptRoot\media-integration-probe.rs").Path.Replace('\','/')
$manifest=@"
[package]
name="still-island-media-probe"
version="0.1.0"
edition="2024"
[[bin]]
name="media-probe"
path="$probeSource"
[dependencies]
windows={version="=0.62.2",features=["Foundation","Media","Media_Control","Storage_Streams","Win32_Foundation","Win32_System_WinRT","Win32_UI_WindowsAndMessaging","Win32_UI_Shell"]}
"@
$manifest|Set-Content -LiteralPath (Join-Path $probeRoot 'Cargo.toml')
cargo run --release --manifest-path (Join-Path $probeRoot 'Cargo.toml') --bin media-probe
if($LASTEXITCODE -ne 0){throw 'Media integration probe failed'}
