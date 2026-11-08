"""Interactive, owned-window-only picker fixture comparison; no user app input."""
import ctypes as C
import ctypes.wintypes as W
import json
import pathlib
import subprocess
import time
from PIL import ImageGrab

root=pathlib.Path(__file__).resolve().parents[1]
directory=root/'target/native-picker'
u=C.WinDLL('user32',use_last_error=True)
callback=C.WINFUNCTYPE(W.BOOL,W.HWND,W.LPARAM)
u.EnumWindows.argtypes=[callback,W.LPARAM]
u.GetForegroundWindow.restype=W.HWND
u.SetForegroundWindow.argtypes=[W.HWND]
u.GetWindowThreadProcessId.argtypes=[W.HWND,C.POINTER(W.DWORD)]
u.GetWindowTextW.argtypes=[W.HWND,W.LPWSTR,C.c_int]
u.IsWindowVisible.argtypes=[W.HWND]
u.PostMessageW.argtypes=[W.HWND,W.UINT,W.WPARAM,W.LPARAM]
class RECT(C.Structure):_fields_=[('left',W.LONG),('top',W.LONG),('right',W.LONG),('bottom',W.LONG)]
u.GetWindowRect.argtypes=[W.HWND,C.POINTER(RECT)]
u.SetThreadDpiAwarenessContext.argtypes=[W.HANDLE]
u.SetThreadDpiAwarenessContext.restype=W.HANDLE
def owner(hwnd):
 pid=W.DWORD();u.GetWindowThreadProcessId(hwnd,C.byref(pid));return pid.value
previous=u.GetForegroundWindow()
dpi=u.SetThreadDpiAwarenessContext(C.c_void_p(-4))
results=[]
owned=0
try:
 for binary,apartment in [('plain.exe','sta'),('plain.exe','mta'),('v6.exe','sta')]:
  process=subprocess.Popen([str(directory/binary),apartment,str(root/'target/plugin-packages')],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,creationflags=0x08000000)
  owned=0
  try:
   end=time.monotonic()+6
   while time.monotonic()<end and not owned and process.poll() is None:
    found=[]
    @callback
    def visit(hwnd,_):
     title=C.create_unicode_buffer(256);u.GetWindowTextW(hwnd,title,256)
     if owner(hwnd)==process.pid and u.IsWindowVisible(hwnd) and title.value.startswith('Still Island Picker Probe'):found.append(hwnd)
     return True
    u.EnumWindows(visit,0)
    if found:owned=found[0]
    time.sleep(.1)
   image=None
   if owned:
    u.SetForegroundWindow(owned);time.sleep(1)
    if owner(owned)!=process.pid or u.GetForegroundWindow()!=owned:raise RuntimeError('INCONCLUSIVE: picker is not the verified owned foreground window')
    rect=RECT();u.GetWindowRect(owned,C.byref(rect))
    image=directory/(pathlib.Path(binary).stem+'-'+apartment+'.png')
    ImageGrab.grab(bbox=(rect.left,rect.top,rect.right,rect.bottom),all_screens=True).save(image)
    u.PostMessageW(owned,0x10,0,0)
   else:process.terminate()
   stdout,stderr=process.communicate(timeout=3)
   row=dict(binary=binary,apartment=apartment,dialog_created=bool(owned),screenshot=str(image),stdout=stdout,stderr=stderr,exit_code=process.returncode)
   results.append(row);print(json.dumps(row),flush=True)
  finally:
   if process.poll() is None:
    if owned and owner(owned)==process.pid:u.PostMessageW(owned,0x10,0,0)
    try:process.wait(timeout=2)
    except subprocess.TimeoutExpired:process.terminate();process.wait(timeout=2)
finally:
 (directory/'results.json').write_text(json.dumps(results,indent=2),encoding='utf-8')
 # Only restore if our last owned window still owns foreground; never steal it
 # back after the user changed applications during the probe.
 if u.GetForegroundWindow()==owned:u.SetForegroundWindow(previous)
 if dpi:u.SetThreadDpiAwarenessContext(dpi)
