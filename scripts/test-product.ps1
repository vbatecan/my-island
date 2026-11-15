param(
    [string]$Executable='target/debug/still-island.exe',
    [switch]$VerifyHandoffs,
    [string]$FileSearchPath,
    [switch]$VerifyFileSearchActions,
    [switch]$VerifyLauncherPrefixes
)
$ErrorActionPreference='Stop'

if (($VerifyLauncherPrefixes -and ($VerifyHandoffs -or $FileSearchPath)) -or
    ($VerifyHandoffs -and $FileSearchPath) -or ($VerifyFileSearchActions -and -not $FileSearchPath)) {
    throw 'Choose one product fixture and provide FileSearchPath with VerifyFileSearchActions.'
}

function Test-ExplorerFolder($Shell, [string]$ExpectedPath) {
    try {
        $windows = $Shell.Windows()
    } catch {
        return $false
    }
    foreach ($window in $windows) {
        try {
            if ($window.Document.Folder.Self.Path -ieq $ExpectedPath) { return $true }
        } catch {
            # Explorer can be between documents during navigation.
        }
        try {
            [uri]$location = $null
            if ([uri]::TryCreate([string]$window.LocationURL, [UriKind]::Absolute, [ref]$location) -and
                $location.IsFile -and $location.LocalPath -ieq $ExpectedPath) {
                return $true
            }
        } catch {
            # Continue polling while the Explorer tab rebuilds.
        }
    }
    return $false
}

function Test-ExplorerSelection($Shell, [string]$ExpectedPath) {
    try {
        $windows = $Shell.Windows()
    } catch {
        return $false
    }
    $parent = Split-Path -LiteralPath $ExpectedPath
    foreach ($window in $windows) {
        try {
            if ($window.Document.Folder.Self.Path -ine $parent) { continue }
            foreach ($item in $window.Document.SelectedItems()) {
                if ($item.Path -ieq $ExpectedPath) { return $true }
            }
        } catch {
            # Explorer can be between documents during navigation.
        }
    }
    return $false
}

function Save-PlainClipboard {
    Add-Type -AssemblyName System.Windows.Forms
    if (-not ('StillIslandVerification.ClipboardSequence' -as [type])) {
        Add-Type -Namespace StillIslandVerification -Name ClipboardSequence -MemberDefinition @'
        [System.Runtime.InteropServices.DllImport("user32.dll")]
        public static extern uint GetClipboardSequenceNumber();
'@
    }
    $sequence = [StillIslandVerification.ClipboardSequence]::GetClipboardSequenceNumber()
    $data = [Windows.Forms.Clipboard]::GetDataObject()
    $formats = if ($data) { @($data.GetFormats($false)) }
    if ($formats | Where-Object { $_ -notin 'UnicodeText','Text','OEMText','System.String' }) {
        return
    }
    $text = [Windows.Forms.Clipboard]::GetText()
    if ($sequence -eq [StillIslandVerification.ClipboardSequence]::GetClipboardSequenceNumber()) {
        return [pscustomobject]@{ Sequence = $sequence; HasData = [bool]$formats; Text = $text }
    }
}

function Restore-FixtureClipboard($Snapshot, [string]$ExpectedPath) {
    $sequence = [StillIslandVerification.ClipboardSequence]::GetClipboardSequenceNumber()
    if ($sequence -eq $Snapshot.Sequence -or [Windows.Forms.Clipboard]::GetText() -ine $ExpectedPath) {
        return $false
    }
    if ($sequence -ne [StillIslandVerification.ClipboardSequence]::GetClipboardSequenceNumber()) {
        return $false
    }
    if ($Snapshot.HasData) {
        [Windows.Forms.Clipboard]::SetData([Windows.Forms.DataFormats]::UnicodeText, $Snapshot.Text)
    } else {
        [Windows.Forms.Clipboard]::Clear()
    }
    return $true
}

function Receive-BrowserHandoff([Net.Sockets.TcpListener]$Listener, [string]$RequestPath) {
    if (-not $Listener.Pending()) { return }
    $client = $Listener.AcceptTcpClient()
    try {
        # Browsers may open an idle connection before making the actual request.
        $client.ReceiveTimeout = 1000
        $client.SendTimeout = 1000
        $stream = $client.GetStream()
        $reader = [IO.StreamReader]::new($stream, [Text.Encoding]::ASCII)
        try {
            $request = $reader.ReadLine()
            if ($request -cnotmatch ('^GET ' + [regex]::Escape($RequestPath) + ' HTTP/1\.[01]$')) { return }
            $agent = ''
            $brands = ''
            for ($line = 0; $line -lt 80; $line++) {
                $header = $reader.ReadLine()
                if ([string]::IsNullOrEmpty($header)) { break }
                if ($header -match '^User-Agent:\s*(.*)') { $agent = $Matches[1] }
                if ($header -match '^Sec-CH-UA:\s*(.*)') { $brands = $Matches[1] }
            }
            $body = '<!doctype html><title>Still Island verification</title><p>Website shortcut opened.</p>'
            $response = [Text.Encoding]::ASCII.GetBytes(
                "HTTP/1.1 200 OK`r`nContent-Type: text/html; charset=utf-8`r`nContent-Length: $($body.Length)`r`nConnection: close`r`n`r`n$body"
            )
            $stream.Write($response, 0, $response.Length)
            switch -Regex ("$brands $agent") {
                'Microsoft Edge|\bEdg/' { return 'Microsoft Edge' }
                '\bFirefox/' { return 'Mozilla Firefox' }
                '\bOpera\b|\bOPR/' { return 'Opera' }
                '\bVivaldi\b' { return 'Vivaldi' }
                '\bBrave\b' { return 'Brave' }
                'Google Chrome' { return 'Google Chrome' }
                '\bChrom(?:e|ium)/|\bChromium\b' { return 'Chromium-based browser' }
                default { return 'browser brand unavailable' }
            }
        } catch [IO.IOException] {
            # A preconnection may expire without sending any request.
        } finally {
            $reader.Dispose()
        }
    } finally {
        $client.Dispose()
    }
}

$binary=(Resolve-Path -LiteralPath $Executable).Path
$workspace=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$run=Join-Path $workspace ('target/product-verification/'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $run -Force | Out-Null
$start=[Diagnostics.ProcessStartInfo]::new($binary)
$start.UseShellExecute=$false
$start.CreateNoWindow=$true
$start.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden
$start.WorkingDirectory=$run
$start.Arguments='--verify-product --isolated-material-probe --appearance=solid'
$start.EnvironmentVariables['LOCALAPPDATA']=Join-Path $run 'profile'
$application=$null
$listener=$null
$shell=$null
$clipboard=$null
$copied=$false
$logPath=Join-Path $run 'verification.log'
try {
    if ($VerifyLauncherPrefixes) {
        $fixtureSettings = Join-Path $start.EnvironmentVariables['LOCALAPPDATA'] 'StillIsland'
        [void][IO.Directory]::CreateDirectory($fixtureSettings)
        Set-Content -LiteralPath (Join-Path $fixtureSettings 'settings.ini') -Value @('animations=false','text_scale=1.4')
        $start.EnvironmentVariables['STILL_ISLAND_VERIFY_PREFIX_MARKER'] = Join-Path $run 'prefix-command-marker.txt'
    }
    if ($VerifyHandoffs) {
        $folder = Join-Path $run 'Alias folder with spaces'
        [void][IO.Directory]::CreateDirectory($folder)
        $fixtureSettings = Join-Path $start.EnvironmentVariables['LOCALAPPDATA'] 'StillIsland'
        [void][IO.Directory]::CreateDirectory($fixtureSettings)
        Set-Content -LiteralPath (Join-Path $fixtureSettings 'settings.ini') -Value 'animations=false'
        $requestPath = '/' + [guid]::NewGuid().ToString('N')
        $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
        $listener.Start()
        $url = "http://127.0.0.1:$($listener.LocalEndpoint.Port)$requestPath"
        $start.EnvironmentVariables['STILL_ISLAND_VERIFY_ALIAS_FOLDER'] = $folder
        $start.EnvironmentVariables['STILL_ISLAND_VERIFY_ALIAS_URL'] = $url
        $defaultBrowser = try {
            Get-ItemPropertyValue -LiteralPath 'HKCU:\Software\Microsoft\Windows\Shell\Associations\UrlAssociations\http\UserChoice' -Name 'ProgId'
        } catch { 'unavailable' }
        $shell = try { New-Object -ComObject Shell.Application } catch { $null }
    }
    if ($FileSearchPath) {
        $FileSearchPath = (Resolve-Path -LiteralPath $FileSearchPath).Path
        $fixtureSettings = Join-Path $start.EnvironmentVariables['LOCALAPPDATA'] 'StillIsland'
        [void][IO.Directory]::CreateDirectory($fixtureSettings)
        Set-Content -LiteralPath (Join-Path $fixtureSettings 'settings.ini') -Value @('animations=false','text_scale=1.4')
        $start.EnvironmentVariables['STILL_ISLAND_VERIFY_FILE_SEARCH_PATH'] = $FileSearchPath
        if ($VerifyFileSearchActions) {
            $shell = New-Object -ComObject Shell.Application
            $start.EnvironmentVariables['STILL_ISLAND_VERIFY_FILE_SEARCH_REVEAL'] = '1'
            $clipboard = try { Save-PlainClipboard } catch { $null }
            if ($clipboard) {
                $start.EnvironmentVariables['STILL_ISLAND_VERIFY_FILE_SEARCH_CLIPBOARD'] = [string]$clipboard.Sequence
            } else {
                Write-Output 'Copy path verification skipped because the current clipboard cannot be safely restored.'
            }
        }
    }
    $application=[Diagnostics.Process]::Start($start)
    $timeout = 40000
    if ($VerifyHandoffs) {
        $clock = [Diagnostics.Stopwatch]::StartNew()
        $explorerFound = $false
        $browser = $null
        while ((-not $explorerFound -or -not $browser) -and $clock.ElapsedMilliseconds -lt $timeout) {
            if ($application.HasExited -and $application.ExitCode -ne 0) { break }
            if ($shell -and -not $explorerFound) { $explorerFound = Test-ExplorerFolder $shell $folder }
            if (-not $browser) { $browser = Receive-BrowserHandoff $listener $requestPath }
            Start-Sleep -Milliseconds 150
        }
        $timeout = [Math]::Max(0, $timeout - $clock.ElapsedMilliseconds)
        $explorerStatus = if ($explorerFound) { 'PASS' } else { 'FAILED' }
        $browserStatus = if ($browser) { 'PASS' } else { 'FAILED' }
        if (-not $defaultBrowser) { $defaultBrowser = 'unavailable' }
        if (-not $browser) { $browser = 'no matching request received' }
        $handoffEvidence = @(
            "Explorer $explorerStatus exact folder $folder"
            "Browser $browserStatus exact GET $url ($browser)"
            "Default HTTP association ProgId $defaultBrowser"
        )
        $handoffEvidence | Set-Content -LiteralPath (Join-Path $run 'handoffs.txt')
        $handoffEvidence | Write-Output
    }
    if ($VerifyFileSearchActions) {
        $clock = [Diagnostics.Stopwatch]::StartNew()
        $explorerFound = $false
        while (-not $application.HasExited -or (-not $explorerFound -and $clock.ElapsedMilliseconds -lt $timeout)) {
            if ($clock.ElapsedMilliseconds -ge $timeout) { break }
            if ($application.HasExited -and (Test-Path -LiteralPath $logPath) -and
                (Select-String -LiteralPath $logPath -Pattern '^product FAILED' -Quiet)) { break }
            if (-not $explorerFound) { $explorerFound = Test-ExplorerSelection $shell $FileSearchPath }
            if ($clipboard -and -not $copied -and (Test-Path -LiteralPath $logPath) -and
                (Select-String -LiteralPath $logPath -Pattern '^product PASS Copy path activated' -Quiet)) {
                $copied = Restore-FixtureClipboard $clipboard $FileSearchPath
            }
            Start-Sleep -Milliseconds 120
        }
        $timeout = [Math]::Max(0, $timeout - $clock.ElapsedMilliseconds)
    }
    if(-not $application.WaitForExit([int]$timeout)){throw "Owned product fixture timed out. Evidence: $run"}
    $lines=Get-Content -LiteralPath $logPath
    $lines | Select-String 'product|panicked' | Write-Output
    if($application.ExitCode -ne 0 -or ($lines -match 'FAILED|panicked') -or -not ($lines -match 'product verification complete')){
        throw "Product fixture failed. Evidence: $run"
    }
    if ($VerifyHandoffs -and ($explorerStatus -ne 'PASS' -or $browserStatus -ne 'PASS')) {
        throw "Alias handoff verification failed. Evidence: $run"
    }
    if ($VerifyFileSearchActions) {
        if (-not $explorerFound) {
            throw "Explorer did not select the indexed fixture. Evidence: $run"
        }
        Write-Output 'Show in Explorer PASS selected the exact indexed fixture.'
        if ($copied) {
            Write-Output 'Copy path PASS copied the exact indexed fixture path and restored the clipboard.'
        } elseif ($clipboard -and -not ($lines -match '^product SKIP Copy path')) {
            throw "Copy path was not observed on the clipboard. Evidence: $run"
        }
    }
    Write-Output "Evidence: $run"
} finally {
    if ($listener) { $listener.Stop() }
    if ($shell) { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($shell) }
    if ($application) {
        if(-not $application.HasExited){$application.Kill();$application.WaitForExit()}
        $application.Dispose()
    }
    if ($clipboard -and -not $copied -and (Test-Path -LiteralPath $logPath) -and
        (Select-String -LiteralPath $logPath -Pattern '^product PASS Copy path activated' -Quiet)) {
        [void](Restore-FixtureClipboard $clipboard $FileSearchPath)
    }
}
