param(
    [switch]$Fixture,[string]$ControlDirectory,[switch]$SkipHover,[switch]$TraceHover,
    [ValidateSet('Auto','Hide','Show')][string]$FullscreenOverride='Auto',
    [string]$MonitorId='', [switch]$AllMonitors,
    [ValidateSet('debug','release')][string]$Configuration='release'
)
$ErrorActionPreference='Stop'
if($Fixture){
    Add-Type -AssemblyName System.Windows.Forms
    Add-Type -AssemblyName System.Drawing
    Add-Type -ReferencedAssemblies System.Windows.Forms,System.Drawing -TypeDefinition @'
using System;
using System.IO;
using System.Windows.Forms;
using System.Drawing;
using System.Runtime.InteropServices;
public static class IslandFullscreenFixture {
    [DllImport("user32.dll")] static extern bool SetProcessDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] static extern bool ShowWindow(IntPtr hwnd,int command);
    public static void Run(string directory,string monitorId) {
        SetProcessDpiAwarenessContext(new IntPtr(-4));
        Application.EnableVisualStyles();
        var selected=Screen.PrimaryScreen;
        if(!String.IsNullOrEmpty(monitorId)) {
            selected=Array.Find(Screen.AllScreens,s=>String.Equals(s.DeviceName,monitorId,StringComparison.OrdinalIgnoreCase));
            if(selected==null) throw new ArgumentException("Requested monitor is no longer connected: "+monitorId);
        }
        using(var form=new Form()) using(var timer=new Timer()) {
            form.Text="Still Island Fullscreen Test Fixture";
            form.BackColor=Color.FromArgb(32,38,48);
            form.StartPosition=FormStartPosition.Manual;
            // This is an isolated empty test surface. Keep it above other apps
            // so the parent can validate its hit target before clicking it.
            form.TopMost=true;
            var work=selected.WorkingArea;
            var normal=new Rectangle(work.Left+120,work.Top+160,Math.Min(640,work.Width-160),Math.Min(420,work.Height-200));
            form.Bounds=normal;
            string last="";
            timer.Interval=50;
            timer.Tick+=(sender,args)=>{
                string command="";
                try { command=File.ReadAllText(Path.Combine(directory,"command.txt")).Trim(); } catch(IOException) { return; }
                if(command==last || command.Length==0) return;
                if(command=="quit") { form.Close(); return; }
                form.TopMost=command!="beneath-notch";
                if(command=="fullscreen") {
                    form.WindowState=FormWindowState.Normal;
                    form.FormBorderStyle=FormBorderStyle.None;
                    form.Bounds=selected.Bounds;
                } else if(command=="maximized") {
                    form.WindowState=FormWindowState.Normal;
                    form.FormBorderStyle=FormBorderStyle.Sizable;
                    form.Bounds=normal;
                    form.WindowState=FormWindowState.Maximized;
                } else if(command=="under-notch" || command=="beneath-notch" || command=="away-notch") {
                    form.WindowState=FormWindowState.Normal;
                    form.FormBorderStyle=FormBorderStyle.Sizable;
                    var screen=selected.Bounds;
                    form.Bounds=command!="away-notch"
                        ? new Rectangle(screen.Left+(screen.Width-640)/2,screen.Top,640,420)
                        : normal;
                } else if(command=="normal" || command=="restore") {
                    form.WindowState=FormWindowState.Normal;
                    form.FormBorderStyle=FormBorderStyle.Sizable;
                    form.Bounds=normal;
                } else return;
                form.Show(); ShowWindow(form.Handle,5); form.Activate(); SetForegroundWindow(form.Handle);
                last=command;
                File.WriteAllText(Path.Combine(directory,"ready.txt"),form.Handle.ToInt64().ToString());
                File.WriteAllText(Path.Combine(directory,"ack.txt"),command);
            };
            form.Shown+=(sender,args)=>{
                File.WriteAllText(Path.Combine(directory,"ready.txt"),form.Handle.ToInt64().ToString());
                timer.Start();
            };
            Application.Run(form);
        }
    }
}
'@
    [IslandFullscreenFixture]::Run($ControlDirectory,$MonitorId)
    exit
}

$workspace=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$executable=Join-Path $workspace "target\$Configuration\still-island.exe"
if(-not (Test-Path -LiteralPath $executable)){throw 'Build the release executable first.'}
if(Get-Process still-island -ErrorAction SilentlyContinue){throw 'Exit the existing island before this test; other instances are never stopped.'}
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class IslandFullscreenProbe {
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X,Y; }
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left,Top,Right,Bottom; }
    delegate bool EnumProc(IntPtr hwnd,IntPtr data);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc callback,IntPtr data);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd,out uint process);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr hwnd,StringBuilder text,int count);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point point);
    [StructLayout(LayoutKind.Sequential)] struct MouseInput { public int X,Y; public uint Data,Flags,Time; public UIntPtr Extra; }
    [StructLayout(LayoutKind.Explicit)] struct InputUnion { [FieldOffset(0)] public MouseInput Mouse; }
    [StructLayout(LayoutKind.Sequential)] struct Input { public uint Type; public InputUnion Data; }
    [DllImport("user32.dll",SetLastError=true)] static extern uint SendInput(uint count,Input[] inputs,int size);
    public static bool ClickOwnedFixture(IntPtr fixture,int process) {
        uint owner; GetWindowThreadProcessId(fixture,out owner);
        if(owner!=(uint)process) return false;
        Rect bounds; if(!GetWindowRect(fixture,out bounds)) return false;
        var point=new Point { X=(bounds.Left+bounds.Right)/2,Y=(bounds.Top+bounds.Bottom)/2 };
        if(!SetCursorPos(point.X,point.Y) || WindowFromPoint(point)!=fixture) return false;
        // The fixture contains no controls. Recheck the exact owned HWND before
        // injecting the paired down/up; never click through an unrelated window.
        GetWindowThreadProcessId(fixture,out owner);
        if(WindowFromPoint(point)!=fixture || owner!=(uint)process) return false;
        var inputs=new Input[2];
        inputs[0].Data.Mouse.Flags=0x0002;
        inputs[1].Data.Mouse.Flags=0x0004;
        return SendInput(2,inputs,Marshal.SizeOf(typeof(Input)))==2;
    }
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out Point point);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd,out Rect rect);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hwnd,IntPtr after,int x,int y,int width,int height,uint flags);
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [StructLayout(LayoutKind.Sequential,CharSet=CharSet.Unicode)] struct MonitorInfo {
        public int Size; public Rect Bounds,Work; public uint Flags;
        [MarshalAs(UnmanagedType.ByValTStr,SizeConst=32)] public string Device;
    }
    delegate bool MonitorProc(IntPtr monitor,IntPtr dc,ref Rect rect,IntPtr data);
    [DllImport("user32.dll")] static extern bool EnumDisplayMonitors(IntPtr dc,IntPtr clip,MonitorProc callback,IntPtr data);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern bool GetMonitorInfo(IntPtr monitor,ref MonitorInfo info);
    [DllImport("shcore.dll")] static extern int GetDpiForMonitor(IntPtr monitor,int kind,out uint x,out uint y);
    public static string[] MonitorInventory() {
        var rows=new System.Collections.Generic.List<string>();
        EnumDisplayMonitors(IntPtr.Zero,IntPtr.Zero,delegate(IntPtr monitor,IntPtr dc,ref Rect rect,IntPtr data) {
            var info=new MonitorInfo(); info.Size=Marshal.SizeOf(typeof(MonitorInfo));
            if(GetMonitorInfo(monitor,ref info)) {
                uint x=0,y=0; GetDpiForMonitor(monitor,0,out x,out y);
                rows.Add(String.Join("|",new string[] { info.Device,((info.Flags&1)!=0).ToString(),
                    info.Bounds.Left.ToString(),info.Bounds.Top.ToString(),info.Bounds.Right.ToString(),info.Bounds.Bottom.ToString(),x.ToString(),y.ToString() }));
            }
            return true;
        },IntPtr.Zero);
        return rows.ToArray();
    }
    [DllImport("gdi32.dll")] static extern IntPtr CreateRectRgn(int l,int t,int r,int b);
    [DllImport("gdi32.dll")] static extern int GetRgnBox(IntPtr region,out Rect rect);
    [DllImport("gdi32.dll")] static extern bool DeleteObject(IntPtr region);
    [DllImport("user32.dll")] static extern int GetWindowRgn(IntPtr hwnd,IntPtr region);
    public static int RegionWidth(IntPtr hwnd) {
        IntPtr region=CreateRectRgn(0,0,0,0);
        try { Rect rect; GetWindowRgn(hwnd,region); GetRgnBox(region,out rect); return rect.Right-rect.Left; }
        finally { DeleteObject(region); }
    }
    public static IntPtr Overlay(int process) {
        IntPtr found=IntPtr.Zero;
        EnumWindows((hwnd,data)=>{
            uint owner; GetWindowThreadProcessId(hwnd,out owner);
            if(owner!=(uint)process) return true;
            var text=new StringBuilder(256); GetWindowText(hwnd,text,text.Capacity);
            if(text.ToString()=="Still Island") { found=hwnd; return false; }
            return true;
        },IntPtr.Zero);
        return found;
    }
}
'@
$root=Join-Path $workspace ('target\fullscreen-probe\'+[guid]::NewGuid().ToString('N'))
$settings=Join-Path $root 'StillIsland'
New-Item -ItemType Directory -Force -Path $settings | Out-Null
@'
appearance=solid
form_factor=notch
island_scale=1
compact_now_playing=false
auto_volume=false
notifications_enabled=false
hover_expand=true
hover_delay=200
collapse_delay=200
fullscreen_hide=true
outside_click=false
shortcut_enabled=false
online_lyrics=false
battery_previews=false
'@ | Set-Content -LiteralPath (Join-Path $settings 'settings.ini')
if($FullscreenOverride -eq 'Hide'){
    $path=Join-Path $settings 'settings.ini'
    (Get-Content -LiteralPath $path -Raw).Replace('fullscreen_hide=true','fullscreen_hide=false') | Set-Content -LiteralPath $path
    Add-Content -LiteralPath $path -Value 'fullscreen_always_hide=powershell.exe'
}elseif($FullscreenOverride -eq 'Show'){
    Add-Content -LiteralPath (Join-Path $settings 'settings.ini') -Value 'fullscreen_always_show=powershell.exe'
}
$application=$null
$fixtureProcess=$null
$results=@()
$traceFile=Join-Path $workspace 'verification.log'
$traceStart=if(Test-Path -LiteralPath $traceFile){@(Get-Content -LiteralPath $traceFile).Count}else{0}
$originalForeground=[IslandFullscreenProbe]::GetForegroundWindow()
$previousDpi=[IslandFullscreenProbe]::SetThreadDpiAwarenessContext([IntPtr](-4))
$monitors=@([IslandFullscreenProbe]::MonitorInventory() | ForEach-Object {
    $fields=$_ -split '\|'
    [pscustomobject]@{Device=$fields[0];Primary=$fields[1];Left=$fields[2];Top=$fields[3];Right=$fields[4];Bottom=$fields[5];DpiX=$fields[6];DpiY=$fields[7];SelectedForTest=$false}
})
if($AllMonitors){
    if($MonitorId){throw '-AllMonitors and -MonitorId are mutually exclusive.'}
    if($previousDpi -ne [IntPtr]::Zero){$null=[IslandFullscreenProbe]::SetThreadDpiAwarenessContext($previousDpi)}
    $summary=@()
    $shell=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    foreach($monitor in $monitors){
        $childArguments=@('-NoProfile','-ExecutionPolicy','Bypass','-File',$PSCommandPath,'-MonitorId',$monitor.Device,'-FullscreenOverride',$FullscreenOverride,'-Configuration',$Configuration)
        if($SkipHover){$childArguments+='-SkipHover'}
        if($TraceHover){$childArguments+='-TraceHover'}
        & $shell @childArguments
        $summary += [pscustomobject]@{Device=$monitor.Device;DpiX=$monitor.DpiX;DpiY=$monitor.DpiY;Status=if($LASTEXITCODE -eq 0){'Passed'}else{'FailedOrInconclusive'};ExitCode=$LASTEXITCODE}
    }
    if($monitors.Count -lt 2){
        $summary += [pscustomobject]@{Device='Secondary monitor';DpiX='';DpiY='';Status='Unavailable: only one monitor connected; multi-monitor and mixed-DPI runtime not exercised';ExitCode=''}
    }
    $directory=Join-Path $workspace 'verification'
    New-Item -ItemType Directory -Force -Path $directory | Out-Null
    $summary | Export-Csv -LiteralPath (Join-Path $directory 'fullscreen-all-monitors.csv') -NoTypeInformation
    $summary | Format-Table -Wrap
    if(@($summary | Where-Object {$_.Status -eq 'FailedOrInconclusive'}).Count){exit 1}
    exit 0
}
$selectedMonitor=@(if($MonitorId){$monitors | Where-Object {$_.Device -eq $MonitorId}}else{$monitors | Where-Object {$_.Primary -eq 'True'}})
if($selectedMonitor.Count -ne 1){
    if($previousDpi -ne [IntPtr]::Zero){$null=[IslandFullscreenProbe]::SetThreadDpiAwarenessContext($previousDpi)}
    throw 'Requested monitor is unavailable; the test refuses to silently fall back to primary.'
}
$selectedMonitor=$selectedMonitor[0]
$selectedMonitor.SelectedForTest=$true
Add-Content -LiteralPath (Join-Path $settings 'settings.ini') -Value ('monitor_id='+$selectedMonitor.Device)
$monitorSuffix=if($MonitorId){'-'+($selectedMonitor.Device -replace '[^A-Za-z0-9_-]','')}else{''}
$savedPointer=New-Object IslandFullscreenProbe+Point
$null=[IslandFullscreenProbe]::GetCursorPos([ref]$savedPointer)
function Wait-StableFixturePointer([int]$X,[int]$Y,[int]$Milliseconds){
    $clock=[Diagnostics.Stopwatch]::StartNew()
    do{
        Start-Sleep -Milliseconds 100
        $actual=New-Object IslandFullscreenProbe+Point
        if(-not [IslandFullscreenProbe]::GetCursorPos([ref]$actual)){
            throw 'INCONCLUSIVE: the test could not read the desktop pointer.'
        }
        $dx=[double]$actual.X-$X
        $dy=[double]$actual.Y-$Y
        if([math]::Sqrt($dx*$dx+$dy*$dy) -gt 2){
            throw "INCONCLUSIVE: desktop pointer moved during the controlled hover test (expected $X,$Y; actual $($actual.X),$($actual.Y))."
        }
    }while($clock.ElapsedMilliseconds -lt $Milliseconds)
}
function Send-FixtureCommand([string]$Name){
    Set-Content -LiteralPath (Join-Path $root 'command.txt') -Value $Name
    $deadline=[DateTime]::UtcNow.AddSeconds(4)
    do{
        Start-Sleep -Milliseconds 100
        $ack=if(Test-Path -LiteralPath (Join-Path $root 'ack.txt')){(Get-Content -LiteralPath (Join-Path $root 'ack.txt') -Raw).Trim()}else{''}
    }while($ack -ne $Name -and [DateTime]::UtcNow -lt $deadline)
    if($ack -ne $Name){throw "Fixture did not acknowledge $Name."}
    [IntPtr]([long](Get-Content -LiteralPath (Join-Path $root 'ready.txt') -Raw))
}
try {
    $start=[Diagnostics.ProcessStartInfo]::new($executable)
    $start.UseShellExecute=$false
    $start.CreateNoWindow=$true
    $start.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden
    $start.WorkingDirectory=$workspace
    $start.Arguments='--appearance=solid'
    if($TraceHover){$start.Arguments+=' --trace-hover'}
    $start.EnvironmentVariables['LOCALAPPDATA']=$root
    $application=[Diagnostics.Process]::Start($start)
    $deadline=[DateTime]::UtcNow.AddSeconds(8)
    $overlay=[IntPtr]::Zero
    while($overlay -eq [IntPtr]::Zero -and [DateTime]::UtcNow -lt $deadline){
        if($application.HasExited){throw 'Overlay exited before the test.'}
        $overlay=[IslandFullscreenProbe]::Overlay($application.Id)
        Start-Sleep -Milliseconds 100
    }
    if($overlay -eq [IntPtr]::Zero){throw 'Overlay HWND was not created.'}
    # Native placement and Slint's initial frame must have completed before a
    # secondary-display assertion can distinguish placement from startup timing.
    Start-Sleep -Milliseconds 1000
    $position=New-Object IslandFullscreenProbe+Rect
    $null=[IslandFullscreenProbe]::GetWindowRect($overlay,[ref]$position)
    $expectedCenter=([double]$selectedMonitor.Left+[double]$selectedMonitor.Right)/2
    $actualCenter=($position.Left+$position.Right)/2
    $centered=[math]::Abs($expectedCenter-$actualCenter) -le 2 -and [math]::Abs($position.Top-[int]$selectedMonitor.Top) -le 2
    $results += [pscustomobject]@{Stage='selected-monitor-position';ExpectedVisible=$true;ActualVisible=[IslandFullscreenProbe]::IsWindowVisible($overlay);Pass=$centered;Detail="device=$($selectedMonitor.Device); expected center=$expectedCenter; actual=$actualCenter; dpi=$($selectedMonitor.DpiX)"}
    if(-not $centered){throw 'Overlay was not centered on the explicitly selected monitor.'}
    if(-not $SkipHover -and $FullscreenOverride -ne 'Hide' -and -not [IslandFullscreenProbe]::IsWindowVisible($overlay)){throw 'Overlay was initially hidden; close any existing fullscreen or maximized application before the hover test.'}
    # Create an explicit fresh outside-to-inside input edge after placement.
    if(-not $SkipHover -and $FullscreenOverride -eq 'Auto'){
    $bounds=New-Object IslandFullscreenProbe+Rect
    $null=[IslandFullscreenProbe]::GetWindowRect($overlay,[ref]$bounds)
    $hoverX=[int](($bounds.Left+$bounds.Right)/2)
    $outsideY=$bounds.Bottom+40
    $null=[IslandFullscreenProbe]::SetCursorPos($hoverX,$outsideY)
    Wait-StableFixturePointer $hoverX $outsideY 1100
    $idleWidth=[IslandFullscreenProbe]::RegionWidth($overlay)
    $physicalScale=[IslandFullscreenProbe]::GetDpiForWindow($overlay)/96.0
    $insideY=$bounds.Top+[int](16*$physicalScale)
    # Each visit ends before the configured 200ms dwell. None may inherit time
    # from an earlier visit; restrained hover feedback may add a few pixels.
    for($visit=0;$visit -lt 3;$visit++){
        $null=[IslandFullscreenProbe]::SetCursorPos($hoverX,$insideY)
        Wait-StableFixturePointer $hoverX $insideY 100
        $shortWidth=[IslandFullscreenProbe]::RegionWidth($overlay)
        $shortPass=$shortWidth -lt $idleWidth+40
        $results += [pscustomobject]@{Stage="short-hover-$visit";ExpectedVisible=$true;ActualVisible=[IslandFullscreenProbe]::IsWindowVisible($overlay);Pass=$shortPass;Detail="region width=$shortWidth; idle=$idleWidth"}
        if(-not $shortPass){throw 'A short crossing expanded before its dwell.'}
        $null=[IslandFullscreenProbe]::SetCursorPos($hoverX,$outsideY)
        Wait-StableFixturePointer $hoverX $outsideY 300
    }
    $null=[IslandFullscreenProbe]::SetCursorPos($hoverX,$insideY)
    Wait-StableFixturePointer $hoverX $insideY 1200
    $expandedWidth=[IslandFullscreenProbe]::RegionWidth($overlay)
    $expanded=$expandedWidth -gt $idleWidth+80
    $results += [pscustomobject]@{Stage='hover-expand';ExpectedVisible=$true;ActualVisible=[IslandFullscreenProbe]::IsWindowVisible($overlay);Pass=$expanded}
    if(-not $expanded){throw "Native hover did not expand: idle=$idleWidth expanded=$expandedWidth"}
    # Pointer-only travel across launcher child controls must keep the parent's
    # hover lifetime alive. Never click a media/volume/service command.
    foreach($point in @(@(-100,60),@(0,60),@(100,60),@(100,105),@(0,105),@(-100,105))){
        $childX=$hoverX+[int]($point[0]*$physicalScale); $childY=$bounds.Top+[int]($point[1]*$physicalScale)
        $null=[IslandFullscreenProbe]::SetCursorPos($childX,$childY)
        Wait-StableFixturePointer $childX $childY 350
        $childWidth=[IslandFullscreenProbe]::RegionWidth($overlay)
        $childPass=$childWidth -ge $expandedWidth-5
        $results += [pscustomobject]@{Stage="child-hover-$($point[0])-$($point[1])";ExpectedVisible=$true;ActualVisible=[IslandFullscreenProbe]::IsWindowVisible($overlay);Pass=$childPass;Detail="region width=$childWidth"}
        if(-not $childPass){throw 'Crossing expanded child controls contracted the island.'}
    }
    $null=[IslandFullscreenProbe]::GetWindowRect($overlay,[ref]$bounds)
    $outsideY=$bounds.Bottom+40
    $null=[IslandFullscreenProbe]::SetCursorPos($hoverX,$outsideY)
    Wait-StableFixturePointer $hoverX $outsideY 1400
    $contractedWidth=[IslandFullscreenProbe]::RegionWidth($overlay)
    $contracted=[math]::Abs($contractedWidth-$idleWidth) -le 3
    $results += [pscustomobject]@{Stage='hover-contract';ExpectedVisible=$true;ActualVisible=[IslandFullscreenProbe]::IsWindowVisible($overlay);Pass=$contracted}
    if(-not $contracted){throw "Native pointer exit did not contract: idle=$idleWidth current=$contractedWidth"}
    }
    # Only this short-lived fixture takes foreground; no existing app is resized.
    $shell=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $fixtureProcess=Start-Process -FilePath $shell -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-STA','-File',('"'+$PSCommandPath+'"'),'-Fixture','-ControlDirectory',('"'+$root+'"'),'-MonitorId',('"'+$selectedMonitor.Device+'"') -WindowStyle Hidden -PassThru
    $deadline=[DateTime]::UtcNow.AddSeconds(10)
    while(-not (Test-Path -LiteralPath (Join-Path $root 'ready.txt')) -and [DateTime]::UtcNow -lt $deadline){
        if($fixtureProcess.HasExited){throw 'Fullscreen fixture exited early.'}
        Start-Sleep -Milliseconds 100
    }
    if(-not (Test-Path -LiteralPath (Join-Path $root 'ready.txt'))){throw 'Fullscreen fixture did not become ready.'}
    $fixtureWindow=[IntPtr]([long](Get-Content -LiteralPath (Join-Path $root 'ready.txt') -Raw))
    if(-not $SkipHover -and $FullscreenOverride -eq 'Auto'){
        $null=[IslandFullscreenProbe]::SetCursorPos($hoverX,$insideY)
        Wait-StableFixturePointer $hoverX $insideY 1200
        if([IslandFullscreenProbe]::RegionWidth($overlay) -lt $idleWidth+80){throw 'Island did not reopen before the stationary pointer test.'}
        $fixtureWindow=Send-FixtureCommand 'beneath-notch'
        Wait-StableFixturePointer $hoverX $insideY 1400
        $hit=New-Object IslandFullscreenProbe+Point; $hit.X=$hoverX; $hit.Y=$insideY
        if([IslandFullscreenProbe]::WindowFromPoint($hit) -ne $overlay){throw 'INCONCLUSIVE: captioned fixture did not remain beneath the notch hit target.'}
        $beneathWidth=[IslandFullscreenProbe]::RegionWidth($overlay)
        $beneathVisible=[IslandFullscreenProbe]::IsWindowVisible($overlay)
        $beneathPass=$beneathVisible -and $beneathWidth -gt $idleWidth+80
        $results += [pscustomobject]@{Stage='captioned-beneath-stationary';ExpectedVisible=$true;ActualVisible=$beneathVisible;Pass=$beneathPass;Detail="region width=$beneathWidth"}
        if(-not $beneathPass){throw 'A captioned window dragged beneath the notch hid it or cleared unobstructed hover.'}
        $fixtureWindow=Send-FixtureCommand 'under-notch'
        Wait-StableFixturePointer $hoverX $insideY 1400
        $hit=New-Object IslandFullscreenProbe+Point; $hit.X=$hoverX; $hit.Y=$insideY
        if([IslandFullscreenProbe]::WindowFromPoint($hit) -ne $fixtureWindow){throw 'INCONCLUSIVE: owned fixture did not cover the stationary pointer.'}
        $obscuredWidth=[IslandFullscreenProbe]::RegionWidth($overlay)
        $obscuredPass=[math]::Abs($obscuredWidth-$idleWidth) -le 3
        $results += [pscustomobject]@{Stage='stationary-covered';ExpectedVisible=$true;ActualVisible=[IslandFullscreenProbe]::IsWindowVisible($overlay);Pass=$obscuredPass;Detail="region width=$obscuredWidth"}
        if(-not $obscuredPass){throw 'Moving a fixture over the stationary pointer did not clear hover.'}
        $fixtureWindow=Send-FixtureCommand 'away-notch'
        Wait-StableFixturePointer $hoverX $insideY 1400
        $uncoveredWidth=[IslandFullscreenProbe]::RegionWidth($overlay)
        $uncoveredPass=$uncoveredWidth -gt $idleWidth+80
        $results += [pscustomobject]@{Stage='stationary-uncovered';ExpectedVisible=$true;ActualVisible=[IslandFullscreenProbe]::IsWindowVisible($overlay);Pass=$uncoveredPass;Detail="region width=$uncoveredWidth"}
        if(-not $uncoveredPass){throw 'Moving a fixture away did not reconcile stationary hover.'}
        $null=[IslandFullscreenProbe]::SetCursorPos($hoverX,$outsideY)
        Wait-StableFixturePointer $hoverX $outsideY 1400
    }
    $stages=@(@{Name='normal';Visible=$true},@{Name='maximized';Visible=$false},@{Name='fullscreen';Visible=$false},@{Name='restore';Visible=$true})
    foreach($stage in $stages){
        if($FullscreenOverride -eq 'Hide'){$stage.Visible=$false}
        elseif($FullscreenOverride -eq 'Show'){$stage.Visible=$true}
        $fixtureWindow=Send-FixtureCommand $stage.Name
        if([IslandFullscreenProbe]::GetForegroundWindow() -ne $fixtureWindow){
            if($SkipHover){throw 'INCONCLUSIVE: owned fixture did not obtain foreground; no input was injected.'}
            if(-not [IslandFullscreenProbe]::ClickOwnedFixture($fixtureWindow,$fixtureProcess.Id)){
                throw 'Fixture is not the verified click target; no focus click was sent and result is inconclusive.'
            }
            Start-Sleep -Milliseconds 200
            if([IslandFullscreenProbe]::GetForegroundWindow() -ne $fixtureWindow){throw 'Owned fixture click did not obtain foreground; result is inconclusive.'}
        }
        # Give queued native policy events time to hide/show the overlay, and then
        # require the expected state to stay stable for 400ms (not a transient hit).
        $stable=0
        $deadline=[DateTime]::UtcNow.AddSeconds(4)
        do{
            Start-Sleep -Milliseconds 100
            $visible=[IslandFullscreenProbe]::IsWindowVisible($overlay)
            if($visible -eq $stage.Visible){$stable++}else{$stable=0}
        }while($stable -lt 4 -and [DateTime]::UtcNow -lt $deadline)
        $pass=$stable -ge 4
        $results += [pscustomobject]@{Stage=$stage.Name;ExpectedVisible=$stage.Visible;ActualVisible=$visible;Pass=$pass}
        if(-not $pass){throw "Overlay visibility failed at $($stage.Name)."}
        if($stage.Name -eq 'fullscreen' -and -not $stage.Visible){
            $null=[IslandFullscreenProbe]::SetWindowPos($overlay,[IntPtr]::Zero,0,0,0,0,0x57)
            Start-Sleep -Milliseconds 400
            $stillHidden=-not [IslandFullscreenProbe]::IsWindowVisible($overlay)
            $results += [pscustomobject]@{Stage='hidden-backend-reshow';ExpectedVisible=$false;ActualVisible=(-not $stillHidden);Pass=$stillHidden}
            if(-not $stillHidden){throw 'A backend show request resurrected the hidden overlay.'}
        }
    }
}finally{
    $foreground=[IslandFullscreenProbe]::GetForegroundWindow()
    $restoreFocus=($foreground -eq $fixtureWindow -or $foreground -eq $overlay)
    if(-not $SkipHover){$null=[IslandFullscreenProbe]::SetCursorPos($savedPointer.X,$savedPointer.Y)}
    if($previousDpi -ne [IntPtr]::Zero){$null=[IslandFullscreenProbe]::SetThreadDpiAwarenessContext($previousDpi)}
    if($fixtureProcess -and -not $fixtureProcess.HasExited){
        Set-Content -LiteralPath (Join-Path $root 'command.txt') -Value 'quit'
        if(-not $fixtureProcess.WaitForExit(2000)){Stop-Process -Id $fixtureProcess.Id}
    }
    if($application -and -not $application.HasExited){
        $null=$application.CloseMainWindow()
        if(-not $application.WaitForExit(2000)){Stop-Process -Id $application.Id}
    }
    if($restoreFocus -and [IslandFullscreenProbe]::IsWindow($originalForeground) -and [IslandFullscreenProbe]::IsWindowVisible($originalForeground) -and -not [IslandFullscreenProbe]::IsIconic($originalForeground)){
        $null=[IslandFullscreenProbe]::SetForegroundWindow($originalForeground)
    }
    $directory=Join-Path $workspace 'verification'
    New-Item -ItemType Directory -Force -Path $directory | Out-Null
    $resultFile=if($FullscreenOverride -eq 'Auto'){'fullscreen-hide'+$monitorSuffix+'.csv'}else{'fullscreen-override-'+$FullscreenOverride.ToLowerInvariant()+$monitorSuffix+'.csv'}
    $results | Select-Object Stage,ExpectedVisible,ActualVisible,Pass,Detail | Export-Csv -LiteralPath (Join-Path $directory $resultFile) -NoTypeInformation
    $monitors | Export-Csv -LiteralPath (Join-Path $directory ('fullscreen-monitor-inventory'+$monitorSuffix+'.csv')) -NoTypeInformation
    $monitors | Format-Table
    Write-Host ('Only selected monitor '+$selectedMonitor.Device+' is exercised in this run. This fixture does not change display configuration/scaling or test exclusive games. Use -AllMonitors to run each currently connected display sequentially.')
    $results | Format-Table
    if($TraceHover -and (Test-Path -LiteralPath $traceFile)){
        # Emit plain strings rather than MatchInfo formatting records, including
        # the appended count, so traces remain visible alongside table output.
        $appended=@(Get-Content -LiteralPath $traceFile | Select-Object -Skip $traceStart)
        [Console]::Error.WriteLine(("Hover trace: {0} appended log lines" -f $appended.Count))
        foreach($line in @($appended | Where-Object { $_ -match 'hover|pointer|dwell|collapse' } | Select-Object -Last 100)){
            [Console]::Error.WriteLine([string]$line)
        }
    }
}
