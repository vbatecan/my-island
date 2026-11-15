param([ValidateSet('Check','Apply')][string]$Operation='Check', [string]$CurrentVersion,
    [string]$Executable, [int]$ParentPid=0)
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$root=Join-Path $env:LOCALAPPDATA 'StillIsland/updates'
New-Item -ItemType Directory -Force -Path $root | Out-Null
function Fetch($url,$path,$limit) {
    & curl.exe --fail --silent --show-error --location --proto '=https' --proto-redir '=https' --connect-timeout 15 --max-time 300 --max-filesize $limit --user-agent 'StillIsland-Updater' --output $path $url
    if($LASTEXITCODE -ne 0){throw 'Download failed. Check your connection and try again.'}
}
function Verify($path,$file) {
    if(!(Test-Path -LiteralPath $path) -or (Get-Item -LiteralPath $path).Length -ne $file.bytes -or (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ine $file.sha256){throw 'Update verification failed. Check for updates again.'}
}
try {
    if($Operation -eq 'Check') {
        $work=Join-Path $root ([guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $work | Out-Null
        Fetch 'https://api.github.com/repos/vbatecan/my-island/releases/latest' "$work/release.json" 2097152
        $release=Get-Content "$work/release.json" -Raw | ConvertFrom-Json
        if($release.draft -or $release.prerelease -or $release.tag_name -notmatch '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'){throw 'No compatible update is available.'}
        $version=$release.tag_name.Substring(1)
        if([version]$version -le [version]$CurrentVersion){'Current';exit 0}
        $base="https://github.com/vbatecan/my-island/releases/download/v$version"
        Fetch "$base/release-manifest.json" "$work/manifest.json" 1048576
        $manifest=Get-Content "$work/manifest.json" -Raw | ConvertFrom-Json
        $existingSignature=Get-AuthenticodeSignature -LiteralPath $Executable
        if($existingSignature.Status -eq 'Valid' -and !$manifest.signed){throw 'A signed installation requires a signed update.'}
        if($manifest.version -cne $version -or $manifest.platform -ne 'windows' -or $manifest.architecture -ne 'x64'){throw 'This release is incompatible.'}
        $registration=Get-ItemProperty -LiteralPath 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{FA56F811-8F7A-4F1A-AD38-94DCB332EAF8}_is1' -ErrorAction SilentlyContinue
        $installed=$registration -and $registration.InstallLocation -and ([IO.Path]::GetFullPath($registration.InstallLocation).TrimEnd('\') -ieq [IO.Path]::GetFullPath((Split-Path $Executable)).TrimEnd('\'))
        $name=if($installed){"StillIsland-$version-windows-x64-setup.exe"}else{"StillIsland-$version-windows-x64-portable.zip"}
        $files=@($manifest.files | Where-Object name -CEQ $name)
        if($files.Count -ne 1 -or $files[0].sha256 -notmatch '^[0-9a-fA-F]{64}$' -or $files[0].bytes -le 0 -or $files[0].bytes -gt 268435456){throw 'Update metadata is invalid.'}
        $file=$files[0]
        Fetch "$base/$name" "$work/payload" $file.bytes
        Verify "$work/payload" $file
        if($manifest.signed){
            if(!$installed){
                Add-Type -AssemblyName System.IO.Compression.FileSystem
                $zip=[IO.Compression.ZipFile]::OpenRead("$work/payload")
                try{
                    $entries=@($zip.Entries | Where-Object FullName -CEQ 'still-island.exe')
                    if($entries.Count -ne 1 -or $entries[0].Length -gt 268435456){throw 'Invalid portable archive.'}
                    New-Item -ItemType Directory "$work/extracted" | Out-Null
                    [IO.Compression.ZipFileExtensions]::ExtractToFile($entries[0],"$work/extracted/still-island.exe")
                }finally{$zip.Dispose()}
                $signedPath="$work/extracted/still-island.exe"
            }else{$signedPath="$work/payload"}
            $old=Get-AuthenticodeSignature -LiteralPath $Executable
            $new=Get-AuthenticodeSignature -LiteralPath $signedPath
            if($new.Status -ne 'Valid' -or ($old.Status -eq 'Valid' -and $old.SignerCertificate.Subject -cne $new.SignerCertificate.Subject)){throw 'Update signature verification failed.'}
        }
        if($installed){Move-Item -LiteralPath "$work/payload" -Destination "$work/setup.exe"}
        else{
            if(!(Test-Path "$work/extracted")){
                Add-Type -AssemblyName System.IO.Compression.FileSystem
                # Extract only the executable; no archive-controlled paths are used.
                $zip=[IO.Compression.ZipFile]::OpenRead("$work/payload")
                try{
                    $entries=@($zip.Entries | Where-Object FullName -CEQ 'still-island.exe')
                    if($entries.Count -ne 1 -or $entries[0].Length -gt 268435456){throw 'Invalid portable archive.'}
                    New-Item -ItemType Directory "$work/extracted" | Out-Null
                    [IO.Compression.ZipFileExtensions]::ExtractToFile($entries[0],"$work/extracted/still-island.exe")
                }finally{$zip.Dispose()}
            }
        }
        $prepared=if($installed){"$work/setup.exe"}else{"$work/extracted/still-island.exe"}
        @{version=$version;executable=$Executable;sha256=(Get-FileHash -LiteralPath $prepared -Algorithm SHA256).Hash;bytes=(Get-Item -LiteralPath $prepared).Length} | ConvertTo-Json | Set-Content "$work/ready.json" -Encoding utf8
        # Return this request's directory; callers never share a mutable ready marker.
        $work
    }else{
        throw 'Apply requires the dedicated restart helper.'
    }
}catch{
    [Console]::Error.WriteLine('Could not prepare the update. Check your connection and try again.')
    exit 1
}
