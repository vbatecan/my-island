[CmdletBinding()]
param([string]$ReleaseTag)

$ErrorActionPreference = 'Stop'
$manifest = Join-Path $PSScriptRoot '../Cargo.toml'
$current = (& "$PSScriptRoot/release-version.ps1" -CargoToml $manifest).version

if ($ReleaseTag) {
    if ($ReleaseTag -cnotmatch '^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$') {
        throw 'Release tag must be a stable vX.Y.Z version.'
    }
    $version = $ReleaseTag.Substring(1)
    if ($version -ne $current) {
        $text = [IO.File]::ReadAllText($manifest)
        $versionLine = [regex]::new('(?m)^version = "' + [regex]::Escape($current) + '"')
        [IO.File]::WriteAllText($manifest, $versionLine.Replace($text, ('version = "' + $version + '"'), 1))
    }
    & "$PSScriptRoot/release-version.ps1" -Tag $ReleaseTag -CargoToml $manifest | Out-Null
    if ($version -ne $current) {
        cargo update --workspace --manifest-path $manifest
        if ($LASTEXITCODE) { throw 'Lockfile update failed' }
    }
    return
}

$tags = git tag --list 'v*'
if ($LASTEXITCODE) { throw 'Could not list release tags' }
$stable = @($tags | Where-Object { $_ -cmatch '^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$' } |
    ForEach-Object { [version]$_.Substring(1) } | Sort-Object)
$latest = if ($stable.Count) { $stable[-1] } else { [version]'0.0.0' }
$version = if ([version]$current -gt $latest) { $current } else { [version]::new($latest.Major, $latest.Minor, ($latest.Build + 1)) }
$tag = "v$version"
if ($env:GITHUB_OUTPUT) { "tag=$tag" >> $env:GITHUB_OUTPUT }
$tag
