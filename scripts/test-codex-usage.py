"""Offline smoke of the actual packaged plugin and disposable stdio server.

No real Codex process, account, auth file, GUI, or network service is used.
"""
import argparse
import ctypes
import json
import os
import pathlib
import queue
import shutil
import signal
import subprocess
import threading
import time
import uuid

ROOT = pathlib.Path(__file__).resolve().parents[1]
SECRET = "SECRET-FIXTURE-MUST-NOT-LEAK"


def nodes(message):
    def walk(node):
        yield node
        for child in node.get("children", []):
            yield from walk(child)
    return list(walk(message["root"])) if message.get("type") == "view" else []


def quota(message):
    return [n["value"] for n in nodes(message) if n.get("kind") == "progress"]


def text(message):
    return " ".join(n.get("text", "") for n in nodes(message))


def alive(pid):
    if os.name != "nt":
        try:
            os.kill(pid, 0)
            return True
        except ProcessLookupError:
            return False
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.OpenProcess.restype = ctypes.c_void_p
    kernel.OpenProcess.argtypes = [ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
    kernel.WaitForSingleObject.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
    kernel.CloseHandle.argtypes = [ctypes.c_void_p]
    handle = kernel.OpenProcess(0x100000, False, pid)
    if not handle:
        return False
    try:
        return kernel.WaitForSingleObject(handle, 0) == 258
    finally:
        kernel.CloseHandle(handle)


def terminate_fixture(pid, directory):
    """Failure-only cleanup, verifying executable identity before termination."""
    if os.name != 'nt':
        executable = pathlib.Path(f'/proc/{pid}/exe')
        if executable.exists() and executable.resolve().parent == directory:
            os.kill(pid, signal.SIGTERM)
        return
    kernel = ctypes.WinDLL('kernel32', use_last_error=True)
    kernel.OpenProcess.restype = ctypes.c_void_p
    kernel.OpenProcess.argtypes = [ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
    kernel.QueryFullProcessImageNameW.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_wchar_p, ctypes.POINTER(ctypes.c_ulong)]
    kernel.TerminateProcess.argtypes = [ctypes.c_void_p, ctypes.c_uint]
    kernel.CloseHandle.argtypes = [ctypes.c_void_p]
    handle = kernel.OpenProcess(0x1001, False, pid)
    if handle:
        try:
            path = ctypes.create_unicode_buffer(32768)
            size = ctypes.c_ulong(len(path))
            if kernel.QueryFullProcessImageNameW(handle, 0, path, ctypes.byref(size)):
                executable = pathlib.Path(path.value).resolve()
                if executable.parent == directory and executable.name in {'fixture.exe', 'fixture-second.exe'}:
                    kernel.TerminateProcess(handle, 1)
        finally:
            kernel.CloseHandle(handle)


class Probe:
    def __init__(self, binary, fixture, directory):
        self.directory = directory
        directory.mkdir()
        self.mode = directory / "mode.txt"
        self.log = directory / "methods.txt"
        self.set_mode("normal")
        env = os.environ.copy()
        env["ISLAND_CODEX_FIXTURE_MODE"] = str(self.mode)
        env["ISLAND_CODEX_FIXTURE_LOG"] = str(self.log)
        # A mistaken fallback cannot find the user's real executable/profile.
        env["PATH"] = str(directory)
        env["LOCALAPPDATA"] = str(directory)
        env["CODEX_HOME"] = str(directory / "no-real-account")
        self.stderr = open(directory / "stderr.txt", "w", encoding="utf-8")
        self.process = subprocess.Popen([str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=self.stderr, text=True, encoding="utf-8", env=env)
        self.messages = []
        self.pending = queue.Queue(maxsize=256)
        self.reader_error = None
        def read():
            try:
                for line in self.process.stdout:
                    self.pending.put(json.loads(line), timeout=1)
            except Exception as error:
                self.reader_error = error
        self.reader = threading.Thread(target=read, daemon=True)
        self.reader.start()
        try:
            started = time.monotonic()
            self.send("init", protocol_version=1, config={"executable": str(fixture)})
            assert self.until(lambda m: m.get("type") == "ready", 1.5)["protocol_version"] == 1
            assert time.monotonic() - started < 1.5
            self.until(lambda m: m.get("type") == "view")
            assert not self.records(), "Init unexpectedly spawned Codex"
        except Exception:
            self.process.kill()
            self.process.wait(timeout=3)
            for pid, method in self.records():
                if method == 'START' and alive(int(pid)):
                    terminate_fixture(int(pid), directory.parent)
            self.stderr.close()
            raise

    def set_mode(self, mode):
        self.mode.write_text(mode, encoding="utf-8")

    def send(self, kind, **fields):
        self.process.stdin.write(json.dumps(dict(type=kind, **fields)) + "\n")
        self.process.stdin.flush()

    def invoke(self, action):
        self.send("invoke", action_id=action, value=None)

    def until(self, predicate, timeout=3):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            assert self.reader_error is None, self.reader_error
            try:
                message = self.pending.get(timeout=min(.1, max(.001, end-time.monotonic())))
            except queue.Empty:
                continue
            assert SECRET not in json.dumps(message), "Remote secret leaked in plugin output"
            self.messages.append(message)
            if predicate(message):
                return message
        raise AssertionError(f"Plugin response timed out; recent messages: {self.messages[-3:]}")

    def records(self):
        return [line.split(maxsplit=1) for line in self.log.read_text().splitlines()] if self.log.exists() else []

    def wait_start(self, count=1):
        end = time.monotonic()+2
        while time.monotonic()<end:
            starts=[int(pid) for pid, method in self.records() if method=='START']
            if len(starts)>=count:
                return starts[-1]
            time.sleep(.02)
        raise AssertionError("Fixture did not start")

    def close(self, deadline=2):
        elapsed=0
        if self.process.poll() is None:
            started=time.monotonic()
            self.send("shutdown")
            self.process.wait(timeout=deadline)
            elapsed=time.monotonic()-started
            assert elapsed < deadline
        assert self.process.returncode == 0
        self.reader.join(timeout=1)
        assert not self.reader.is_alive(), 'Plugin stdout remained open after exit'
        assert self.reader_error is None, self.reader_error
        while not self.pending.empty():
            message=self.pending.get_nowait()
            assert SECRET not in json.dumps(message), 'Remote secret leaked in trailing output'
            self.messages.append(message)
        self.stderr.close()
        assert SECRET not in (self.directory/'stderr.txt').read_text(encoding='utf-8')
        # Each session's disposal must reap every child, including hung servers.
        assert all(not alive(int(pid)) for pid, method in self.records() if method=='START'), self.records()
        assert {method for _,method in self.records()} <= {'START','initialize','initialized','account/rateLimits/read'}
        return elapsed


def run(binary_dir):
    binary = binary_dir / ('still-island-plugin-codex-usage.exe' if os.name=='nt' else 'still-island-plugin-codex-usage')
    assert binary.is_file(), binary
    directory=ROOT/'target'/'codex-usage-smoke'/uuid.uuid4().hex
    directory.mkdir(parents=True)
    fixture=directory/('fixture.exe' if os.name=='nt' else 'fixture')
    subprocess.run(['rustc','--edition=2024',str(ROOT/'scripts/fixtures/codex-app-server.rs'),'-o',str(fixture)],check=True,timeout=60)
    second=directory/('fixture-second.exe' if os.name=='nt' else 'fixture-second')
    shutil.copyfile(fixture,second)
    probes=[]
    def start(name):
        probe=Probe(binary,fixture,directory/name);probes.append(probe);return probe
    try:
        p=start('normal')
        p.send('view_request',view_id='main')
        for _ in range(10):p.invoke('refresh')
        started=time.monotonic()
        for _ in range(10):
            ack=p.until(lambda m:m.get('type')=='action_result' and m.get('action_id')=='refresh',1.5)
            assert ack['success']
        assert time.monotonic()-started<1.5
        view=p.until(lambda m:quota(m)==[.255,.63])
        assert '5-hour limit' in text(view) and '7-day limit' in text(view)
        assert view['compact']['progress']==.63 and '63%' in view['compact']['title']
        assert '7-day limit' in view['compact']['body']
        assert sum(method=='START' for _,method in p.records())==1,p.records()
        p.invoke('show');p.until(lambda m:m.get('type')=='activity')
        p.set_mode('error');p.invoke('refresh')
        stale=p.until(lambda m:'Refresh failed' in text(m))
        assert quota(stale)==[.255,.63]
        assert stale['compact']['progress']==.63 and stale['compact']['body'].startswith('Last reported · ')
        p.invoke('unknown');assert not p.until(lambda m:m.get('action_id')=='unknown')['success']
        p.invoke('show');p.until(lambda m:m.get('type')=='activity')
        p.send('configure',config={'executable':str(directory/'missing.exe')})
        p.until(lambda m:m.get('type')=='view' and not quota(m))
        p.invoke('refresh')
        invalid=p.until(lambda m:'Reading Codex' not in text(m) and m.get('type')=='view')
        assert not quota(invalid) and 'Cannot start the selected Codex executable' in text(invalid)
        assert invalid.get('compact') is None
        p.close()
        print('PASS Codex usage: lazy Ready, noise handshake, two normalized quotas, 10 responsive deduplicated refreshes, preview, stale snapshot, sanitized errors, unknown action, invalid executable',flush=True)

        p=start('generation');p.set_mode('slow');p.send('view_request',view_id='main');old=p.wait_start()
        p.set_mode('normal');p.send('configure',config={'executable':str(second)})
        p.until(lambda m:m.get('type')=='view' and 'Reading Codex' not in text(m))
        assert not alive(old),'Configuration cancellation left old child alive'
        assert all(not quota(m) for m in p.messages)
        p.invoke('refresh');p.until(lambda m:quota(m)==[.255,.63]);p.close()
        print('PASS Codex usage: executable change cancels pending generation and discards old quota',flush=True)

        p=start('oversized');p.set_mode('oversized');p.send('view_request',view_id='main')
        p.until(lambda m:'output budget' in text(m));assert not any(quota(m) for m in p.messages);p.close()
        print('PASS Codex usage: oversized child output rejected without invented quota',flush=True)

        p=start('deadline');p.set_mode('slow');p.send('view_request',view_id='main');p.wait_start()
        started=time.monotonic();p.invoke('unknown')
        p.until(lambda m:m.get('action_id')=='unknown',1)
        p.until(lambda m:'timed out' in text(m),9)
        deadline_elapsed=time.monotonic()-started
        assert 7 <= deadline_elapsed <= 9
        p.set_mode('normal');p.invoke('refresh');p.until(lambda m:quota(m)==[.255,.63]);p.close()
        print(f'PASS Codex usage: deadline observed at {deadline_elapsed:.3f}s, IPC responsiveness during hung child, recovery',flush=True)

        p=start('shutdown');p.set_mode('slow');p.send('view_request',view_id='main');p.wait_start();shutdown_elapsed=p.close()
        print(f'PASS Codex usage: shutdown {shutdown_elapsed:.3f}s reaps hung child; only initialize/initialized/account rate-limit read methods observed',flush=True)
    finally:
        for p in probes:
            if p.process.poll() is None:
                try:p.close()
                except Exception:
                    p.process.kill();p.process.wait(timeout=3)
            for pid,method in p.records():
                if method=='START' and alive(int(pid)):
                    terminate_fixture(int(pid),directory)


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('--binary-dir',type=pathlib.Path,default=ROOT/'target'/'release')
    args=parser.parse_args()
    run(args.binary_dir.resolve())
