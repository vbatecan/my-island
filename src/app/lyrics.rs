//! Lyrics orchestration stays on the UI thread; lookup and parsing belong to the service.
use super::*;
use crate::animation::lyrics::{LineChange, LyricCursor};
use crate::services::lyrics::{LyricsContent, LyricsEvent, LyricsService, LyricsStatus, TrackKey};

#[derive(Default)]
pub(super) struct Controller {
    service: Option<LyricsService>,
    incoming: Arc<Mutex<Option<LyricsEvent>>>,
    scheduled: Arc<AtomicBool>,
    generation: u64,
    track: Option<TrackKey>,
    content: Option<Arc<LyricsContent>>,
    loading: bool,
    released: bool,
    cursor: LyricCursor,
}

pub(super) fn wire(app: &Rc<RefCell<App>>, ui: &IslandWindow) {
    let weak = Rc::downgrade(app);
    ui.on_lyrics_updated(move || {
        if let Some(app) = weak.upgrade() {
            receive(&app);
        }
    });
    let weak = Rc::downgrade(app);
    ui.on_lyrics_retry(move || {
        if let Some(app) = weak.upgrade() {
            sync(&app, true);
        }
    });
    let weak = Rc::downgrade(app);
    ui.on_lyrics_enable(move || {
        if let Some(app) = weak.upgrade() {
            let mut settings = app.borrow().settings.clone();
            settings.online_lyrics = true;
            apply_settings(&app, settings, true);
        }
    });
}

fn clear(ui: &IslandWindow, message: &str) {
    ui.invoke_move_lyrics(-1., false);
    ui.set_lyrics_rows(slint::ModelRc::default());
    ui.set_lyrics_loading(false);
    ui.set_lyrics_available(false);
    ui.set_lyrics_synced(false);
    ui.set_lyrics_status(message.into());
    ui.set_lyrics_previous("".into());
    ui.set_lyrics_current("".into());
    ui.set_lyrics_next("".into());
    ui.set_lyrics_plain("".into());
}

pub(super) fn sync(app: &Rc<RefCell<App>>, force: bool) {
    let mut a = app.borrow_mut();
    let Some(ui) = a.ui.upgrade() else {
        return;
    };
    ui.set_lyrics_enabled(a.settings.online_lyrics);
    let visible = a.state == IslandState::Media && !a.hidden && !a.settings_visible;
    let motion_enabled = a.animations_enabled() && visible;
    ui.set_lyrics_motion_enabled(motion_enabled);
    if !motion_enabled {
        ui.invoke_stop_lyrics();
    }
    // Deterministic GUI fixtures never disclose the user's media metadata.
    if a.verification {
        return;
    }
    if !a.settings.online_lyrics {
        a.lyrics.release();
        clear(&ui, "Online lyrics are off");
        return;
    }
    a.lyrics.released = false;
    if a.state != IslandState::Media || a.hidden || a.settings_visible {
        if a.lyrics.loading {
            a.lyrics.cancel();
            a.lyrics.track = None;
        }
        return;
    }
    let track = a.media.as_ref().and_then(|service| {
        service.with_snapshot(|snapshot| {
            snapshot
                .available
                .then(|| {
                    TrackKey::new(
                        &snapshot.title,
                        &snapshot.artist,
                        &snapshot.album,
                        snapshot.duration_seconds,
                    )
                })
                .flatten()
        })
    });
    let Some(track) = track else {
        a.lyrics.cancel();
        a.lyrics.track = None;
        a.lyrics.content = None;
        clear(
            &ui,
            if ui.get_media_available() {
                "Waiting for track details"
            } else {
                "Play a track to find lyrics"
            },
        );
        return;
    };
    if !force && a.lyrics.track.as_ref() == Some(&track) {
        return;
    }
    a.lyrics.cancel();
    a.lyrics.track = Some(track.clone());
    a.lyrics.content = None;
    a.lyrics.cursor.reset();
    clear(&ui, "Finding lyrics on LRCLIB…");
    ui.set_lyrics_loading(true);
    a.lyrics.loading = true;
    if a.lyrics.service.is_none() {
        let incoming = a.lyrics.incoming.clone();
        let scheduled = a.lyrics.scheduled.clone();
        let weak = ui.as_weak();
        match LyricsService::new(move |event| {
            if let Ok(mut slot) = incoming.lock() {
                *slot = Some(event);
            }
            if !scheduled.swap(true, Ordering::AcqRel) {
                let scheduled = scheduled.clone();
                let failed = scheduled.clone();
                if weak
                    .upgrade_in_event_loop(move |ui| {
                        scheduled.store(false, Ordering::Release);
                        ui.invoke_lyrics_updated();
                    })
                    .is_err()
                {
                    failed.store(false, Ordering::Release);
                }
            }
        }) {
            Ok(service) => a.lyrics.service = Some(service),
            Err(_) => {
                a.lyrics.loading = false;
                clear(&ui, "Lyrics lookup unavailable. Try again.");
                return;
            }
        }
    }
    let generation = a.lyrics.generation;
    if let Some(service) = &a.lyrics.service {
        if force {
            service.request_refresh(generation, track);
        } else {
            service.request(generation, track);
        }
    }
}

impl Controller {
    /// Release retained documents on disable. Keep the sleeping worker's tiny
    /// provider cooldown state, so re-enabling cannot bypass Retry-After.
    fn release(&mut self) {
        if self.released {
            return;
        }
        self.released = true;
        self.cancel();
        self.track = None;
        self.content = None;
        self.cursor.reset();
        if let Ok(mut incoming) = self.incoming.lock() {
            incoming.take();
        }
        if let Some(service) = &self.service {
            service.clear_cache();
        }
    }

    fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.loading = false;
        if let Some(service) = &self.service {
            service.cancel();
        }
    }
}

fn receive(app: &Rc<RefCell<App>>) {
    let event = app
        .borrow()
        .lyrics
        .incoming
        .lock()
        .ok()
        .and_then(|mut slot| slot.take());
    let Some(event) = event else {
        return;
    };
    let mut a = app.borrow_mut();
    let Some(ui) = a.ui.upgrade() else {
        return;
    };
    if event.generation != a.lyrics.generation
        || a.lyrics.track.as_ref() != Some(&event.track)
        || !a.settings.online_lyrics
        || a.state != IslandState::Media
        || a.hidden
    {
        return;
    }
    a.lyrics.loading = false;
    a.lyrics.cursor.reset();
    match event.status {
        LyricsStatus::Found(content) => {
            clear(&ui, "");
            match content.as_ref() {
                LyricsContent::Synced(_) => {
                    ui.set_lyrics_available(true);
                    ui.set_lyrics_synced(true);
                }
                LyricsContent::Plain(lines) => {
                    ui.set_lyrics_available(true);
                    ui.set_lyrics_plain(lines.join("\n").into());
                }
                LyricsContent::Instrumental => clear(&ui, "Instrumental · no lyrics"),
            }
            a.lyrics.content = Some(content);
            let position = position(&a, &ui);
            update_position(&mut a, &ui, position);
        }
        LyricsStatus::NotFound => clear(&ui, "No matching lyrics on LRCLIB"),
        LyricsStatus::Unavailable(_) => clear(&ui, "Couldn’t reach LRCLIB. Try again."),
    }
}

pub(super) fn position(a: &App, ui: &IslandWindow) -> f64 {
    let (position, duration, received) = a.media_position;
    (position
        + if ui.get_playing() {
            received.elapsed().as_secs_f64()
        } else {
            0.
        })
    .clamp(0., duration.max(0.))
}

/// Called by the existing playback clock, including pause/seek snapshot updates.
/// No independent animation or polling timer is installed for lyrics.
pub(super) fn update_position(a: &mut App, ui: &IslandWindow, position: f64) {
    let Some(content) = &a.lyrics.content else {
        return;
    };
    let LyricsContent::Synced(lines) = content.as_ref() else {
        return;
    };
    let index = content.active_line(position);
    let motion = a.animations_enabled()
        && !a.hidden
        && !a.settings_visible
        && a.state == IslandState::Media
        && ui.get_playing();
    let change = a.lyrics.cursor.update(index, position, motion);
    if change == LineChange::Unchanged {
        return;
    }
    let text = |index: Option<usize>| {
        index
            .and_then(|i| lines.get(i))
            .map_or("", |line| line.text.as_str())
    };
    ui.set_lyrics_previous(text(index.and_then(|i| i.checked_sub(1))).into());
    ui.set_lyrics_current(text(index).into());
    ui.set_lyrics_next(text(Some(index.map_or(0, |i| i + 1))).into());
    let cursor = index.map_or(-1, |index| index as i32);
    // Absolute line identities stay in fixed modulo slots. An adjacent step only
    // replaces offscreen slots, so visible text follows one continuous cursor.
    let mut rows = vec![crate::LyricRow::default(); 7];
    for line in cursor - 3..=cursor + 3 {
        rows[line.rem_euclid(7) as usize] = crate::LyricRow {
            line_index: line,
            text: usize::try_from(line)
                .ok()
                .and_then(|i| lines.get(i))
                .map_or("", |line| line.text.as_str())
                .into(),
        };
    }
    ui.set_lyrics_rows(slint::ModelRc::from(Rc::new(slint::VecModel::from(rows))));
    let animate = change == LineChange::Advance
        && (ui.get_lyrics_visual_cursor() - cursor as f32).abs() <= 1.75;
    ui.invoke_move_lyrics(cursor as f32, animate);
}

pub(super) fn verification_begin(app: &Rc<RefCell<App>>, track: TrackKey) -> u64 {
    let mut a = app.borrow_mut();
    assert!(
        a.verification,
        "synthetic lyrics require explicit verification mode"
    );
    a.lyrics.cancel();
    a.lyrics.track = Some(track);
    a.lyrics.content = None;
    a.lyrics.cursor.reset();
    a.lyrics.loading = true;
    a.lyrics.generation
}

pub(super) fn verification_receive(app: &Rc<RefCell<App>>, event: LyricsEvent) {
    assert!(
        app.borrow().verification,
        "synthetic lyrics require explicit verification mode"
    );
    *app.borrow()
        .lyrics
        .incoming
        .lock()
        .expect("verification inbox") = Some(event);
    receive(app);
}

#[cfg(test)]
mod resource_tests {
    use super::*;

    #[test]
    fn disabling_releases_documents_and_preserves_provider_lifetime() {
        for _ in 0..100 {
            let mut controller = Controller::default();
            let document = Arc::new(LyricsContent::Plain(vec!["Original fixture".into()]));
            let document_weak = Arc::downgrade(&document);
            let track = TrackKey::new("Fixture", "Still Island", "", 120.).unwrap();
            controller.content = Some(document.clone());
            controller.track = Some(track.clone());
            *controller.incoming.lock().unwrap() = Some(LyricsEvent {
                generation: 0,
                track,
                status: LyricsStatus::Found(document),
            });
            let worker_resource = Arc::new(());
            let worker_weak = Arc::downgrade(&worker_resource);
            controller.service = Some(
                LyricsService::new(move |_| {
                    let _ = &worker_resource;
                })
                .unwrap(),
            );
            controller.release();
            assert!(document_weak.upgrade().is_none());
            assert!(controller.service.is_some());
            assert!(controller.track.is_none());
            assert!(worker_weak.upgrade().is_some());
            drop(controller);
            assert!(worker_weak.upgrade().is_none());
        }
    }
}
