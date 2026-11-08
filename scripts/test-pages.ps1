# Runs actual Slint page navigation and a packaged Codex plugin using only an
# offline Rust stdio fixture. Does not move the cursor or send desktop input.
param([string]$Binary='target\debug\still-island.exe',[string]$Package='')
$ErrorActionPreference='Stop'
$workspace=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if(Get-Process still-island -ErrorAction SilentlyContinue){throw 'Exit the existing island first; this test never stops another instance.'}
if(-not [IO.Path]::IsPathRooted($Binary)){$Binary=Join-Path $workspace $Binary}
$Binary=(Resolve-Path -LiteralPath $Binary).Path
if(-not $Package){$Package=(Get-ChildItem -LiteralPath (Join-Path $workspace 'target\plugin-packages') -Filter 'org.stillisland.codex-usage-*.zip' -File | Sort-Object LastWriteTime -Descending | Select-Object -First 1).FullName}
$Package=(Resolve-Path -LiteralPath $Package).Path
$run=Join-Path $workspace ('target\pages-probe\'+[guid]::NewGuid().ToString('N'))
$profile=Join-Path $run 'StillIsland'
New-Item -ItemType Directory -Force -Path $profile | Out-Null
$fixture=Join-Path $run 'codex-fixture.exe'
rustc --edition 2024 (Join-Path $workspace 'scripts\fixtures\codex-app-server.rs') -o $fixture
if($LASTEXITCODE -ne 0){throw 'Offline Codex fixture did not compile.'}
$mode=Join-Path $run 'mode.txt'
$methods=Join-Path $run 'methods.txt'
# UTF-8 without BOM: the Rust fixture deliberately accepts only its named modes.
[IO.File]::WriteAllText($mode,'normal',[Text.UTF8Encoding]::new($false))
@'
appearance=solid
form_factor=notch
hover_expand=true
compact_now_playing=false
auto_volume=false
notifications_enabled=false
battery_previews=false
online_lyrics=false
'@ | Set-Content -LiteralPath (Join-Path $profile 'settings.ini')
$trace=Join-Path $workspace 'verification.log'
$traceStart=if(Test-Path -LiteralPath $trace){@(Get-Content -LiteralPath $trace).Count}else{0}
$app=$null
try{
    $start=[Diagnostics.ProcessStartInfo]::new($Binary)
    $start.UseShellExecute=$false;$start.CreateNoWindow=$true;$start.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden
    $start.WorkingDirectory=$workspace
    $start.Arguments='--verify-plugins --verify-pages --verify-plugin-package="'+$Package+'" --verify-codex-executable="'+$fixture+'"'
    $start.EnvironmentVariables['LOCALAPPDATA']=$run
    $start.EnvironmentVariables['CODEX_HOME']=(Join-Path $run 'no-real-account')
    $start.EnvironmentVariables['ISLAND_CODEX_FIXTURE_MODE']=$mode
    $start.EnvironmentVariables['ISLAND_CODEX_FIXTURE_LOG']=$methods
    $app=[Diagnostics.Process]::Start($start)
    if(-not $app.WaitForExit(25000)){throw 'Finite page verification exceeded 25 seconds.'}
    if($app.ExitCode -ne 0){throw "Page verifier exited $($app.ExitCode)."}
    $lines=@(Get-Content -LiteralPath $trace | Select-Object -Skip $traceStart)
    $lines | Set-Content -LiteralPath (Join-Path $run 'verification.txt')
    $checks=@($lines | Where-Object {$_ -match '^pages (PASS|FAILED)'})
    $checks | Write-Output
    if($lines -match '^pages FAILED'){throw 'A page navigation or renderer assertion failed.'}
    if(-not ($lines -match '^pages verification complete$')){throw 'The app did not complete its page regression.'}
    if(@($checks | Where-Object {$_ -match '^pages PASS'}).Count -lt 25){throw 'Expected at least25 page/renderer assertions.'}
    if(-not (Test-Path -LiteralPath $methods)){throw 'No offline child was observed; rendered quota was not verified through IPC.'}
    $records=@(Get-Content -LiteralPath $methods)
    foreach($entry in $records){
        $parts=$entry -split ' ',2
        if($parts[1] -notin @('START','initialize','initialized','account/rateLimits/read')){throw 'Fixture received a method outside the read-only handshake.'}
    }
    foreach($entry in @($records | Where-Object {$_ -match ' START$'})){
        $fixtureId=[int]($entry -split ' ',2)[0]
        $child=Get-Process -Id $fixtureId -ErrorAction SilentlyContinue
        if($child){throw 'An owned offline fixture child remained alive after verification.'}
    }
    Write-Output ('PASS page carousel and offline Codex GUI; log '+$run+'; snapshots verification/pages-codex-{default,text140,compact}*.bmp')
}finally{
    if($app -and -not $app.HasExited){
        $null=$app.CloseMainWindow()
        if(-not $app.WaitForExit(2000)){Stop-Process -Id $app.Id}
    }
}
