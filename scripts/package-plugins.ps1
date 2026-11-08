param([switch]$SkipBuild, [switch]$KeepStaging)
$ErrorActionPreference = 'Stop'
$project = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
function Assert-PluginStaticCrt([string]$path) {
    $bytes = [IO.File]::ReadAllBytes($path)
    if ($bytes.Length -lt 64 -or [BitConverter]::ToUInt16($bytes, 0) -ne 0x5a4d) { throw 'Invalid plugin DOS header' }
    $pe = [BitConverter]::ToUInt32($bytes, 60)
    if ($pe -gt $bytes.Length - 264 -or [BitConverter]::ToUInt32($bytes, $pe) -ne 0x4550 -or [BitConverter]::ToUInt16($bytes, $pe + 4) -ne 0x8664) { throw 'Plugin must be a valid x64 PE' }
    $optional = $pe + 24
    if ([BitConverter]::ToUInt16($bytes, $optional) -ne 0x20b) { throw 'Plugin must be PE32+' }
    $sectionCount = [BitConverter]::ToUInt16($bytes, $pe + 6)
    $optionalSize = [BitConverter]::ToUInt16($bytes, $pe + 20)
    $sections = @(for ($i = 0; $i -lt $sectionCount; $i++) {
        $entry = $optional + $optionalSize + $i * 40
        if ($entry -gt $bytes.Length - 40) { throw 'Truncated plugin section table' }
        [pscustomobject]@{ Rva=[BitConverter]::ToUInt32($bytes,$entry+12); Size=[math]::Max([BitConverter]::ToUInt32($bytes,$entry+8),[BitConverter]::ToUInt32($bytes,$entry+16)); Raw=[BitConverter]::ToUInt32($bytes,$entry+20) }
    })
    $toOffset = { param([uint32]$rva)
        foreach ($section in $sections) { if ($rva -ge $section.Rva -and $rva -lt $section.Rva + $section.Size) { return [int64]($rva - $section.Rva + $section.Raw) } }
        throw 'Plugin import RVA outside sections'
    }
    $imports = [BitConverter]::ToUInt32($bytes, $optional + 120)
    if ($imports -eq 0) { return }
    $offset = & $toOffset $imports
    for ($i=0; $i -lt 1024; $i++) {
        if ($offset -gt $bytes.Length - 20) { throw 'Truncated plugin import descriptor' }
        $nameRva = [BitConverter]::ToUInt32($bytes, [int]($offset + 12))
        if ($nameRva -eq 0) { Write-Output "PASS static CRT: $([IO.Path]::GetFileName($path))"; return }
        $start = & $toOffset $nameRva
        $end=$start
        while ($end -lt $bytes.Length -and $bytes[$end] -ne 0 -and $end-$start -lt 260) { $end++ }
        if ($end -ge $bytes.Length -or $end-$start -ge 260) { throw 'Invalid plugin import name' }
        $name = [Text.Encoding]::ASCII.GetString($bytes,[int]$start,[int]($end-$start))
        if ($name -match '^(VCRUNTIME|MSVCP|CONCRT|MSVCR)') { throw "Plugin requires unsupported redistributable: $name" }
        $offset += 20
    }
    throw 'Plugin import table too large'
}
Push-Location $project
try {
    # Every packaged example is a separate Cargo binary with its own manifest.
    # Discovery also includes new examples without a second hard-coded build list.
    $examplesRoot = Join-Path $project 'examples'
    $examples = @(Get-ChildItem -LiteralPath $examplesRoot -Directory | Sort-Object Name | Where-Object {
        Test-Path -LiteralPath (Join-Path $_.FullName 'manifest.json') -PathType Leaf
    })
    if ($examples.Count -eq 0) { throw 'No plugin examples were found' }
    $packageArguments = @()
    foreach ($example in $examples) {
        $cargoManifest = Get-Content -LiteralPath (Join-Path $example.FullName 'Cargo.toml') -Raw
        $packageName = [regex]::Match($cargoManifest, '(?m)^name\s*=\s*"([a-zA-Z0-9_-]+)"\s*$')
        if (-not $packageName.Success) { throw "Missing Cargo package name in example $($example.Name)" }
        $packageArguments += @('-p', $packageName.Groups[1].Value)
    }
    $targetTriple = 'x86_64-pc-windows-msvc'
    $buildRoot = Join-Path $project 'target/plugin-build'
    if (-not $SkipBuild) {
        $previousFlags = $env:RUSTFLAGS
        $previousEncodedFlags = $env:CARGO_ENCODED_RUSTFLAGS
        try {
            $env:RUSTFLAGS = '-C target-feature=+crt-static'
            $env:CARGO_ENCODED_RUSTFLAGS = $null
            cargo build --release --locked --target $targetTriple --target-dir $buildRoot @packageArguments
            if ($LASTEXITCODE -ne 0) { throw 'Plugin build failed' }
        } finally {
            $env:RUSTFLAGS = $previousFlags
            $env:CARGO_ENCODED_RUSTFLAGS = $previousEncodedFlags
        }
    }
    $output = Join-Path $project 'target/plugin-packages'
    New-Item -ItemType Directory -Force -Path $output | Out-Null
    $archives = @()
    foreach ($example in $examples) {
        $manifestPath = Join-Path $example.FullName 'manifest.json'
        $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
        if ($manifest.id -notmatch '^[a-zA-Z0-9][a-zA-Z0-9._-]+$' -or $manifest.version -notmatch '^\d+\.\d+\.\d+$') { throw 'Invalid plugin identity/version' }
        if ($manifest.entrypoint -notmatch '^[a-zA-Z0-9][a-zA-Z0-9._-]*\.exe$') { throw 'Example entrypoint must be a Windows executable filename' }
        $executable = Join-Path $buildRoot "$targetTriple/release/$($manifest.entrypoint)"
        if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) { throw "Missing plugin executable: $executable" }
        Assert-PluginStaticCrt $executable
        $stage = Join-Path $output ([guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $stage | Out-Null
        try {
            Copy-Item -LiteralPath $manifestPath -Destination (Join-Path $stage 'manifest.json')
            Copy-Item -LiteralPath $executable -Destination (Join-Path $stage $manifest.entrypoint)
            $archive = Join-Path $output "$($manifest.id)-$($manifest.version)-windows-x64.zip"
            Compress-Archive -LiteralPath (Join-Path $stage 'manifest.json'),(Join-Path $stage $manifest.entrypoint) -DestinationPath $archive -Force
            $archives += $archive
            Write-Output $archive
        } finally {
            if (-not $KeepStaging) {
            $resolvedStage = (Resolve-Path -LiteralPath $stage).Path
            $resolvedOutput = (Resolve-Path -LiteralPath $output).Path
            if (-not $resolvedStage.StartsWith($resolvedOutput + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Stage path escaped plugin output directory' }
            # Only these two files are staged; never recursively delete a path.
            foreach ($stagedFile in @('manifest.json', $manifest.entrypoint)) {
                $stagedPath = Join-Path $resolvedStage $stagedFile
                if (Test-Path -LiteralPath $stagedPath -PathType Leaf) { Remove-Item -LiteralPath $stagedPath -Force }
            }
            Remove-Item -LiteralPath $resolvedStage
            }
        }
    }
    $checksums = $archives | ForEach-Object { "{0}  {1}" -f (Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash.ToLowerInvariant(),[IO.Path]::GetFileName($_) }
    $checksums | Set-Content -LiteralPath (Join-Path $output 'SHA256SUMS') -Encoding ascii
} finally { Pop-Location }
