[CmdletBinding()]
param(
    [string]$Tag,
    [string]$OutputFile,
    [string]$CargoToml
)
$ErrorActionPreference = 'Stop'
if (-not $CargoToml) { $CargoToml = Join-Path $PSScriptRoot '..\Cargo.toml' }

function Test-StableVersion([string]$Value, [string]$Description) {
    if ($Value -notmatch '\A(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\z') {
        throw "$Description must be a stable X.Y.Z version without leading zeros, prerelease, or build metadata."
    }
    foreach ($part in $Value.Split('.')) {
        [uint32]$number = 0
        if ($part.Length -gt 5 -or -not [uint32]::TryParse($part, [ref]$number) -or $number -gt 65535) {
            throw "$Description components must be in 0..65535 for MSIX."
        }
    }
}
$text = [IO.File]::ReadAllText((Resolve-Path -LiteralPath $CargoToml).Path)
$package = [regex]::Match($text, '(?ms)^\s*\[package\][ \t]*(?:#[^\r\n]*)?\r?\n(?<body>.*?)(?=^\s*\[|\z)')
if (-not $package.Success) { throw 'Cargo.toml has no [package] table.' }
$versionPattern = @'
(?m)^[ \t]*version[ \t]*=[ \t]*["'](?<version>[^"'\r\n]+)["'][ \t]*(?:#.*)?\r?$
'@
$versions = [regex]::Matches($package.Groups['body'].Value, $versionPattern)
if ($versions.Count -ne 1) { throw 'Cargo.toml must contain exactly one literal [package].version = "X.Y.Z".' }
$version = $versions[0].Groups['version'].Value
Test-StableVersion $version 'Cargo package version'
if ($Tag) {
    if ($Tag -notmatch '\Av(.+)\z') { throw 'Release tags must use the exact form vX.Y.Z.' }
    $tagVersion = $Matches[1]
    Test-StableVersion $tagVersion 'Release tag version'
    if ($tagVersion -cne $version) { throw 'Release tag does not match Cargo.toml [package].version.' }
}
$result = [pscustomobject][ordered]@{
    version = $version
    msix_version = "$version.0"
    tag = "v$version"
}
if ($OutputFile) {
    # Validated values cannot inject output lines. Use UTF-8 without a BOM even
    # when running on Windows PowerShell 5.
    $lines = 'version={0}{2}msix_version={1}{2}' -f $result.version, $result.msix_version, [Environment]::NewLine
    [IO.File]::AppendAllText([IO.Path]::GetFullPath($OutputFile), $lines, [Text.UTF8Encoding]::new($false))
}
$result
