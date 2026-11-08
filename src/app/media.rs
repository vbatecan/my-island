//! Media-session UI binding and playback clock; the service owns WinRT work.
use super::*;

pub(super) fn wire(app: &Rc<RefCell<App>>, ui: &IslandWindow) {
    {
        let app = app.clone();
        ui.on_open_media(move || {
            if let Some(media) = app.borrow().media.clone() {
                media.refresh();
            }
            pages::open(&app, crate::navigation::Page::Media);
        });
    }
    if !verification::has_arg("--verify-lyrics")
        && !verification::has_arg("--verify-launcher")
        && !verification::has_arg("--verify-pages")
        && !verification::has_arg("--verify-size")
        && !verification::has_arg("--verify-layout")
        && !verification::has_arg("--verify-media-polish")
    {
        let weak = ui.as_weak();
        let artwork_cache = Arc::new(Mutex::new(artwork::ArtworkCache::default()));
        let callback_cache = artwork_cache.clone();
        match MediaService::new(move |snapshot| {
            let artwork_changed = callback_cache
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .changed(snapshot.artwork.as_deref());
            let _ = weak.upgrade_in_event_loop(move |ui| {
                apply_media(&ui, &snapshot, artwork_changed);
                ui.invoke_media_updated();
            });
        }) {
            Ok(service) => {
                let snapshot = service.snapshot();
                let artwork_changed = artwork_cache
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .changed(snapshot.artwork.as_deref());
                apply_media(ui, &snapshot, artwork_changed);
                app.borrow_mut().media = Some(Rc::new(service));
            }
            Err(e) => ui.set_service_status(e.into()),
        }
    }
    {
        let app = app.clone();
        ui.on_play_pause(move || command(&app, MediaService::play_pause));
    }
    {
        let app = app.clone();
        ui.on_previous(move || command(&app, MediaService::previous));
    }
    {
        let app = app.clone();
        ui.on_next(move || command(&app, MediaService::next));
    }
    {
        let app = app.clone();
        ui.on_seek(move |fraction| {
            let duration = app.borrow().media_position.1;
            command(&app, |service| service.seek(fraction as f64 * duration));
        });
    }
    {
        let app = app.clone();
        ui.on_media_updated(move || {
            let ui = app.borrow().ui.upgrade();
            if let Some(ui) = ui {
                app.borrow_mut().media_position = (
                    ui.get_position_seconds() as f64,
                    ui.get_duration_seconds() as f64,
                    Instant::now(),
                );
            }
            {
                let mut a = app.borrow_mut();
                let now = a.now();
                let media = a.settings.compact_now_playing
                    && a.ui
                        .upgrade()
                        .is_some_and(|ui| ui.get_media_available() && ui.get_playing());
                a.coordinator.set_media(media, now);
            }
            sync_media_clock(&app);
            sync_presentation(&app);
            let mut a = app.borrow_mut();
            if let Some(ui) = a.ui.upgrade() {
                let position = lyrics::position(&a, &ui);
                lyrics::update_position(&mut a, &ui, position);
            }
        });
        // Initialize from the already-read snapshot as well as future events.
        ui.invoke_media_updated();
    }
    {
        let app = app.clone();
        ui.on_choose_player(move |id| {
            command(&app, |service| service.select_player(id.to_string()));
        });
    }
}

fn command(app: &Rc<RefCell<App>>, action: impl FnOnce(&MediaService) -> Result<(), String>) {
    let service = app.borrow().media.clone();
    if let Some(service) = service
        && let Err(error) = action(&service)
    {
        status(app, &error);
    }
}

pub(super) fn apply_media(ui: &IslandWindow, snapshot: &MediaSnapshot, artwork_changed: bool) {
    let metadata_changed = ui.get_track_title() != snapshot.title.as_str()
        || ui.get_artist_name() != snapshot.artist.as_str()
        || ui.get_media_source() != snapshot.source.as_str()
        || artwork_changed;
    // Timeline events do not change the catalog. Preserve the existing model so
    // each playback tick avoids allocating rows and rebuilding the chooser.
    let current = ui.get_media_players();
    use slint::Model;
    let catalog_changed = current.row_count() != snapshot.players.len()
        || snapshot.players.iter().enumerate().any(|(index, player)| {
            current.row_data(index).is_none_or(|row| {
                row.id != player.id || row.label != player.name || row.playing != player.playing
            })
        });
    if catalog_changed {
        let players = snapshot
            .players
            .iter()
            .map(|p| crate::PlayerChoice {
                id: p.id.as_str().into(),
                label: p.name.as_str().into(),
                playing: p.playing,
            })
            .collect::<Vec<_>>();
        ui.set_media_players(slint::ModelRc::from(Rc::new(slint::VecModel::from(
            players,
        ))));
    }
    ui.set_player_label(
        snapshot
            .players
            .iter()
            .find(|p| p.id == snapshot.selected_player)
            .map(|p| p.name.as_str())
            .unwrap_or("Automatic")
            .into(),
    );
    ui.set_media_available(snapshot.available);
    ui.set_playing(snapshot.playing);
    ui.set_track_title(snapshot.title.as_str().into());
    ui.set_artist_name(snapshot.artist.as_str().into());
    ui.set_media_source(snapshot.source.as_str().into());
    ui.set_can_play_pause(snapshot.can_play_pause);
    ui.set_can_previous(snapshot.can_previous);
    ui.set_can_next(snapshot.can_next);
    ui.set_can_seek(snapshot.can_seek);
    ui.set_position_seconds(snapshot.position_seconds as f32);
    ui.set_duration_seconds(snapshot.duration_seconds as f32);
    ui.set_progress(if snapshot.duration_seconds > 0. {
        (snapshot.position_seconds / snapshot.duration_seconds).clamp(0., 1.) as f32
    } else {
        0.
    });
    ui.set_elapsed_label(time_label(snapshot.position_seconds).into());
    ui.set_duration_label(time_label(snapshot.duration_seconds).into());
    ui.set_service_status(snapshot.error.as_deref().unwrap_or("").into());
    if artwork_changed {
        if let Some(image) = snapshot.artwork.as_deref().and_then(artwork::decode) {
            ui.set_artwork(image);
            ui.set_has_artwork(true);
        } else {
            ui.set_artwork(slint::Image::default());
            ui.set_has_artwork(false);
        }
    }
    if metadata_changed && snapshot.available {
        ui.invoke_media_change();
    }
}

pub(super) fn sync_media_clock(app: &Rc<RefCell<App>>) {
    let (active, compact) = {
        let a = app.borrow();
        let compact = matches!(
            a.presentation.as_ref().map(|p| &p.content),
            Some(Content::CompactMedia)
        );
        (
            !a.hidden
                && (a.state == IslandState::Media || compact)
                && a.ui.upgrade().is_some_and(|ui| {
                    ui.get_media_available() && ui.get_playing() && ui.get_duration_seconds() > 0.
                }),
            compact,
        )
    };
    if !active {
        app.borrow().media_clock.stop();
        return;
    }
    if app.borrow().media_clock.running() && app.borrow().media_clock_compact == compact {
        return;
    }
    app.borrow().media_clock.stop();
    app.borrow_mut().media_clock_compact = compact;
    let weak = Rc::downgrade(app);
    app.borrow().media_clock.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(if compact { 1000 } else { 250 }),
        move || {
            if let Some(app) = weak.upgrade() {
                let mut a = app.borrow_mut();
                if let Some(ui) = a.ui.upgrade() {
                    let duration = a.media_position.1;
                    let position = lyrics::position(&a, &ui);
                    ui.set_progress(if duration > 0. {
                        (position / duration).clamp(0., 1.) as f32
                    } else {
                        0.
                    });
                    ui.set_elapsed_label(time_label(position).into());
                    if a.state == IslandState::Media {
                        lyrics::update_position(&mut a, &ui, position);
                    }
                }
            }
        },
    );
}
