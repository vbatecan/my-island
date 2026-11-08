# Native Shell discovery/icons plus a launch of our own finite marker executable.
# Does not launch user applications, change Start Menu entries, or send input.
param([ValidateSet('debug','release')][string]$Configuration='release')
$ErrorActionPreference='Stop'
$workspace=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$dependencies=Join-Path $workspace "target\$Configuration\deps"
$windowsLibrary=Get-ChildItem -LiteralPath $dependencies -Filter 'libwindows-*.rlib' -File | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if(-not $windowsLibrary){throw "Build the $Configuration application first so its pinned windows crate is available."}
$run=Join-Path $workspace ('target\launcher-native\'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $run | Out-Null
$fixture=Join-Path $run 'owned-launch-fixture.exe'
$probe=Join-Path $run 'launcher-native-probe.exe'
& rustc --edition=2024 (Join-Path $PSScriptRoot 'fixtures\launcher-marker.rs') -o $fixture
if($LASTEXITCODE -ne 0){throw 'Could not build the owned launcher fixture.'}
$probeCodegen=if($Configuration -eq 'release'){@('-C','panic=abort','-C','opt-level=3','-C','lto=thin','-C','codegen-units=1')}else{@()}
& rustc --edition=2024 @probeCodegen (Join-Path $PSScriptRoot 'fixtures\launcher-catalog-probe.rs') -o $probe --extern "windows=$($windowsLibrary.FullName)" -L "dependency=$dependencies"
if($LASTEXITCODE -ne 0){throw 'Could not build the native Shell probe.'}
$start=[Diagnostics.ProcessStartInfo]::new($probe)
$start.Arguments='"'+$run+'"'
$start.UseShellExecute=$false
$start.CreateNoWindow=$true
$start.RedirectStandardOutput=$true
$start.RedirectStandardError=$true
$application=[Diagnostics.Process]::Start($start)
try{
    if(-not $application.WaitForExit(20000)){
        $application.Kill()
        throw 'Native Shell probe exceeded its finite deadline.'
    }
    $output=$application.StandardOutput.ReadToEnd()+$application.StandardError.ReadToEnd()
    $probeExit=$application.ExitCode
}finally{
    $application.Dispose()
}
$output | Set-Content -LiteralPath (Join-Path $run 'native-probe.log')
$output | Write-Output
if($probeExit -ne 0){throw "Native Shell probe exited $probeExit. See $run"}
$report=[ordered]@{
    run=$run
    gui_target=(Join-Path $run 'Owned GUI launch.lnk')
    gui_marker=(Join-Path $run 'gui-launch-result.txt')
    fixture=$fixture
}
$report | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $run 'fixture.json')
Write-Output "Native launcher evidence and reusable GUI fixture: $run"
