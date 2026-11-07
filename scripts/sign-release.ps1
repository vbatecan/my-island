[CmdletBinding()]
param(
    [string[]]$Paths = @(),
    [string]$ExpectedPublisher,
    [string]$OutputFile,
    [switch]$RequireSignature
)
$ErrorActionPreference = 'Stop'

function Write-SigningResult([bool]$Enabled, [int]$Count) {
    $value = if ($Enabled) { 'true' } else { 'false' }
    if ($OutputFile) {
        $line = 'signing_enabled={0}{1}' -f $value, [Environment]::NewLine
        [IO.File]::AppendAllText([IO.Path]::GetFullPath($OutputFile), $line, [Text.UTF8Encoding]::new($false))
    }
    [pscustomobject][ordered]@{
        signing_enabled = $Enabled
        signed = ($Enabled -and $Count -gt 0)
        signed_count = $Count
    }
}
function Find-SignTool {
    $command = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if ($command) { return $command.Source }
    $kits = Join-Path ([Environment]::GetFolderPath('ProgramFilesX86')) 'Windows Kits\10\bin'
    $versions = @(Get-ChildItem -LiteralPath $kits -Directory -ErrorAction SilentlyContinue |
        Where-Object Name -Match '^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$' |
        Sort-Object { [version]$_.Name } -Descending)
    foreach ($version in $versions) {
        $candidate = Join-Path $version.FullName 'x64\signtool.exe'
        if (Test-Path -LiteralPath $candidate) { return $candidate }
    }
    throw 'Windows SDK x64 SignTool.exe was not found.'
}
function ConvertTo-WindowsArgument([string]$Value) {
    # PowerShell 5's native argument binder loses some embedded quotes. This is
    # the Windows CommandLineToArgvW/MSVCRT quoting rule for that fallback only.
    $builder = [Text.StringBuilder]::new()
    [void]$builder.Append([char]34)
    $slashes = 0
    foreach ($character in $Value.ToCharArray()) {
        if ($character -eq [char]92) { $slashes++; continue }
        if ($character -eq [char]34) {
            [void]$builder.Append([char]92, (2 * $slashes + 1))
            [void]$builder.Append([char]34)
        } else {
            [void]$builder.Append([char]92, $slashes)
            [void]$builder.Append($character)
        }
        $slashes = 0
    }
    [void]$builder.Append([char]92, (2 * $slashes))
    [void]$builder.Append([char]34)
    $builder.ToString()
}
function Invoke-QuietSignTool([string]$Tool, [string[]]$Arguments, [string]$Operation) {
    # Never print the command/arguments. SignTool's password exists briefly in
    # native process arguments: use isolated trusted release runners.
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $Tool
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    if ($info.PSObject.Properties['ArgumentList']) {
        foreach ($argument in $Arguments) { $info.ArgumentList.Add($argument) }
    } else {
        $info.Arguments = ($Arguments | ForEach-Object { ConvertTo-WindowsArgument $_ }) -join ' '
    }
    foreach ($key in @('WINDOWS_CERTIFICATE_PFX', 'WINDOWS_CERTIFICATE_PASSWORD', 'WINDOWS_SIGNING_PUBLISHER')) {
        $info.EnvironmentVariables.Remove($key)
    }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $info
    try {
        if (-not $process.Start()) { throw "SignTool $Operation could not start." }
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(60000)) {
            $process.Kill()
            throw "SignTool $Operation exceeded its 60-second limit."
        }
        $code = $process.ExitCode
        $diagnostic = $stdout.GetAwaiter().GetResult() + [Environment]::NewLine + $stderr.GetAwaiter().GetResult()
    } finally { $process.Dispose() }
    if ($code -ne 0) {
        foreach ($secret in @($env:WINDOWS_CERTIFICATE_PFX, $env:WINDOWS_CERTIFICATE_PASSWORD, $env:WINDOWS_SIGNING_PUBLISHER)) {
            if (-not [string]::IsNullOrEmpty($secret)) { $diagnostic = $diagnostic.Replace($secret, '[REDACTED]') }
        }
        if ($diagnostic.Length -gt 3000) { $diagnostic = $diagnostic.Substring(0, 3000) }
        throw "SignTool $Operation failed (exit $code). $diagnostic"
    }
}

$targets = @()
foreach ($path in $Paths) {
    $item = Get-Item -LiteralPath $path
    if ($item.PSIsContainer) { throw 'Signing targets must be files.' }
    if ($item.Extension.ToLowerInvariant() -notin @('.exe', '.dll', '.msix', '.appx', '.msixbundle', '.appxbundle', '.msi', '.msp', '.cab')) {
        throw 'A signing target has an unsupported Windows artifact extension.'
    }
    $targets += $item.FullName
}
$present = @(
    -not [string]::IsNullOrEmpty($env:WINDOWS_CERTIFICATE_PFX)
    -not [string]::IsNullOrEmpty($env:WINDOWS_CERTIFICATE_PASSWORD)
    -not [string]::IsNullOrWhiteSpace($env:WINDOWS_SIGNING_PUBLISHER)
)
$configured = @($present | Where-Object { $_ }).Count
if ($configured -eq 0) {
    if ($RequireSignature) { throw 'Release signing is required, but its three signing secrets are not configured.' }
    Write-SigningResult $false 0
    return
}
if ($configured -ne 3) { throw 'Signing configuration is incomplete: configure all three Windows signing secrets, or none.' }
if ($targets.Count -eq 0) { throw 'At least one existing artifact path is required when signing is configured.' }
if ([string]::IsNullOrWhiteSpace($ExpectedPublisher)) { $ExpectedPublisher = $env:WINDOWS_SIGNING_PUBLISHER }
if ($ExpectedPublisher -cne $env:WINDOWS_SIGNING_PUBLISHER) { throw 'ExpectedPublisher differs from the configured Windows signing publisher.' }

$certificate = $null
$pfxBytes = $null
$temporaryPfx = $null
try {
    try { $pfxBytes = [Convert]::FromBase64String($env:WINDOWS_CERTIFICATE_PFX) }
    catch { throw 'WINDOWS_CERTIFICATE_PFX must contain valid base64 PFX data.' }
    try {
        # EphemeralKeySet loads an in-memory certificate/key without an X509Store
        # import, persisted private key, or trust-store changes.
        $certificate = [Security.Cryptography.X509Certificates.X509Certificate2]::new(
            $pfxBytes, $env:WINDOWS_CERTIFICATE_PASSWORD,
            [Security.Cryptography.X509Certificates.X509KeyStorageFlags]::EphemeralKeySet
        )
    } catch { throw 'The PFX could not be opened with the supplied signing password.' }
    if (-not $certificate.HasPrivateKey) { throw 'The signing PFX does not contain a private key.' }
    if ($certificate.Subject -cne $ExpectedPublisher) { throw 'The certificate Subject does not exactly match the expected MSIX Publisher.' }
    $now = [DateTime]::UtcNow
    if ($certificate.NotBefore.ToUniversalTime() -gt $now -or $certificate.NotAfter.ToUniversalTime() -le $now) { throw 'The signing certificate is not currently valid.' }
    $eku = @($certificate.Extensions | Where-Object { $_.Oid.Value -eq '2.5.29.37' })
    if ($eku.Count -ne 1) { throw 'The signing certificate must explicitly declare the Code Signing EKU.' }
    $usage = [Security.Cryptography.X509Certificates.X509EnhancedKeyUsageExtension]::new($eku[0], $eku[0].Critical)
    if (-not @($usage.EnhancedKeyUsages | Where-Object Value -EQ '1.3.6.1.5.5.7.3.3').Count) { throw 'The signing certificate does not allow Code Signing.' }
    $tool = Find-SignTool
    $temporaryPfx = Join-Path ([IO.Path]::GetTempPath()) ('still-island-sign-' + [Guid]::NewGuid().ToString('N') + '.pfx')
    [IO.File]::WriteAllBytes($temporaryPfx, $pfxBytes)
    foreach ($target in $targets) {
        Invoke-QuietSignTool $tool @(
            'sign', '/f', $temporaryPfx, '/p', $env:WINDOWS_CERTIFICATE_PASSWORD,
            '/fd', 'SHA256', '/tr', 'http://timestamp.digicert.com', '/td', 'SHA256', $target
        ) 'signing'
        Invoke-QuietSignTool $tool @('verify', '/pa', '/all', '/tw', $target) 'verification'
        $signature = Get-AuthenticodeSignature -LiteralPath $target
        if ($signature.Status -ne 'Valid' -or $null -eq $signature.SignerCertificate -or
            $signature.SignerCertificate.Thumbprint -cne $certificate.Thumbprint -or
            $null -eq $signature.TimeStamperCertificate) {
            throw 'The artifact did not retain a valid signature from the expected certificate with a timestamp.'
        }
    }
    Write-SigningResult $true $targets.Count
} finally {
    if ($certificate) { $certificate.Dispose() }
    if ($pfxBytes) { [Array]::Clear($pfxBytes, 0, $pfxBytes.Length) }
    if ($temporaryPfx -and (Test-Path -LiteralPath $temporaryPfx)) {
        # Remove only the exact temporary file, never a directory or store entry.
        Remove-Item -LiteralPath $temporaryPfx -Force -ErrorAction Stop
    }
}
