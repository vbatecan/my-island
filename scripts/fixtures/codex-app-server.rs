//! Offline child-process fixture; never shipped as a plugin or reads real auth.
use std::{
    env,
    fs::OpenOptions,
    io::{self, BufRead, Write},
    thread,
    time::Duration,
};

fn record(method: &str) {
    let path = env::var_os("ISLAND_CODEX_FIXTURE_LOG").expect("fixture log required");
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    writeln!(log, "{} {method}", std::process::id()).unwrap();
}
fn reply(text: &str) {
    let mut output = io::stdout().lock();
    writeln!(output, "{text}").unwrap();
    output.flush().unwrap();
}
fn main() {
    let mode = std::fs::read_to_string(
        env::var_os("ISLAND_CODEX_FIXTURE_MODE").expect("fixture mode required"),
    )
    .unwrap();
    record("START");
    let args: Vec<_> = env::args().skip(1).collect();
    assert!(
        args.windows(3)
            .any(|a| a == ["app-server", "--listen", "stdio://"])
    );
    assert!(args.iter().any(|a| a == "analytics.enabled=false"));
    assert_eq!(env::var("OTEL_SDK_DISABLED").unwrap(), "true");
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        let method = line
            .split("\"method\"")
            .nth(1)
            .unwrap()
            .split(':')
            .nth(1)
            .unwrap()
            .trim()
            .trim_start_matches('"')
            .split('"')
            .next()
            .unwrap();
        record(method);
        match method {
            "initialize" => {
                reply(r#"{"method":"fixture/notification","params":{"ignored":true}}"#);
                if mode.trim() == "slow" {
                    thread::sleep(Duration::from_secs(15));
                }
                if mode.trim() == "oversized" {
                    reply(&"x".repeat(64 * 1024));
                    continue;
                }
                thread::sleep(Duration::from_millis(200));
                reply(r#"{"id":1,"result":{}}"#);
            }
            "initialized" => {}
            "account/rateLimits/read" => {
                if mode.trim() == "error" {
                    reply(
                        r#"{"id":2,"error":{"code":401,"message":"SECRET-FIXTURE-MUST-NOT-LEAK"}}"#,
                    );
                } else {
                    reply(
                        r#"{"id":2,"result":{"rateLimits":{"limitId":"codex","primary":{"usedPercent":25.5,"windowDurationMins":300},"secondary":{"usedPercent":63,"windowDurationMins":10080}}}}"#,
                    );
                }
            }
            _ => panic!("unexpected fixture method"),
        }
    }
}
