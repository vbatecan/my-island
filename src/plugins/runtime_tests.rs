//! Real standalone process fixtures. Never launches a user's application,
//! touches their configured plugins, opens a URI, or accesses the network.
use super::*;
use std::{
    io::Write,
    os::windows::process::CommandExt,
    sync::{OnceLock, mpsc},
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
};

const FIXTURE: &str = r###"
use std::{io::{self,BufRead,Write},process::{Command,Stdio},os::windows::process::CommandExt};
fn emit(message:&str){println!("{message}");io::stdout().flush().unwrap();}
fn main(){
 if std::env::args().any(|s|s=="--descendant"){loop{std::thread::park();}}
 for line in io::stdin().lock().lines(){let line=line.unwrap();
  if line.contains("\"type\":\"init\""){
   emit(r#"{"type":"ready","protocol_version":1}"#);
   // This unsolicited response must never reach the UI or open anything.
   emit(r#"{"type":"action_result","action_id":"spawn","success":true,"message":"unsolicited","open_uri":"https://invalid.example"}"#);
  }else if line.contains("\"type\":\"view_request\""){
   emit(&format!(r#"{{"type":"view","view_id":"main","revision":1,"root":{{"id":"root","kind":"text","text":"{}"}}}}"#,std::process::id()));
  }else if line.contains("\"action_id\":\"crash\""){std::process::exit(42);
  }else if line.contains("\"action_id\":\"bad\""){emit("{not-json");
  }else if line.contains("\"action_id\":\"hang\""){loop{std::thread::park();}
  }else if line.contains("\"action_id\":\"spawn\""){
   let child=Command::new(std::env::current_exe().unwrap()).arg("--descendant").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).creation_flags(0x08000000).spawn().unwrap();
   emit(&format!(r#"{{"type":"action_result","action_id":"spawn","success":true,"message":"{}"}}"#,child.id()));
  }else if line.contains("\"action_id\":\"flood\""){
   for _ in 0..50{emit(r#"{"type":"activity","id":"burst","title":"Update","body":"Status changed"}"#);}
   emit(r#"{"type":"action_result","action_id":"flood","success":true,"message":"done"}"#);
  }else if line.contains("\"action_id\":\"echo\""){
   emit(r#"{"type":"action_result","action_id":"echo","success":true,"message":"acknowledged"}"#);
  }else if line.contains("\"type\":\"shutdown\""){
   std::fs::write(std::env::current_exe().unwrap().with_file_name(format!("shutdown-{}.txt",std::process::id())),"graceful").unwrap();break;
  }
 }
}
"###;

fn fixture() -> PathBuf {
    static FILE: OnceLock<PathBuf> = OnceLock::new();
    FILE.get_or_init(|| {
        let root = std::env::temp_dir().join(format!(
            "still-island-runtime-fixture-{}-{}",
            std::process::id(),
            SystemClock::stamp()
        ));
        fs::create_dir(&root).unwrap();
        let source = root.join("fixture.rs");
        let binary = root.join("fixture.exe");
        fs::write(&source, FIXTURE).unwrap();
        let output = std::process::Command::new("rustc")
            .args(["--edition=2024", "-O"])
            .arg(&source)
            .arg("-o")
            .arg(&binary)
            .creation_flags(0x08000000)
            .output()
            .expect("rustc must be installed for the native fixture");
        assert!(
            output.status.success(),
            "Fixture failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        binary
    })
    .clone()
}
struct SystemClock;
impl SystemClock {
    fn stamp() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }
}
fn archive(root: &Path) -> PathBuf {
    let path = root.join("fixture.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    zip.start_file("manifest.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    let actions =
        ["echo", "spawn", "bad", "crash", "hang", "flood"].map(|id| json!({"id":id,"label":id}));
    zip.write_all(serde_json::to_string(&json!({"id":"org.stillisland.fixture","name":"Runtime fixture","version":"1.0.0","api_version":1,"entrypoint":"fixture.exe","actions":actions,"views":[{"id":"main","label":"Fixture"}]})).unwrap().as_bytes()).unwrap();
    zip.start_file("fixture.exe", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(&fs::read(fixture()).unwrap()).unwrap();
    zip.finish().unwrap();
    path
}
fn wait(receiver: &mpsc::Receiver<PluginEvent>, predicate: impl Fn(&Value) -> bool) -> Value {
    let until = Instant::now() + Duration::from_secs(15);
    loop {
        let event = receiver
            .recv_timeout(until.saturating_duration_since(Instant::now()))
            .expect("Expected fixture event within deadline");
        assert_ne!(
            event.message.get("message").and_then(Value::as_str),
            Some("unsolicited"),
            "Unsolicited URI result escaped correlation"
        );
        if predicate(&event.message) {
            return event.message;
        }
    }
}
fn process_handle(pid: u32) -> HANDLE {
    // SAFETY: read-only query access for the exact fixture PID returned by its
    // protocol response. The test owns and closes the resulting kernel handle.
    unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .expect("Fixture process should be alive")
}
fn assert_exited(handle: HANDLE) {
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        let mut code = 259;
        let result = unsafe { GetExitCodeProcess(handle, &mut code) };
        if result.is_ok() && code != 259 {
            break;
        }
        assert!(
            Instant::now() < until,
            "Fixture process survived job cleanup"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = unsafe { CloseHandle(handle) };
}

#[test]
fn real_plugin_handshake_actions_failures_timeout_and_tree_cleanup() {
    let root = std::env::temp_dir().join(format!(
        "still-island-runtime-host-{}-{}",
        std::process::id(),
        SystemClock::stamp()
    ));
    fs::create_dir(&root).unwrap();
    let (tx, rx) = mpsc::channel();
    let activities = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let event_count = activities.clone();
    let host = PluginHost::with_root(root.join("storage"), move |e| {
        if e.message["type"] == "activity" {
            event_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        let _ = tx.send(e);
    })
    .unwrap();
    host.import(&archive(&root)).unwrap();
    let id = "org.stillisland.fixture";
    assert!(!host.list()[0].enabled);
    host.set_enabled(id, true).unwrap();
    host.request_view(id, "main").unwrap();
    let view = wait(&rx, |v| v["type"] == "view");
    let pid = view["root"]["text"].as_str().unwrap().parse().unwrap();
    let process = process_handle(pid);
    host.invoke(id, "echo", Value::Null).unwrap();
    wait(&rx, |v| {
        v["type"] == "action_result" && v["action_id"] == "echo" && v["success"] == true
    });
    host.invoke(id, "flood", Value::Null).unwrap();
    wait(&rx, |v| {
        v["type"] == "action_result" && v["action_id"] == "flood"
    });
    assert_eq!(
        activities.load(std::sync::atomic::Ordering::Relaxed),
        1,
        "Activity bursts must be throttled before GUI delivery"
    );
    host.invoke(id, "spawn", Value::Null).unwrap();
    let result = wait(&rx, |v| {
        v["type"] == "action_result" && v["action_id"] == "spawn"
    });
    let descendant = process_handle(result["message"].as_str().unwrap().parse().unwrap());
    host.set_enabled(id, false).unwrap();
    assert_exited(process);
    assert_exited(descendant);
    assert!(
        host.list()[0]
            .directory
            .join(format!("shutdown-{pid}.txt"))
            .is_file(),
        "Healthy plugins must receive Shutdown before job cleanup"
    );
    for action in ["crash", "bad"] {
        host.set_enabled(id, true).unwrap();
        host.request_view(id, "main").unwrap();
        wait(&rx, |v| v["type"] == "view");
        host.invoke(id, action, Value::Null).unwrap();
        wait(&rx, |v| {
            v["type"] == "host_status" && v["status"] == "Unavailable"
        });
        assert!(host.list()[0].enabled);
        assert!(
            host.request_view(id, "main").is_err(),
            "Crashes must require an explicit restart"
        );
        host.set_enabled(id, false).unwrap();
    }
    host.set_enabled(id, true).unwrap();
    host.request_view(id, "main").unwrap();
    let view = wait(&rx, |v| v["type"] == "view");
    let process = process_handle(view["root"]["text"].as_str().unwrap().parse().unwrap());
    host.invoke(id, "hang", Value::Null).unwrap();
    let failure = wait(&rx, |v| {
        v["type"] == "host_status" && v["status"] == "Unavailable"
    });
    assert!(failure["error"].as_str().unwrap().contains("timed out"));
    assert_exited(process);
    host.restart(id).unwrap();
    host.request_view(id, "main").unwrap();
    let view = wait(&rx, |v| v["type"] == "view");
    let live = process_handle(view["root"]["text"].as_str().unwrap().parse().unwrap());
    let directory = host.list()[0].directory.clone();
    let started = Instant::now();
    host.remove(id).unwrap();
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_exited(live);
    assert!(
        !directory.exists(),
        "Removing a running plugin must delete mapped executables after shutdown"
    );
    assert!(host.list().is_empty());
    // Artifacts are intentionally retained only inside this unique test-owned
    // temporary directory for diagnosis; no user's plugin directory is touched.
}
