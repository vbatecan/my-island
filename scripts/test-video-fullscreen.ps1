# Exercises a real installed VLC renderer, using only generated silent video and
# isolated preferences. Requires Python 3 and an existing release build.
param([string]$VlcPath='C:\Program Files\VideoLAN\VLC\vlc.exe')
$ErrorActionPreference='Stop'
$workspace=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if(Get-Process still-island,vlc -ErrorAction SilentlyContinue){throw 'Exit existing island/VLC instances first; the probe never stops other instances.'}
if(-not (Test-Path -LiteralPath $VlcPath)){throw 'Specify an installed VLC executable with -VlcPath.'}
$run=Join-Path $workspace ('target\video-probe\'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $run | Out-Null
$version=(Get-Item -LiteralPath $VlcPath).VersionInfo.FileVersion
$script=Join-Path $run 'probe.py'
@'
import ctypes as C, ctypes.wintypes as W
import csv, json, os, pathlib, socket, struct, subprocess, sys, time

workspace, run, vlc_path, version = map(str, sys.argv[1:])
run=pathlib.Path(run)
u=C.WinDLL('user32',use_last_error=True)
class POINT(C.Structure): _fields_=[('x',W.LONG),('y',W.LONG)]
class RECT(C.Structure): _fields_=[('left',W.LONG),('top',W.LONG),('right',W.LONG),('bottom',W.LONG)]
for name, result, args in [
 ('GetForegroundWindow',W.HWND,[]),('SetForegroundWindow',W.BOOL,[W.HWND]),
 ('IsWindowVisible',W.BOOL,[W.HWND]),('IsWindow',W.BOOL,[W.HWND]),
 ('GetWindowRect',W.BOOL,[W.HWND,C.POINTER(RECT)]),
 ('GetWindowThreadProcessId',W.DWORD,[W.HWND,C.POINTER(W.DWORD)]),
 ('WindowFromPoint',W.HWND,[POINT]),('GetCursorPos',W.BOOL,[C.POINTER(POINT)]),
 ('SetCursorPos',W.BOOL,[C.c_int,C.c_int]),
 ('SetThreadDpiAwarenessContext',W.HANDLE,[W.HANDLE]),
 ('MonitorFromWindow',W.HANDLE,[W.HWND,W.DWORD])]:
 fn=getattr(u,name);fn.restype=result;fn.argtypes=args
class MONITORINFO(C.Structure):
 _fields_=[('size',W.DWORD),('monitor',RECT),('work',RECT),('flags',W.DWORD)]
u.GetMonitorInfoW.argtypes=[W.HANDLE,C.POINTER(MONITORINFO)]
u.GetWindowTextW.argtypes=[W.HWND,W.LPWSTR,C.c_int]
u.GetClientRect.argtypes=[W.HWND,C.POINTER(RECT)]
u.ClientToScreen.argtypes=[W.HWND,C.POINTER(POINT)]
u.EnumWindows.argtypes=[C.WINFUNCTYPE(W.BOOL,W.HWND,W.LPARAM),W.LPARAM]
class MI(C.Structure):
 _fields_=[('dx',W.LONG),('dy',W.LONG),('data',W.DWORD),('flags',W.DWORD),('time',W.DWORD),('extra',C.c_size_t)]
class UNION(C.Union): _fields_=[('mi',MI)]
class INPUT(C.Structure): _fields_=[('kind',W.DWORD),('data',UNION)]
u.SendInput.argtypes=[W.UINT,C.POINTER(INPUT),C.c_int]
def owner(hwnd):
 pid=W.DWORD();u.GetWindowThreadProcessId(hwnd,C.byref(pid));return pid.value
def windows(pid):
 result=[]
 @C.WINFUNCTYPE(W.BOOL,W.HWND,W.LPARAM)
 def visit(hwnd,_):
  if owner(hwnd)==pid and u.IsWindowVisible(hwnd):
   rect=RECT();u.GetWindowRect(hwnd,C.byref(rect));text=C.create_unicode_buffer(256);u.GetWindowTextW(hwnd,text,256)
   result.append((hwnd,text.value,(rect.right-rect.left)*(rect.bottom-rect.top)))
  return True
 u.EnumWindows(visit,0);return result
def wait_for(get, seconds=8):
 end=time.monotonic()+seconds
 while time.monotonic()<end:
  result=get()
  if result:return result
  time.sleep(.1)
 raise RuntimeError('Timed out waiting for owned test window/renderer state')
def focus_owned(hwnd,pid):
 if owner(hwnd)!=pid:raise RuntimeError('Unexpected video window ownership')
 u.SetForegroundWindow(hwnd);time.sleep(.2)
 if u.GetForegroundWindow()==hwnd:return
 rect=RECT();u.GetWindowRect(hwnd,C.byref(rect));point=POINT((rect.left+rect.right)//2,(rect.top+rect.bottom)//2)
 u.SetCursorPos(point.x,point.y)
 if u.WindowFromPoint(point)!=hwnd or owner(hwnd)!=pid:raise RuntimeError('INCONCLUSIVE: owned video is not the verified focus-click target')
 inputs=(INPUT*2)();inputs[0].data.mi.flags=2;inputs[1].data.mi.flags=4
 if u.SendInput(2,inputs,C.sizeof(INPUT))!=2:raise RuntimeError('Owned focus click could not be injected')
 wait_for(lambda:u.GetForegroundWindow()==hwnd,2)
def geometry(hwnd):
 client=RECT();u.GetClientRect(hwnd,C.byref(client));point=POINT();u.ClientToScreen(hwnd,C.byref(point))
 info=MONITORINFO();info.size=C.sizeof(info);u.GetMonitorInfoW(u.MonitorFromWindow(hwnd,2),C.byref(info))
 rect=[point.x,point.y,point.x+client.right,point.y+client.bottom]
 mon=[info.monitor.left,info.monitor.top,info.monitor.right,info.monitor.bottom]
 covers=rect[0]<=mon[0]+2 and rect[1]<=mon[1]+2 and rect[2]>=mon[2]-2 and rect[3]>=mon[3]-2
 return rect,mon,covers
def chunk(tag,data):return tag+struct.pack('<I',len(data))+data+(b'\0' if len(data)%2 else b'')
def listing(tag,data):return chunk(b'LIST',tag+data)
# Uncompressed 160x90 DIB AVI: generated moving stripes, no user recordings.
w,h,fps,n=160,90,10,300
frame_size=w*h*3
avih=struct.pack('<14I',1000000//fps,frame_size*fps,0,16,n,0,1,frame_size,w,h,0,0,0,0)
strh=struct.pack('<4s4sIHHIIIIIIIIhhhh',b'vids',b'DIB ',0,0,0,0,1,fps,0,n,frame_size,0xffffffff,0,0,0,w,h)
strf=struct.pack('<IiiHHIIiiII',40,w,h,1,24,0,frame_size,0,0,0,0)
header=listing(b'hdrl',chunk(b'avih',avih)+listing(b'strl',chunk(b'strh',strh)+chunk(b'strf',strf)))
frames=[];index=[];offset=4
for i in range(n):
 row=bytes(v for x in range(w) for v in ((x+i*3)%256,80,180))
 data=chunk(b'00db',row*h);frames.append(data);index.append(struct.pack('<4sIII',b'00db',16,offset,frame_size));offset+=len(data)
avi=b'AVI '+header+listing(b'movi',b''.join(frames))+chunk(b'idx1',b''.join(index))
video=run/'synthetic.avi';video.write_bytes(chunk(b'RIFF',avi))
del frames,avi,index
settings=run/'profile'/'StillIsland';settings.mkdir(parents=True)
(settings/'settings.ini').write_text('appearance=solid\nform_factor=notch\nfullscreen_hide=true\nhover_expand=false\nauto_volume=false\ncompact_now_playing=false\nnotifications_enabled=false\nbattery_previews=false\nonline_lyrics=false\n',encoding='utf-8')
previous=u.GetForegroundWindow();pointer=POINT();u.GetCursorPos(C.byref(pointer));dpi=u.SetThreadDpiAwarenessContext(C.c_void_p(-4))
overlay_process=player=None;remote=None;rows=[];owned_foreground=False
try:
 env=os.environ.copy();env['LOCALAPPDATA']=str(run/'profile')
 overlay_process=subprocess.Popen([str(pathlib.Path(workspace)/'target/release/still-island.exe'),'--appearance=solid'],env=env,cwd=workspace)
 overlay=wait_for(lambda:next((hwnd for hwnd,title,_ in windows(overlay_process.pid) if title=='Still Island'),None))
 port_socket=socket.socket();port_socket.bind(('127.0.0.1',0));port=port_socket.getsockname()[1];port_socket.close()
 log=open(run/'vlc.log','w',encoding='utf-8')
 args=[vlc_path,'--ignore-config','--config='+str(run/'vlcrc'),'--no-one-instance','--intf=dummy','--extraintf=rc','--rc-host=127.0.0.1:'+str(port),'--rc-quiet','--no-audio','--no-metadata-network-access','--no-video-title-show','--no-fullscreen','--loop',str(video)]
 vlc_env=os.environ.copy()
 for variable in ['APPDATA','LOCALAPPDATA']:
  directory=run/('vlc-'+variable.lower());directory.mkdir(exist_ok=True);vlc_env[variable]=str(directory)
 player=subprocess.Popen(args,cwd=run,stdout=log,stderr=log,env=vlc_env)
 def connect():
  try:return socket.create_connection(('127.0.0.1',port),timeout=.2)
  except OSError:
   if player.poll() is not None:raise RuntimeError('VLC exited; inspect isolated vlc.log')
   return None
 remote=wait_for(connect);remote.settimeout(.5)
 def command(text):
  remote.sendall((text+'\n').encode());time.sleep(.2)
  data=b''
  try:
   while True:
    part=remote.recv(4096)
    if not part:break
    data+=part
  except socket.timeout:pass
  return data.decode(errors='replace')
 command('get_length')
 for stage,full,expected in [('video-windowed',False,True),('video-fullscreen',True,False),('video-windowed-restored',False,True)]:
  if stage!='video-windowed':command('fullscreen '+('on' if full else 'off'))
  def video_window():
   candidates=windows(player.pid)
   if not candidates:return None
   hwnd,title,area=max(candidates,key=lambda x:x[2])
   if area<1000:return None
   rect,monitor,covers=geometry(hwnd)
   return (hwnd,title,rect,monitor) if covers==full else None
  hwnd,title,rect,monitor=wait_for(video_window)
  focus_owned(hwnd,player.pid)
  stable=0;deadline=time.monotonic()+5
  while stable<5 and time.monotonic()<deadline:
   time.sleep(.1);visible=bool(u.IsWindowVisible(overlay));stable=stable+1 if visible==expected else 0
  playback=command('get_time').strip()
  row=dict(Stage=stage,ExpectedVisible=expected,ActualVisible=visible,Pass=stable>=5,VlcVersion=version,ClientBounds=json.dumps(rect),MonitorBounds=json.dumps(monitor),VideoWindowTitle=title,PlaybackResponse=playback)
  rows.append(row);print(json.dumps(row),flush=True)
  if not row['Pass']:raise RuntimeError('Overlay visibility failed during actual VLC video '+stage)
 print('PASS: actual VLC windowed/fullscreen/windowed playback visibility. Not an exclusive-game or multi-monitor test.',flush=True)
finally:
 foreground=u.GetForegroundWindow();owned_foreground=bool((player and owner(foreground)==player.pid) or (overlay_process and owner(foreground)==overlay_process.pid))
 if remote:
  try:remote.sendall(b'quit\n');remote.close()
  except OSError:pass
 for process in [player,overlay_process]:
  if process and process.poll() is None:
   try:process.wait(timeout=2)
   except subprocess.TimeoutExpired:process.terminate();process.wait(timeout=3)
 u.SetCursorPos(pointer.x,pointer.y)
 if owned_foreground and u.IsWindow(previous) and u.IsWindowVisible(previous):u.SetForegroundWindow(previous)
 if dpi:u.SetThreadDpiAwarenessContext(dpi)
 output=pathlib.Path(workspace)/'verification/video-fullscreen.csv';output.parent.mkdir(exist_ok=True)
 if rows:
  with output.open('w',newline='',encoding='utf-8') as f:
   writer=csv.DictWriter(f,fieldnames=list(rows[0]));writer.writeheader();writer.writerows(rows)
'@ | Set-Content -LiteralPath $script -Encoding UTF8
python $script $workspace $run $VlcPath $version
if($LASTEXITCODE -ne 0){throw "Video fullscreen probe failed; inspect $run"}
