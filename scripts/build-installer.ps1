<#
.SYNOPSIS
Build the per-user Windows 11 installer with an existing Inno Setup 6 compiler.
.EXAMPLE
./scripts/build-installer.ps1 -Version 0.2.0
.EXAMPLE
./scripts/build-installer.ps1 -Executable ./target/msix-build/x86_64-pc-windows-msvc/release/still-island.exe -Version 0.2.0 -Compiler 'C:/Program Files (x86)/Inno Setup 6/ISCC.exe'
#>
[CmdletBinding()]
param(
    [string]$Executable,
    [Parameter(Mandatory = $true)]
    [string]$Version,
    [string]$OutputDirectory,
    [string]$Compiler
)
$ErrorActionPreference = 'Stop'
$workspace = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))

# Strict SemVer, including optional prerelease/build identifiers, is safe for both
# the Inno preprocessor and the resulting filename. PE resources use numeric core.
$versionPattern = '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-((?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*))*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$'
$match = [regex]::Match($Version, $versionPattern)
if (-not $match.Success) { throw 'Version must be SemVer, for example 0.2.0 or 0.2.0-rc.1.' }
$core = @($match.Groups[1].Value, $match.Groups[2].Value, $match.Groups[3].Value)
foreach ($part in $core) {
    $number = [uint16]0
    if (-not [uint16]::TryParse($part, [ref]$number)) { throw 'Numeric version components must be in 0..65535 for Windows PE resources.' }
}
$numericVersion = ($core -join '.') + '.0'
if (-not $Executable) { $Executable = Join-Path $workspace 'target/msix-build/x86_64-pc-windows-msvc/release/still-island.exe' }
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $workspace 'target/dist' }
$Executable = (Resolve-Path -LiteralPath $Executable -ErrorAction Stop).ProviderPath
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
$source = Join-Path $workspace 'packaging/windows/installer.iss'
foreach ($path in @($source, (Join-Path $workspace 'LICENSE'), (Join-Path $workspace 'README.md'), $Executable)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Required input file not found: $path" }
    if ($path.Contains('"') -or $path.Contains("`r") -or $path.Contains("`n")) { throw 'Input paths cannot contain quotes or newlines.' }
}
if ($OutputDirectory.Contains('"') -or $OutputDirectory.Contains("`r") -or $OutputDirectory.Contains("`n")) { throw 'Output path cannot contain quotes or newlines.' }

function Assert-StaticX64Executable([string]$Path) {
    $bytes = [IO.File]::ReadAllBytes($Path)
    if ($bytes.Length -lt 64 -or $bytes[0] -ne 0x4d -or $bytes[1] -ne 0x5a) { throw 'Executable is not a Windows PE image.' }
    $pe = [BitConverter]::ToUInt32($bytes, 60)
    if ($pe -gt $bytes.Length - 264 -or [BitConverter]::ToUInt32($bytes, $pe) -ne 0x4550) { throw 'Invalid PE header.' }
    if ([BitConverter]::ToUInt16($bytes, $pe + 4) -ne 0x8664) { throw 'Installer requires an x64 executable.' }
    $optional = $pe + 24
    if ([BitConverter]::ToUInt16($bytes, $optional) -ne 0x20b) { throw 'Executable is not PE32+.' }
    $sectionCount = [BitConverter]::ToUInt16($bytes, $pe + 6)
    $optionalSize = [BitConverter]::ToUInt16($bytes, $pe + 20)
    $sections = @(for ($i = 0; $i -lt $sectionCount; $i++) {
        $offset = $optional + $optionalSize + $i * 40
        if ($offset -gt $bytes.Length - 40) { throw 'Truncated PE section table.' }
        [pscustomobject]@{ Size = [BitConverter]::ToUInt32($bytes, $offset + 8); Rva = [BitConverter]::ToUInt32($bytes, $offset + 12); RawSize = [BitConverter]::ToUInt32($bytes, $offset + 16); Raw = [BitConverter]::ToUInt32($bytes, $offset + 20) }
    })
    $toOffset = { param([uint32]$Rva)
        foreach ($section in $sections) {
            if ($Rva -ge $section.Rva -and $Rva -lt $section.Rva + [math]::Max($section.Size, $section.RawSize)) { return [int64]($Rva - $section.Rva + $section.Raw) }
        }
        throw 'PE import RVA is outside the section table.'
    }
    $imports = [BitConverter]::ToUInt32($bytes, $optional + 120)
    if ($imports -eq 0) { return }
    $offset = & $toOffset $imports
    for ($i = 0; $i -lt 1024; $i++) {
        if ($offset -gt $bytes.Length - 20) { throw 'Truncated PE import table.' }
        $rva = [BitConverter]::ToUInt32($bytes, [int]($offset + 12))
        if ($rva -eq 0) { return }
        $start = & $toOffset $rva
        $end = $start
        while ($end -lt $bytes.Length -and $bytes[$end] -ne 0 -and $end - $start -lt 260) { $end++ }
        if ($end -ge $bytes.Length -or $end - $start -ge 260) { throw 'Invalid PE import name.' }
        $name = [Text.Encoding]::ASCII.GetString($bytes, [int]$start, [int]($end - $start))
        if ($name -match '^(?i:vcruntime|msvcp|msvcr)[0-9].*\.dll$') { throw "Executable depends on $name. Rebuild with RUSTFLAGS='-C target-feature=+crt-static'." }
        $offset += 20
    }
    throw 'PE import table has too many entries.'
}
Assert-StaticX64Executable $Executable

if ($Compiler) {
    $Compiler = (Resolve-Path -LiteralPath $Compiler -ErrorAction Stop).ProviderPath
} else {
    $command = Get-Command ISCC.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($command) { $Compiler = $command.Source }
    foreach ($programDirectory in @(${env:ProgramFiles(x86)}, $env:ProgramFiles)) {
        if (-not $Compiler -and $programDirectory) {
            $candidate = Join-Path $programDirectory 'Inno Setup 6/ISCC.exe'
            if (Test-Path -LiteralPath $candidate -PathType Leaf) { $Compiler = $candidate }
        }
    }
}
if (-not $Compiler -or -not (Test-Path -LiteralPath $Compiler -PathType Leaf)) {
    throw 'Inno Setup 6 ISCC.exe was not found. Install Inno Setup 6 or pass -Compiler with its existing path. This script does not download tools.'
}
# ISCC's official PE version resource can be 0.0.0.0. Check its own banner.
$banner=(& $Compiler '/?' 2>&1 | Out-String)
if($banner -notmatch 'Inno Setup 6 Command-Line Compiler'){throw 'Use the Inno Setup 6 compiler (ISCC.exe).'}

New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
$arguments = @('/Qp', "/DAppVersion=$Version", "/DNumericVersion=$numericVersion", "/DSourceExecutable=$Executable", "/DRepoRoot=$workspace", "/DOutputDirectory=$OutputDirectory", $source)
& $Compiler @arguments
if ($LASTEXITCODE -ne 0) { throw "Inno Setup compilation failed with exit code $LASTEXITCODE." }
$installer = Join-Path $OutputDirectory "StillIsland-$Version-windows-x64-setup.exe"
if (-not (Test-Path -LiteralPath $installer -PathType Leaf)) { throw "Compiler reported success without creating $installer" }
Get-Item -LiteralPath $installer
