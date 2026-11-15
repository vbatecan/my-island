$ErrorActionPreference='Stop'
$workspace=Split-Path $PSScriptRoot
$fixture=Join-Path $workspace ('target/update-fixture-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $fixture | Out-Null
$savedLocal=$env:LOCALAPPDATA
try {
    $env:LOCALAPPDATA=$fixture
    $version='99.0.0'
    Set-Content -LiteralPath "$fixture/still-island.exe" -Value 'fixture executable'
    Compress-Archive -LiteralPath "$fixture/still-island.exe" -DestinationPath "$fixture/portable.zip"
    $file=@{name="StillIsland-$version-windows-x64-portable.zip";bytes=(Get-Item "$fixture/portable.zip").Length;sha256=(Get-FileHash "$fixture/portable.zip").Hash}
    @{tag_name="v$version";draft=$false;prerelease=$false} | ConvertTo-Json | Set-Content "$fixture/latest.json"
    $manifest=@{version=$version;platform='windows';architecture='x64';signed=$false;files=@($file)}
    $manifest | ConvertTo-Json -Depth 5 | Set-Content "$fixture/manifest.json"
    function curl.exe {
        $destination=$args[[array]::IndexOf($args,'--output')+1]
        $url=$args[-1]
        $source=if($url.EndsWith('/latest')){'latest.json'}elseif($url.EndsWith('/release-manifest.json')){'manifest.json'}else{'portable.zip'}
        Copy-Item -LiteralPath "$fixture/$source" -Destination $destination
        $global:LASTEXITCODE=0
    }
    $work=& "$PSScriptRoot/app-update.ps1" -CurrentVersion '0.3.1' -Executable "$fixture/still-island.exe"
    if(!(Test-Path -LiteralPath "$work/extracted/still-island.exe") -or !(Test-Path -LiteralPath "$work/ready.json")){throw 'Valid portable update was not prepared'}
    $current=& "$PSScriptRoot/app-update.ps1" -CurrentVersion $version -Executable "$fixture/still-island.exe"
    if($current -ne 'Current'){throw 'Equal version was not ignored'}
    $file.sha256='0'*64
    $manifest | ConvertTo-Json -Depth 5 | Set-Content "$fixture/manifest.json"
    & "$PSScriptRoot/app-update.ps1" -CurrentVersion '0.3.1' -Executable "$fixture/still-island.exe" 2>$null
    if($LASTEXITCODE -ne 1){throw 'Corrupt checksum accepted'}
    @{tag_name='v100.0.0-rc1';draft=$false;prerelease=$true} | ConvertTo-Json | Set-Content "$fixture/latest.json"
    & "$PSScriptRoot/app-update.ps1" -CurrentVersion '0.3.1' -Executable "$fixture/still-island.exe" 2>$null
    if($LASTEXITCODE -ne 1){throw 'Prerelease accepted'}
    $errors=$null;$tokens=$null
    foreach($name in @('app-update.ps1','app-update-restart.ps1','prepare-auto-release.ps1')){
        [Management.Automation.Language.Parser]::ParseFile("$PSScriptRoot/$name",[ref]$tokens,[ref]$errors)|Out-Null
        if($errors.Count){throw "Parse failure: $name"}
    }
    # Exercise the actual restart helper against a disposable fixture executable.
    $restartWork=Join-Path $fixture 'restart'
    New-Item -ItemType Directory -Path "$restartWork/extracted" | Out-Null
    rustc "$PSScriptRoot/fixtures/update-app.rs" -o "$restartWork/extracted/still-island.exe"
    if($LASTEXITCODE){throw 'Fixture compilation failed'}
    $payload="$restartWork/extracted/still-island.exe"
    @{sha256=(Get-FileHash $payload).Hash;bytes=(Get-Item $payload).Length} | ConvertTo-Json | Set-Content "$restartWork/ready.json"
    $destination=Join-Path $fixture 'fixture-installed.exe'
    Set-Content -LiteralPath $destination -Value 'previous version'
    & "$PSScriptRoot/app-update-restart.ps1" -Work $restartWork -Executable $destination -ParentPid 2147483647
    for($attempt=0;$attempt -lt 50 -and !(Test-Path "$fixture/fixture-installed.started");$attempt++){Start-Sleep -Milliseconds 100}
    if(!(Test-Path "$fixture/fixture-installed.started") -or !(Test-Path "$destination.update-backup")){throw 'Portable replacement or relaunch failed'}
    'PASS: portable preparation, equal-version skip, checksum/prerelease rejection, helper syntax, isolated executable replacement/backup/relaunch; no user installation or desktop input.'
}finally{$env:LOCALAPPDATA=$savedLocal}
