# Integrated opt-in regression: cancels one owned picker, selects a known ZIP in
# another, and captures only its verified foreground rectangle. No package import.
param(
    [string]$Binary='target\debug\still-island.exe',
    [string]$Package='',
    [string]$SuccessMarker='picker verification complete'
)
$ErrorActionPreference='Stop'
$workspace=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if(-not [IO.Path]::IsPathRooted($Binary)){$Binary=Join-Path $workspace $Binary}
$Binary=(Resolve-Path -LiteralPath $Binary).Path
if(Get-Process still-island -ErrorAction SilentlyContinue){throw 'Exit the existing island first; this test never closes another instance or dialog.'}
if(-not $Package){$Package=(Get-ChildItem -LiteralPath (Join-Path $workspace 'target\plugin-packages') -Filter '*.zip' -File | Select-Object -First 1).FullName}
$Package=(Resolve-Path -LiteralPath $Package).Path
if([IO.Path]::GetExtension($Package) -ne '.zip'){throw 'Choose a generated test ZIP package.'}
$folder=Split-Path -Parent $Package
if(-not $folder.StartsWith((Join-Path $workspace 'target'),[StringComparison]::OrdinalIgnoreCase)){throw 'The fixture only selects generated packages under target.'}
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes,System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class IslandPickerProbe {
    public delegate bool EnumProc(IntPtr hwnd,IntPtr data);
    [StructLayout(LayoutKind.Sequential)] public struct Rect {public int Left,Top,Right,Bottom;}
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc callback,IntPtr data);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent,EnumProc callback,IntPtr data);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr hwnd,StringBuilder text,int count);
    [DllImport("user32.dll")] static extern int GetDlgCtrlID(IntPtr hwnd);
    [DllImport("user32.dll")] static extern IntPtr GetParent(IntPtr hwnd);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern IntPtr SendMessageTimeout(IntPtr hwnd,uint message,IntPtr wparam,string text,uint flags,uint timeout,out IntPtr result);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd,out uint process);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr hwnd,StringBuilder text,int count);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd,out Rect rect);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd,uint message,IntPtr wparam,IntPtr lparam);
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    public static bool SetFilename(IntPtr dialog,string value,int process) {
        uint owner;GetWindowThreadProcessId(dialog,out owner);if(owner!=(uint)process)return false;
        IntPtr edit=IntPtr.Zero;
        EnumChildWindows(dialog,(child,data)=>{
            var name=new StringBuilder(128);GetClassName(child,name,name.Capacity);
            int id=GetDlgCtrlID(child),parent=GetDlgCtrlID(GetParent(child));
            if(name.ToString()=="Edit" && (id==1148 || id==1001 || parent==1148)) {edit=child;return false;}
            return true;
        },IntPtr.Zero);
        if(edit==IntPtr.Zero)return false;
        GetWindowThreadProcessId(edit,out owner);if(owner!=(uint)process)return false;
        IntPtr result;return SendMessageTimeout(edit,0xC,IntPtr.Zero,value,2,2000,out result)!=IntPtr.Zero && result!=IntPtr.Zero;
    }
    public static IntPtr Dialog(int process) {
        IntPtr result=IntPtr.Zero;
        EnumWindows((hwnd,data)=>{
            uint owner;GetWindowThreadProcessId(hwnd,out owner);
            if(owner!=(uint)process || !IsWindowVisible(hwnd)) return true;
            var text=new StringBuilder(256);GetWindowText(hwnd,text,text.Capacity);
            if(text.ToString()=="Select a plugin package") {result=hwnd;return false;}
            return true;
        },IntPtr.Zero);
        return result;
    }
}
'@
$run=Join-Path $workspace ('target\plugin-picker-probe\'+[guid]::NewGuid().ToString('N'))
$preferences=Join-Path $run 'StillIsland'
New-Item -ItemType Directory -Force -Path $preferences | Out-Null
@'
appearance=solid
hover_expand=false
compact_now_playing=false
auto_volume=false
notifications_enabled=false
battery_previews=false
online_lyrics=false
'@ | Set-Content -LiteralPath (Join-Path $preferences 'settings.ini')
$trace=Join-Path $workspace 'verification.log'
$traceStart=if(Test-Path -LiteralPath $trace){@(Get-Content -LiteralPath $trace).Count}else{0}
$application=$null
$dialog=[IntPtr]::Zero
$previousForeground=[IslandPickerProbe]::GetForegroundWindow()
$previousDpi=[IslandPickerProbe]::SetThreadDpiAwarenessContext([IntPtr](-4))
function Assert-OwnedDialog([IntPtr]$Window){
    $ownerId=[uint32]0
    $null=[IslandPickerProbe]::GetWindowThreadProcessId($Window,[ref]$ownerId)
    if($ownerId -ne $application.Id -or -not [IslandPickerProbe]::IsWindowVisible($Window)){throw 'Owned picker identity changed; no automation sent.'}
}
function Wait-Dialog([IntPtr]$Previous){
    $deadline=[DateTime]::UtcNow.AddSeconds(10)
    do{
        if($application.HasExited){throw 'App exited before completing picker stages.'}
        $found=[IslandPickerProbe]::Dialog($application.Id)
        if($found -ne [IntPtr]::Zero -and $found -ne $Previous){return $found}
        Start-Sleep -Milliseconds 100
    }while([DateTime]::UtcNow -lt $deadline)
    throw 'Owned native plugin picker did not appear.'
}
function Find-Filename([IntPtr]$Window){
    Assert-OwnedDialog $Window
    $deadline=[DateTime]::UtcNow.AddSeconds(3)
    do{
    $root=[System.Windows.Automation.AutomationElement]::FromHandle($Window)
    $all=$root.FindAll([System.Windows.Automation.TreeScope]::Descendants,[System.Windows.Automation.Condition]::TrueCondition)
    foreach($element in $all){
        $current=$element.Current
        if($current.IsEnabled -and ($current.AutomationId -eq '1001' -or $current.AutomationId -eq '1148' -or $current.Name -eq 'File name:')){
            $pattern=$null
            if($element.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern,[ref]$pattern)){
                return [System.Windows.Automation.ValuePattern]$pattern
            }
        }
    }
    Start-Sleep -Milliseconds 100
    }while([DateTime]::UtcNow -lt $deadline)
    $all | ForEach-Object { [pscustomobject]@{Id=$_.Current.AutomationId;Name=$_.Current.Name;Type=$_.Current.ControlType.ProgrammaticName} } | Export-Csv -LiteralPath (Join-Path $run 'uia-diagnostic.csv') -NoTypeInformation
    throw 'Native picker filename ValuePattern was not found.'
}
function Invoke-Select([IntPtr]$Window){
    Assert-OwnedDialog $Window
    $root=[System.Windows.Automation.AutomationElement]::FromHandle($Window)
    $buttons=$root.FindAll([System.Windows.Automation.TreeScope]::Descendants,(New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty,[System.Windows.Automation.ControlType]::Button)))
    foreach($button in $buttons){
        if($button.Current.Name -eq 'Select' -or $button.Current.Name -eq 'Open'){
            $pattern=$null
            if($button.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern,[ref]$pattern)){
                ([System.Windows.Automation.InvokePattern]$pattern).Invoke();return
            }
        }
    }
    # Legacy themed controls can expose only Pane through UIA. Send the standard
    # IDOK command exclusively to the already verified owned IFileOpenDialog.
    $null=[IslandPickerProbe]::PostMessage($Window,0x111,[IntPtr]1,[IntPtr]::Zero)
}
function Set-Filename([IntPtr]$Window,[string]$Value){
    Assert-OwnedDialog $Window
    # Prefer the exact owned Edit: this shell's legacy themed controls expose
    # Pane through UIA instead of ValuePattern, despite rendering correctly.
    $deadline=[DateTime]::UtcNow.AddSeconds(3)
    do{
        if([IslandPickerProbe]::SetFilename($Window,$Value,$application.Id)){return}
        Start-Sleep -Milliseconds 100
        Assert-OwnedDialog $Window
    }while([DateTime]::UtcNow -lt $deadline)
    (Find-Filename $Window).SetValue($Value)
}
function Save-OwnedDialog([IntPtr]$Window,[string]$Name){
    Assert-OwnedDialog $Window
    $null=[IslandPickerProbe]::SetForegroundWindow($Window)
    Start-Sleep -Milliseconds 500
    if([IslandPickerProbe]::GetForegroundWindow() -ne $Window){throw 'INCONCLUSIVE: picker is not foreground; no screenshot captured.'}
    $bounds=New-Object IslandPickerProbe+Rect
    $null=[IslandPickerProbe]::GetWindowRect($Window,[ref]$bounds)
    $bitmap=New-Object Drawing.Bitmap(($bounds.Right-$bounds.Left),($bounds.Bottom-$bounds.Top))
    try{
        $graphics=[Drawing.Graphics]::FromImage($bitmap)
        try{$graphics.CopyFromScreen($bounds.Left,$bounds.Top,0,0,$bitmap.Size)}finally{$graphics.Dispose()}
        $bitmap.Save((Join-Path $run $Name),[Drawing.Imaging.ImageFormat]::Png)
    }finally{$bitmap.Dispose()}
}
try{
    $start=[Diagnostics.ProcessStartInfo]::new($Binary)
    $start.UseShellExecute=$false;$start.CreateNoWindow=$true;$start.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden
    $start.WorkingDirectory=$workspace
    $start.Arguments='--verify-plugins --verify-plugin-picker --verify-picker-package="'+$Package+'"'
    $start.EnvironmentVariables['LOCALAPPDATA']=$run
    $application=[Diagnostics.Process]::Start($start)
    $dialog=Wait-Dialog ([IntPtr]::Zero)
    Set-Filename $dialog ($folder+[IO.Path]::DirectorySeparatorChar)
    Invoke-Select $dialog
    Start-Sleep -Milliseconds 700
    Save-OwnedDialog $dialog 'cancel-stage.png'
    Assert-OwnedDialog $dialog
    $first=$dialog
    $null=[IslandPickerProbe]::PostMessage($dialog,0x10,[IntPtr]::Zero,[IntPtr]::Zero)
    $dialog=Wait-Dialog $first
    Set-Filename $dialog ($folder+[IO.Path]::DirectorySeparatorChar)
    Invoke-Select $dialog
    Start-Sleep -Milliseconds 700
    Set-Filename $dialog $Package
    Save-OwnedDialog $dialog 'select-stage.png'
    Invoke-Select $dialog
    $dialog=[IntPtr]::Zero
    if(-not $application.WaitForExit(12000)){throw 'Integrated picker verification did not finish.'}
    if($application.ExitCode -ne 0){throw "Picker verifier exited $($application.ExitCode)."}
    $lines=@(Get-Content -LiteralPath $trace | Select-Object -Skip $traceStart)
    $lines | Set-Content -LiteralPath (Join-Path $run 'verification.txt')
    if(-not ($lines -match [regex]::Escape($SuccessMarker))){throw 'App exited without the expected integrated picker success marker.'}
    if($lines -match 'picker FAILED'){throw 'An integrated picker assertion failed.'}
    if(@($lines -match 'picker PASS').Count -lt 6){throw 'Expected six integrated picker assertions, including GUI pulses and selection-only behavior.'}
    Write-Output ('PASS integrated plugin picker: cancellation, selected ZIP, GUI pulse verification; screenshots/log: '+$run)
}finally{
    if($application -and -not $application.HasExited){
        if($dialog -ne [IntPtr]::Zero){
            $ownerId=[uint32]0;$null=[IslandPickerProbe]::GetWindowThreadProcessId($dialog,[ref]$ownerId)
            if($ownerId -eq $application.Id){$null=[IslandPickerProbe]::PostMessage($dialog,0x10,[IntPtr]::Zero,[IntPtr]::Zero)}
        }
        if(-not $application.WaitForExit(2000)){Stop-Process -Id $application.Id}
    }
    if([IslandPickerProbe]::GetForegroundWindow() -eq $dialog){$null=[IslandPickerProbe]::SetForegroundWindow($previousForeground)}
    if($previousDpi -ne [IntPtr]::Zero){$null=[IslandPickerProbe]::SetThreadDpiAwarenessContext($previousDpi)}
}
