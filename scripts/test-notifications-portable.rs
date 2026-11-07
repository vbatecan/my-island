//! Portable safety probe: never requests permission, reads notifications, opens
//! apps, or installs a package. The temporary runner compiles against this source.
#[allow(dead_code)]
#[path = "../src/services/notifications.rs"]
mod notifications;
use notifications::{AccessStatus, NotificationEvent, NotificationService};
use std::sync::{Arc, Mutex};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if notifications::has_package_identity()? {
        return Err(
            "This probe must run unpackaged; refusing any packaged notification access".into(),
        );
    }
    let events = Arc::new(Mutex::new(Vec::new()));
    let output = events.clone();
    let service = NotificationService::new(move |event| {
        output.lock().unwrap().push(event);
    })?;
    assert_eq!(service.access_status(), AccessStatus::UnavailablePortable);
    service.set_enabled(true);
    service.refresh_access();
    assert_eq!(service.access_status(), AccessStatus::UnavailablePortable);
    assert!(events.lock().unwrap().iter().all(|event| matches!(
        event,
        NotificationEvent::Access(AccessStatus::UnavailablePortable)
    )));
    assert!(!notifications::can_open_source(""));
    assert!(!notifications::can_open_source(
        "DefinitelyNotARegisteredStillIslandTestAumid!Missing"
    ));
    assert!(!notifications::can_open_source(
        "C:\\Windows\\System32\\notepad.exe"
    ));
    assert!(notifications::open_source_app("").is_err());
    service.set_enabled(false);
    drop(service);
    println!(
        "PASS portable access gate: UnavailablePortable, no content events, no permission request, no app activation"
    );
    Ok(())
}
