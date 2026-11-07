param(
    [switch]$SkipBuild,
    [string]$Executable,
    [string]$OutputDirectory,
    [string]$Version='0.2.0.0',
    [string]$Publisher='CN=StillIslandDevelopment',
    [switch]$AllowDynamicCrt
)
$ErrorActionPreference='Stop'
$workspace=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if(-not $OutputDirectory){$OutputDirectory=Join-Path $workspace 'target\msix'}
$OutputDirectory=[IO.Path]::GetFullPath($OutputDirectory)
$parts=$Version.Split('.')
if($parts.Count -ne 4 -or @($parts|Where-Object {$_ -notmatch '^\d+$' -or [uint64]$_ -gt 65535}).Count){throw 'Version must contain four integers in 0..65535'}
if($Publisher -notmatch '^CN=' -or $Publisher.Contains([char]0)){throw 'Publisher must be a valid certificate subject beginning CN='}

function Find-MakeAppx {
    $found=Get-Command MakeAppx.exe -ErrorAction SilentlyContinue
    if($found){return $found.Source}
    $kits=Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    $versions=@(Get-ChildItem -LiteralPath $kits -Directory -ErrorAction SilentlyContinue|Where-Object Name -Match '^\d+\.\d+\.\d+\.\d+$'|Sort-Object {[version]$_.Name} -Descending)
    foreach($sdk in $versions){$tool=Join-Path $sdk.FullName 'x64\MakeAppx.exe';if(Test-Path -LiteralPath $tool){return $tool}}
    throw 'Windows SDK x64 MakeAppx.exe not found. Install the Windows SDK packaging tools.'
}
function Get-PeDependencies([string]$path) {
    $bytes=[IO.File]::ReadAllBytes($path)
    $pe=[BitConverter]::ToUInt32($bytes,60)
    if([BitConverter]::ToUInt32($bytes,$pe) -ne 0x4550){throw 'Executable is not a PE image'}
    if([BitConverter]::ToUInt16($bytes,$pe+4) -ne 0x8664){throw 'Only native Windows x64 executables can be packaged by this manifest'}
    $sectionCount=[BitConverter]::ToUInt16($bytes,$pe+6)
    $optionalSize=[BitConverter]::ToUInt16($bytes,$pe+20)
    $optional=$pe+24
    if([BitConverter]::ToUInt16($bytes,$optional) -ne 0x20b){throw 'Executable is not a PE32+ image'}
    $sections=@(for($i=0;$i -lt $sectionCount;$i++){
        $offset=$optional+$optionalSize+$i*40
        [pscustomobject]@{VirtualSize=[BitConverter]::ToUInt32($bytes,$offset+8);Rva=[BitConverter]::ToUInt32($bytes,$offset+12);RawSize=[BitConverter]::ToUInt32($bytes,$offset+16);Raw=[BitConverter]::ToUInt32($bytes,$offset+20)}
    })
    $rvaToOffset={param([uint32]$rva)
        foreach($section in $sections){if($rva -ge $section.Rva -and $rva -lt $section.Rva+[math]::Max($section.VirtualSize,$section.RawSize)){return [int64]($rva-$section.Rva+$section.Raw)}}
        return [int64]$rva
    }
    $imports=[BitConverter]::ToUInt32($bytes,$optional+120)
    if($imports -eq 0){return @()}
    $offset=& $rvaToOffset $imports
    $names=[Collections.Generic.List[string]]::new()
    for($i=0;$i -lt 1024;$i++){
        $nameRva=[BitConverter]::ToUInt32($bytes,[int]($offset+12))
        if($nameRva -eq 0){break}
        $nameOffset=& $rvaToOffset $nameRva
        $end=$nameOffset;while($end -lt $bytes.Length -and $bytes[$end] -ne 0){$end++}
        if($end-$nameOffset -gt 260){throw 'Invalid DLL import name'}
        $names.Add([Text.Encoding]::ASCII.GetString($bytes,[int]$nameOffset,[int]($end-$nameOffset)))
        $offset+=20
    }
    return @($names|Sort-Object -Unique)
}

function Set-StableZipTimestamps([string]$path) {
    # MakeAppx stamps ZIP headers with wall-clock time instead of source times.
    # Normalize only fixed-width timestamp fields, preserving compressed streams,
    # CRCs, block hashes, LFH sizes, and offsets. Signing happens later, manually.
    $bytes=[IO.File]::ReadAllBytes($path)
    $end=$bytes.Length-22
    while($end -ge [math]::Max(0,$bytes.Length-65557) -and [BitConverter]::ToUInt32($bytes,$end) -ne 0x06054b50){$end--}
    if($end -lt 0 -or [BitConverter]::ToUInt32($bytes,$end) -ne 0x06054b50){throw 'ZIP central directory not found'}
    $count=[BitConverter]::ToUInt16($bytes,$end+10)
    $directory=[BitConverter]::ToUInt32($bytes,$end+16)
    if($count -eq 65535 -or $directory -eq [uint32]::MaxValue){
        $locator=$end-20
        if([BitConverter]::ToUInt32($bytes,$locator) -ne 0x07064b50){throw 'ZIP64 end locator not found'}
        $zip64=[int][BitConverter]::ToUInt64($bytes,$locator+8)
        if([BitConverter]::ToUInt32($bytes,$zip64) -ne 0x06064b50){throw 'Invalid ZIP64 end record'}
        $count=[int][BitConverter]::ToUInt64($bytes,$zip64+32)
        $directory=[int64][BitConverter]::ToUInt64($bytes,$zip64+48)
    }
    $time=[BitConverter]::GetBytes([uint16]0)
    $date=[BitConverter]::GetBytes([uint16]((2026-1980)*512+1*32+1))
    $position=[int64]$directory
    for($index=0;$index -lt $count;$index++) {
        if([BitConverter]::ToUInt32($bytes,[int]$position) -ne 0x02014b50){throw 'Invalid ZIP central directory entry'}
        $local=[int64][BitConverter]::ToUInt32($bytes,[int]($position+42))
        if($local -eq [uint32]::MaxValue){
            $extra=$position+46+[BitConverter]::ToUInt16($bytes,[int]($position+28))
            $extraEnd=$extra+[BitConverter]::ToUInt16($bytes,[int]($position+30))
            $found=$false
            while($extra+4 -le $extraEnd){
                $tag=[BitConverter]::ToUInt16($bytes,[int]$extra);$length=[BitConverter]::ToUInt16($bytes,[int]($extra+2))
                if($tag -eq 1){
                    $data=$extra+4
                    if([BitConverter]::ToUInt32($bytes,[int]($position+24)) -eq [uint32]::MaxValue){$data+=8}
                    if([BitConverter]::ToUInt32($bytes,[int]($position+20)) -eq [uint32]::MaxValue){$data+=8}
                    $local=[int64][BitConverter]::ToUInt64($bytes,[int]$data);$found=$true;break
                }
                $extra+=4+$length
            }
            if(-not $found){throw 'ZIP64 local header offset missing'}
        }
        if([BitConverter]::ToUInt32($bytes,[int]$local) -ne 0x04034b50){throw 'Invalid ZIP local file header'}
        [Array]::Copy($time,0,$bytes,$position+12,2);[Array]::Copy($date,0,$bytes,$position+14,2)
        [Array]::Copy($time,0,$bytes,[int64]$local+10,2);[Array]::Copy($date,0,$bytes,[int64]$local+12,2)
        $position+=46+[BitConverter]::ToUInt16($bytes,[int]($position+28))+[BitConverter]::ToUInt16($bytes,[int]($position+30))+[BitConverter]::ToUInt16($bytes,[int]($position+32))
    }
    [IO.File]::WriteAllBytes($path,$bytes)
}

$makeAppx=Find-MakeAppx
if(-not $SkipBuild){
    if($Executable){throw 'Executable override requires -SkipBuild'}
    $oldFlags=$env:RUSTFLAGS
    try {
        # Keep the package self-contained without redistributing a VC runtime.
        # Separate target directory leaves normal portable release builds intact.
        $env:RUSTFLAGS=([string]$oldFlags+' -C target-feature=+crt-static').Trim()
        Push-Location $workspace
        try {& cargo build --release --locked --target x86_64-pc-windows-msvc --target-dir (Join-Path $workspace 'target\msix-build');if($LASTEXITCODE -ne 0){throw "Static-CRT release build failed: $LASTEXITCODE"}} finally {Pop-Location}
    } finally {$env:RUSTFLAGS=$oldFlags}
    $Executable=Join-Path $workspace 'target\msix-build\x86_64-pc-windows-msvc\release\still-island.exe'
}
if(-not $Executable){$Executable=Join-Path $workspace 'target\msix-build\x86_64-pc-windows-msvc\release\still-island.exe'}
$Executable=(Resolve-Path -LiteralPath $Executable).Path
$dependencies=@(Get-PeDependencies $Executable)
$dynamicCrt=@($dependencies|Where-Object {$_ -match '^(VCRUNTIME|MSVCP|CONCRT)\d+'})
if($dynamicCrt.Count -and -not $AllowDynamicCrt){throw "Executable needs app-local VC runtime DLLs ($($dynamicCrt -join ', ')). Use the default isolated static-CRT build. -AllowDynamicCrt is for temporary validation only."}
$forbidden=@($dependencies|Where-Object {$_ -match 'WebView2|Microsoft\.WindowsAppRuntime|Microsoft\.UI\.Xaml|Qt\d|libcef|electron|SDL|Tauri'})
if($forbidden.Count){throw "Unexpected external GUI runtime dependency: $($forbidden -join ', ')"}
$null=New-Item -ItemType Directory -Force $OutputDirectory
# A fresh stage avoids destructive cleanup and keeps previous artifacts reviewable.
$stage=Join-Path $OutputDirectory ('stage-'+[Guid]::NewGuid().ToString('N'))
$null=New-Item -ItemType Directory -Force (Join-Path $stage 'Assets')
Copy-Item -LiteralPath $Executable -Destination (Join-Path $stage 'still-island.exe')
foreach($asset in Get-ChildItem -LiteralPath (Join-Path $workspace 'packaging\Assets') -File|Sort-Object Name){Copy-Item -LiteralPath $asset.FullName -Destination (Join-Path $stage 'Assets')}
[xml]$manifest=Get-Content -LiteralPath (Join-Path $workspace 'packaging\AppxManifest.xml') -Raw
$manifest.Package.Identity.Version=$Version
$manifest.Package.Identity.Publisher=$Publisher
$manifest.Save((Join-Path $stage 'AppxManifest.xml'))
# Stable staged timestamps permit identical MSIX bytes from identical inputs.
$epoch=[DateTime]::SpecifyKind([DateTime]'2026-01-01T00:00:00',[DateTimeKind]::Utc)
foreach($file in Get-ChildItem -LiteralPath $stage -Recurse -File){$file.LastWriteTimeUtc=$epoch}
$package=Join-Path $OutputDirectory "StillIsland_$($Version)_x64_unsigned.msix"
& $makeAppx pack /d $stage /p $package /o
if($LASTEXITCODE -ne 0){throw "MakeAppx schema/content validation failed: $LASTEXITCODE"}
Set-StableZipTimestamps $package
$validation=Join-Path $OutputDirectory ('validation-'+[Guid]::NewGuid().ToString('N'))
& $makeAppx unpack /p $package /d $validation /o
if($LASTEXITCODE -ne 0){throw "Normalized MSIX content/block-map validation failed: $LASTEXITCODE"}
$report=[ordered]@{NormalizedZipTimestamps=$true;UnpackValidation=$validation;Package=$package;Version=$Version;Publisher=$Publisher;Architecture='x64';Unsigned=$true;Stage=$stage;ExecutableSha256=(Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash;PackageSha256=(Get-FileHash -LiteralPath $package -Algorithm SHA256).Hash;ImportedDlls=$dependencies;DynamicCrtImports=$dynamicCrt;ExternalGuiRuntimeImports=$forbidden;MakeAppx=$makeAppx;Installed=$false;ConsentRequested=$false}
$report|ConvertTo-Json -Depth 4|Set-Content -LiteralPath (Join-Path $OutputDirectory 'package-audit.json') -Encoding UTF8
Write-Output "Created unsigned package: $package"
Write-Output 'No package installation, certificate creation/import, signing, or permission request was performed.'
if($dynamicCrt.Count){Write-Warning 'VALIDATION ONLY: this package uses a dynamically linked CRT. Do not distribute it as the self-contained package.'}
