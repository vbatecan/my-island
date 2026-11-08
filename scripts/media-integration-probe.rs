//! Standalone, silent Windows integration probe. Compile using a temporary Cargo
//! manifest with this file as [[bin]], windows0.62.2 features Foundation, Media,
//! Media_Control, Storage_Streams, Win32_Foundation, Win32_System_WinRT,
//! Win32_UI_WindowsAndMessaging, Win32_UI_Shell. It publishes only a hidden
//! fixture's SMTC metadata; it never creates audio or commands another player.
#[path = "../src/services/media.rs"]
mod media;
use media::MediaService;
use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};
use windows::{
    Foundation::{TimeSpan, TypedEventHandler},
    Media::{
        MediaPlaybackStatus, MediaPlaybackType, SystemMediaTransportControls as Smtc,
        SystemMediaTransportControlsButton as Button,
        SystemMediaTransportControlsTimelineProperties as Timeline,
    },
    Win32::{
        Foundation::HWND,
        System::WinRT::{
            ISystemMediaTransportControlsInterop, RO_INIT_MULTITHREADED, RoInitialize,
            RoUninitialize,
        },
        UI::{Shell::SetCurrentProcessExplicitAppUserModelID, WindowsAndMessaging::*},
    },
    core::{HSTRING, w},
};

fn pump(duration: Duration) {
    let start = Instant::now();
    while start.elapsed() < duration {
        // SAFETY: MSG is an initialized output buffer. Only this fixture's thread
        // queue is pumped; no user input is synthesized and no window is activated.
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn timeline(position: i64) -> windows::core::Result<Timeline> {
    let value = Timeline::new()?;
    value.SetStartTime(TimeSpan { Duration: 0 })?;
    value.SetEndTime(TimeSpan {
        Duration: 180 * 10_000_000,
    })?;
    value.SetMinSeekTime(TimeSpan { Duration: 0 })?;
    value.SetMaxSeekTime(TimeSpan {
        Duration: 180 * 10_000_000,
    })?;
    value.SetPosition(TimeSpan { Duration: position })?;
    Ok(value)
}
fn metadata(controls: &Smtc, title: &str) -> windows::core::Result<()> {
    let display = controls.DisplayUpdater()?;
    display.SetType(MediaPlaybackType::Music)?;
    let music = display.MusicProperties()?;
    music.SetTitle(&HSTRING::from(title))?;
    music.SetArtist(&HSTRING::from("Integration Fixture"))?;
    display.Update()
}
struct Fixture {
    hwnd: HWND,
    controls: Smtc,
    button: i64,
    seek: i64,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.controls.RemoveButtonPressed(self.button);
        let _ = self
            .controls
            .RemovePlaybackPositionChangeRequested(self.seek);
        let _ = self.controls.SetPlaybackStatus(MediaPlaybackStatus::Closed);
        let _ = self.controls.SetIsEnabled(false);
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
fn create_fixture(count: Arc<AtomicU32>) -> Result<Fixture, Box<dyn std::error::Error>> {
    // SAFETY: an offscreen, hidden native window belongs exclusively to this test
    // thread. The interop call receives its live HWND; Fixture tears it down.
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            w!("STATIC"),
            w!("Still media integration fixture"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            None,
            None,
        )?
    };
    let interop: ISystemMediaTransportControlsInterop = windows::core::factory::<Smtc, _>()?;
    let controls: Smtc = unsafe { interop.GetForWindow(hwnd)? };
    controls.SetIsEnabled(true)?;
    controls.SetIsPlayEnabled(true)?;
    controls.SetIsPauseEnabled(true)?;
    controls.SetIsNextEnabled(true)?;
    controls.SetIsPreviousEnabled(true)?;
    metadata(&controls, "Still integration fixture")?;
    controls.UpdateTimelineProperties(&timeline(25 * 10_000_000)?)?;
    let target = controls.clone();
    let observed = count.clone();
    let button = controls.ButtonPressed(&TypedEventHandler::new(
        move |_,
              args: windows::core::Ref<
            windows::Media::SystemMediaTransportControlsButtonPressedEventArgs,
        >| {
            let args = args
                .as_ref()
                .ok_or_else(windows::core::Error::from_thread)?;
            match args.Button()? {
                Button::Play => target.SetPlaybackStatus(MediaPlaybackStatus::Playing)?,
                Button::Pause => target.SetPlaybackStatus(MediaPlaybackStatus::Paused)?,
                Button::Next => metadata(&target, "Still fixture next")?,
                Button::Previous => metadata(&target, "Still fixture previous")?,
                _ => {}
            }
            observed.fetch_add(1, Ordering::Relaxed);
            Ok(())
        },
    ))?;
    let target = controls.clone();
    let seek = controls.PlaybackPositionChangeRequested(&TypedEventHandler::new(
        move |_,
              args: windows::core::Ref<
            windows::Media::PlaybackPositionChangeRequestedEventArgs,
        >| {
            let position = args
                .as_ref()
                .ok_or_else(windows::core::Error::from_thread)?
                .RequestedPlaybackPosition()?;
            target.UpdateTimelineProperties(&timeline(position.Duration)?)?;
            count.fetch_add(1, Ordering::Relaxed);
            Ok(())
        },
    ))?;
    controls.SetPlaybackStatus(MediaPlaybackStatus::Playing)?;
    pump(Duration::from_millis(300));
    controls.SetPlaybackStatus(MediaPlaybackStatus::Paused)?;
    Ok(Fixture {
        hwnd,
        controls,
        button,
        seek,
    })
}
fn wait_for(service: &MediaService, condition: impl Fn(&media::MediaSnapshot) -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(4) {
        if condition(&service.snapshot()) {
            return true;
        }
        pump(Duration::from_millis(25));
    }
    false
}
fn assert_fixture(service: &MediaService) -> Result<(), Box<dyn std::error::Error>> {
    let s = service.snapshot();
    if s.available && s.artist == "Integration Fixture" {
        Ok(())
    } else {
        Err("Fixture is no longer current; refusing to control another player".into())
    }
}
struct ChildFixture(std::process::Child);
impl Drop for ChildFixture {
    fn drop(&mut self) {
        // Closing the input pipe requests graceful teardown of the hidden
        // fixture, including its SMTC tokens and HWND. No other process is used.
        drop(self.0.stdin.take());
        let _ = self.0.wait();
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let secondary = std::env::args().any(|arg| arg == "--secondary-fixture");
    unsafe {
        RoInitialize(RO_INIT_MULTITHREADED)?;
        SetCurrentProcessExplicitAppUserModelID(if secondary {
            w!("StillIsland.IntegrationFixture.Second")
        } else {
            w!("StillIsland.IntegrationFixture")
        })?;
    }
    let buttons = Arc::new(AtomicU32::new(0));
    let fixture = create_fixture(buttons.clone())?;
    if secondary {
        metadata(&fixture.controls, "Still second fixture")?;
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let _ = std::io::stdin().read_line(&mut line);
            let _ = sender.send(());
        });
        let started = Instant::now();
        while receiver.try_recv().is_err() && started.elapsed() < Duration::from_secs(30) {
            pump(Duration::from_millis(25));
        }
        drop(fixture);
        unsafe { RoUninitialize() };
        return Ok(());
    }
    let service = MediaService::new(|_| {})?;
    let exposed = wait_for(&service, |s| {
        s.players
            .iter()
            .any(|p| p.name.contains("IntegrationFixture") || p.name.contains("media-probe"))
    });
    if exposed {
        let id = service
            .snapshot()
            .players
            .iter()
            .find(|p| p.name.contains("IntegrationFixture") || p.name.contains("media-probe"))
            .unwrap()
            .id
            .clone();
        service.select_player(id)?;
    }
    if !wait_for(&service, |s| {
        s.available && s.artist == "Integration Fixture"
    }) {
        println!(
            "UNVERIFIED: Windows did not expose the silent hidden fixture as current GSMTC session; no player commands sent. available={} error={:?}",
            service.snapshot().available,
            service.snapshot().error
        );
    } else {
        assert_fixture(&service)?;
        service.play_pause()?;
        assert!(
            wait_for(&service, |s| s.playing),
            "play acknowledgement not observed"
        );
        println!("PASS play");
        assert_fixture(&service)?;
        service.play_pause()?;
        assert!(
            wait_for(&service, |s| !s.playing),
            "pause acknowledgement not observed"
        );
        println!("PASS pause");
        if service.snapshot().can_next {
            assert_fixture(&service)?;
            service.next()?;
            assert!(wait_for(&service, |s| s.title == "Still fixture next"));
            println!("PASS next and metadata update");
        }
        if service.snapshot().can_previous {
            assert_fixture(&service)?;
            service.previous()?;
            assert!(wait_for(&service, |s| s.title == "Still fixture previous"));
            println!("PASS previous and metadata update");
        }
        if service.snapshot().can_seek {
            assert_fixture(&service)?;
            service.seek(75.0)?;
            assert!(wait_for(&service, |s| (s.position_seconds - 75.0).abs() < 0.1));
            println!("PASS seek and timeline update");
        } else {
            println!("UNVERIFIED: publisher did not expose seek capability");
        }
        println!(
            "fixture command callbacks={}",
            buttons.load(Ordering::Relaxed)
        );
        let first_id = service.snapshot().selected_player;
        let previous_ids: Vec<_> = service
            .snapshot()
            .players
            .iter()
            .map(|p| p.id.clone())
            .collect();
        let second = ChildFixture(
            std::process::Command::new(std::env::current_exe()?)
                .arg("--secondary-fixture")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .spawn()?,
        );
        let second_exposed = wait_for(&service, |s| {
            s.players.iter().any(|p| {
                !previous_ids.contains(&p.id)
                    && (p.name.contains("IntegrationFixture") || p.name.contains("media-probe"))
            })
        });
        if second_exposed {
            let second_id = service
                .snapshot()
                .players
                .iter()
                .find(|p| {
                    !previous_ids.contains(&p.id)
                        && (p.name.contains("IntegrationFixture") || p.name.contains("media-probe"))
                })
                .unwrap()
                .id
                .clone();
            service.select_player(second_id.clone())?;
            assert!(
                wait_for(&service, |s| s.selected_player == second_id
                    && s.title == "Still second fixture"),
                "Second selection failed: {:?}",
                service.snapshot()
            );
            println!("PASS explicit second-player selection and metadata");
            service.select_player(first_id.clone())?;
            assert!(wait_for(&service, |s| s.selected_player == first_id
                && s.title == "Still fixture previous"));
            println!("PASS switching back preserves original session identity");
            service.select_player(second_id.clone())?;
            assert!(wait_for(&service, |s| s.selected_player == second_id));
            fixture
                .controls
                .SetPlaybackStatus(MediaPlaybackStatus::Playing)?;
            assert!(wait_for(&service, |s| s
                .players
                .iter()
                .any(|p| p.id == first_id && p.playing)));
            assert_eq!(service.snapshot().selected_player, second_id);
            service.select_player(String::new())?;
            assert!(wait_for(&service, |s| s.selected_player.is_empty()
                && s.playing
                && s.title == "Still fixture previous"));
            println!("PASS background playback updates and Automatic follows playing session");
            service.select_player(second_id.clone())?;
            assert!(wait_for(&service, |s| s.selected_player == second_id));
            drop(second);
            assert!(wait_for(&service, |s| s.selected_player.is_empty()
                && !s.players.iter().any(|p| p.id == second_id)));
            println!("PASS removed selection falls back to Automatic");
        } else {
            println!(
                "UNVERIFIED: Windows did not expose the separate fixture process; multi-player selection not exposed"
            );
            drop(second);
        }
    }
    drop(fixture);
    pump(Duration::from_millis(200));
    service.refresh();
    pump(Duration::from_millis(200));
    println!(
        "after fixture closed available={}",
        service.snapshot().available
    );
    drop(service);
    // SAFETY: all owned WinRT interfaces and worker threads are gone.
    unsafe { RoUninitialize() };
    Ok(())
}
