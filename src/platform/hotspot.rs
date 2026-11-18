//! Windows Mobile hotspot control using the system's access-point configuration.
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use windows::Networking::{
    Connectivity::NetworkInformation,
    NetworkOperators::{
        NetworkOperatorTetheringManager, TetheringCapability, TetheringOperationStatus,
        TetheringOperationalState,
    },
};
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum State {
    On,
    Off,
    Changing,
    NoConnection,
    MissingCapability,
    Unavailable,
}

fn manager() -> Result<NetworkOperatorTetheringManager, State> {
    let profile =
        NetworkInformation::GetInternetConnectionProfile().map_err(|_| State::NoConnection)?;
    match NetworkOperatorTetheringManager::GetTetheringCapabilityFromConnectionProfile(&profile) {
        Ok(TetheringCapability::Enabled) => {
            NetworkOperatorTetheringManager::CreateFromConnectionProfile(&profile)
                .map_err(|_| State::Unavailable)
        }
        Ok(TetheringCapability::DisabledBySystemCapability) => Err(State::MissingCapability),
        _ => Err(State::Unavailable),
    }
}

pub async fn request(enabled: Option<bool>, current: Arc<AtomicBool>) -> Result<State, String> {
    let (send, reply) = futures_channel::oneshot::channel();
    std::thread::Builder::new()
        .name("island-hotspot".into())
        .spawn(move || {
            // SAFETY: the apartment and all WinRT interfaces stay on this worker.
            if let Err(error) = unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
                let _ = send.send(Err(error.to_string()));
                return;
            }
            let result = apply(enabled, &current);
            let _ = send.send(result);
            // SAFETY: balances initialization on this thread.
            unsafe { RoUninitialize() };
        })
        .map_err(|error| error.to_string())?;
    reply.await.map_err(|error| error.to_string())?
}

fn state(manager: &NetworkOperatorTetheringManager) -> State {
    match manager.TetheringOperationalState() {
        Ok(TetheringOperationalState::On) => State::On,
        Ok(TetheringOperationalState::Off) => State::Off,
        Ok(TetheringOperationalState::InTransition) => State::Changing,
        _ => State::Unavailable,
    }
}

fn apply(enabled: Option<bool>, current: &AtomicBool) -> Result<State, String> {
    let manager = match manager() {
        Ok(manager) => manager,
        Err(state) => return Ok(state),
    };
    let observed = state(&manager);
    if !current.load(Ordering::Acquire) {
        return Ok(observed);
    }
    let operation = match (enabled, observed) {
        (Some(true), State::Off) => manager.StartTetheringAsync(),
        (Some(false), State::On) => manager.StopTetheringAsync(),
        _ => return Ok(observed),
    };
    let status = operation
        .and_then(|operation| operation.join())
        .and_then(|result| result.Status())
        .map_err(|error| error.to_string())?;
    if status != TetheringOperationStatus::Success {
        return Err(format!("Tethering operation returned status {}", status.0));
    }
    Ok(state(&manager))
}
