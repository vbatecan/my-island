//! Owned rendering fixture. Exercises the pointer-entry hide handler without
//! moving the desktop pointer or sending keyboard input to another application.
use super::*;

fn check(ok: bool, label: &str) {
    record(&format!(
        "hover-morph {} {label}",
        if ok { "PASS" } else { "FAILED" }
    ));
}

fn capture(app: &Rc<RefCell<App>>, name: &str) {
    if verification::has_arg("--perf-hover") {
        return;
    }
    let a = app.borrow();
    if let Some(ui) = a.ui.upgrade() {
        match crate::verification::snapshot(&ui, name) {
            Ok(path) => record(&format!("hover-morph snapshot {}", path.display())),
            Err(error) => record(&format!("hover-morph FAILED snapshot {error}")),
        }
    }
    record(&format!("hover-morph {}", platform::diagnostic_snapshot()));
}

pub(super) fn setup(app: &Rc<RefCell<App>>) {
    let mut settings = app.borrow().settings.clone();
    settings.hide_on_hover = true;
    settings.appearance = if verification::has_arg("--appearance=acrylic") {
        Appearance::Acrylic
    } else {
        Appearance::Solid
    };
    settings.form_factor = crate::shape::FormFactor::Notch;
    settings.animations = true;
    settings.follow_reduced_motion = false;
    settings.animation_speed = if verification::has_arg("--perf-hover") {
        0.7
    } else {
        1.
    };
    settings.offset = 12.;
    apply_settings(app, settings, false);

    for (step, at) in [800, 900, 1800, 1900, 2800, 3600, 3700, 4600, 4700, 5600]
        .into_iter()
        .enumerate()
    {
        let weak = Rc::downgrade(app);
        slint::Timer::single_shot(Duration::from_millis(at), move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            match step {
                0 | 5 => {
                    if app.borrow().settings.appearance == Appearance::Acrylic
                        && !verification::has_arg("--cpu-acrylic")
                    {
                        check(
                            app.borrow()
                                .ui
                                .upgrade()
                                .is_some_and(|ui| ui.get_gpu_material()),
                            "Acrylic is GPU rendered",
                        );
                    }
                    capture(
                        &app,
                        if step == 0 {
                            "hover-notch-before"
                        } else {
                            "hover-island-before"
                        },
                    );
                    app.borrow_mut().visibility.protected_visit = false;
                    check(
                        interaction::request_hover_hide(&app),
                        "entry handler accepts hide",
                    );
                }
                1 | 6 => {
                    {
                        let a = app.borrow();
                        let g = a.geometry(a.state);
                        check(
                            a.springs[1].position > 0.1 && a.springs[1].position < g.height,
                            "bottom edge contracts during hide",
                        );
                        check(
                            a.springs[0].position < g.width,
                            "shoulders narrow during hide",
                        );
                        check(a.slide.position < 0., "morph moves toward top edge");
                        check(
                            a.slide.position.abs() < g.height,
                            "hide is not a full-height slide",
                        );
                    }
                    capture(
                        &app,
                        if step == 1 {
                            "hover-notch-hiding"
                        } else {
                            "hover-island-hiding"
                        },
                    );
                }
                2 | 7 => {
                    {
                        let a = app.borrow();
                        check(
                            a.hidden && !a.timer.running(),
                            "hidden animation settles and stops",
                        );
                        check(a.springs[1].position <= 0.12, "shell retracts completely");
                    }
                    app.borrow_mut().visibility.reveal_hover();
                    sync_presentation(&app);
                }
                3 | 8 => {
                    {
                        let a = app.borrow();
                        check(
                            !a.hidden && a.springs[1].position > 0.1,
                            "reveal grows the shell back from the edge",
                        );
                        check(a.visibility.protected_visit, "reveal protects interaction");
                    }
                    capture(
                        &app,
                        if step == 3 {
                            "hover-notch-revealing"
                        } else {
                            "hover-island-revealing"
                        },
                    );
                }
                4 => {
                    capture(&app, "hover-notch-restored");
                    let mut settings = app.borrow().settings.clone();
                    settings.form_factor = crate::shape::FormFactor::Island;
                    apply_settings(&app, settings, false);
                }
                9 => {
                    capture(&app, "hover-island-restored");
                    let mut settings = app.borrow().settings.clone();
                    settings.animations = false;
                    apply_settings(&app, settings, false);
                    app.borrow_mut().visibility.protected_visit = false;
                    check(
                        interaction::request_hover_hide(&app),
                        "motion-disabled hide accepted",
                    );
                    check(
                        app.borrow().hidden,
                        "motion-disabled hide applies immediately",
                    );
                    app.borrow_mut().visibility.reveal_hover();
                    sync_presentation(&app);
                    check(
                        !app.borrow().hidden,
                        "motion-disabled reveal applies immediately",
                    );
                    record("hover-morph complete");
                    let _ = slint::quit_event_loop();
                }
                _ => unreachable!(),
            }
        });
    }
}
