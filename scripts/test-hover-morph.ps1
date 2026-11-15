param(
    [string]$Executable = 'target/release/still-island.exe',
    [ValidateSet('gpu','cpu','visual','solid')][string]$Mode = 'gpu'
)
$ErrorActionPreference = 'Stop'
$binary = (Resolve-Path -LiteralPath $Executable).Path
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$run = Join-Path $workspace ('target/hover-performance/' + $Mode + '-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $run -Force | Out-Null
$start = [Diagnostics.ProcessStartInfo]::new($binary)
$start.UseShellExecute = $false
$start.CreateNoWindow = $true
$start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
$start.WorkingDirectory = $run
$start.Arguments = '--verify-hover-morph --isolated-material-probe'
if ($Mode -ne 'solid') { $start.Arguments += ' --appearance=acrylic' }
if ($Mode -in @('gpu','cpu')) { $start.Arguments += ' --perf-hover' }
if ($Mode -eq 'cpu') { $start.Arguments += ' --cpu-acrylic' }
$start.EnvironmentVariables['LOCALAPPDATA'] = Join-Path $run 'profile'
$application = [Diagnostics.Process]::Start($start)
try {
    if (-not $application.WaitForExit(20000)) { throw 'Owned hover fixture timed out.' }
    if ($application.ExitCode -ne 0) { throw "Fixture exited $($application.ExitCode)" }
    $lines = Get-Content -LiteralPath (Join-Path $run 'verification.log')
    $lines | Select-String 'FAILED|GPU rendered|render_metrics|complete' | Write-Output
    if (($lines -match 'FAILED|GPU setup failed|GPU rendering failed') -or -not ($lines -match 'hover-morph complete')) {
        throw "Hover fixture failed. Evidence: $run"
    }
    Write-Output "Evidence: $run"
} finally {
    if (-not $application.HasExited) { $application.Kill(); $application.WaitForExit() }
    $application.Dispose()
}
