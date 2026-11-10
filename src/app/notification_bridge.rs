//! Event-driven notification queue delivery and user actions.
use super::*;

pub(super) fn wire(app: &Rc<RefCell<App>>, ui: &IslandWindow) {
    {
        let app = app.clone();
        ui.on_expand_notification(move || {
            let mut a = app.borrow_mut();
            let now = a.now();
            a.coordinator.expand_notification(now);
            drop(a);
            sync_presentation(&app);
        });
    }
    {
        let app = app.clone();
        ui.on_hide_notification(move || {
            let mut a = app.borrow_mut();
            let now = a.now();
            a.coordinator.hide_notification(now);
            drop(a);
            sync_presentation(&app);
        });
    }
    {
        let app = app.clone();
        ui.on_open_source(move || {
            let record =
                app.borrow()
                    .presentation
                    .as_ref()
                    .and_then(|view| match &view.content {
                        Content::Notification(record, _)
                        | Content::ExpandedNotification(record) => Some(record.clone()),
                        _ => None,
                    });
            if let Some(record) = record {
                if let Err(e) = notifications::open_source_app(&record.app_id) {
                    status(&app, &e);
                } else {
                    transition(&app, IslandState::Idle);
                }
            }
        });
    }

    {
        let app = app.clone();
        ui.on_notifications_updated(move || {
            let events = {
                let a = app.borrow();
                let mut queue = a
                    .notification_inbox
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                a.notification_scheduled.store(false, Ordering::Release);
                queue.drain(..).collect::<Vec<_>>()
            };
            for event in events {
                let mut a = app.borrow_mut();
                let now = a.now();
                match event {
                    NotificationEvent::Access(access) => {
                        a.notification_access = access.clone();
                        if access != AccessStatus::Allowed {
                            a.coordinator.clear_notifications(now);
                        }
                    }
                    NotificationEvent::Removed(id) => {
                        a.coordinator.remove(id, now);
                        if matches!(&a.visibility.preview, Some(Content::Notification(record, _)) if record.id == id) {
                            a.visibility.preview = None;
                            a.visibility.until = None;
                            a.visibility.remaining = None;
                        }
                    },
                    NotificationEvent::Added(record) | NotificationEvent::Updated(record) => {
                        if !a.observed_apps.iter().any(|(id, _)| id == &record.app_id) {
                            a.observed_apps
                                .push((record.app_id.clone(), record.app_name.clone()));
                        }
                        if a.settings.notifications_enabled
                            && !a.settings.notifications_paused
                            && !a.settings.blocked_apps.contains(&record.app_id)
                            && platform::notification_allowed()
                        {
                            a.coordinator.push(record, now);
                        }
                    }
                }
            }
            sync_presentation(&app);
            refresh_preferences(&app);
        });
    }
    let (incoming, scheduled) = {
        let a = app.borrow();
        (
            a.notification_inbox.clone(),
            a.notification_scheduled.clone(),
        )
    };
    let weak = ui.as_weak();
    match NotificationService::new(move |event| {
        let mut queue = incoming.lock().unwrap_or_else(|e| e.into_inner());
        if queue.len() >= 128 {
            queue.pop_front();
        }
        queue.push_back(event);
        drop(queue);
        if !scheduled.swap(true, Ordering::AcqRel)
            && weak
                .upgrade_in_event_loop(|ui| ui.invoke_notifications_updated())
                .is_err()
        {
            scheduled.store(false, Ordering::Release);
        }
    }) {
        Ok(service) => {
            let mut a = app.borrow_mut();
            a.notification_access = service.access_status();
            service
                .set_enabled(a.settings.notifications_enabled && !a.settings.notifications_paused);
            a.notifications = Some(Rc::new(service));
        }
        Err(e) => app.borrow_mut().notification_access = AccessStatus::Error(e),
    }
    refresh_preferences(app);
}
