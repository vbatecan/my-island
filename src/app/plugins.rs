//! Native plugin UI binding. Processes and package ownership live in crate::plugins.
use super::*;
use crate::plugins::{PluginEvent, PluginHost};
use crate::{
    PluginActionEntry, PluginConfigEntry, PluginListEntry, PluginManagerWindow, PluginVisualNode,
};
use serde_json::{Value, json};
use slint::{Model, ModelRc, VecModel};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};
use still_island_plugin_protocol::{
    Accent, CompactSummary, ConfigKind, Node, NodeKind, PluginMessage,
};
#[derive(Default)]
struct UiState {
    selected: String,
    selected_view: String,
    roots: HashMap<(String, String), Node>,
    summaries: HashMap<(String, String), CompactSummary>,
    config: Value,
    invoked: HashSet<(String, String)>,
    rendered: Option<(String, String)>,
    config_rendered: String,
    panel_visible: bool,
    manager_open: bool,
}
fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::from(Rc::new(VecModel::from(items)))
}
fn color(accent: Accent) -> slint::Color {
    match accent {
        Accent::Green => slint::Color::from_rgb_u8(110, 218, 147),
        Accent::Amber => slint::Color::from_rgb_u8(238, 200, 112),
        Accent::Red => slint::Color::from_rgb_u8(238, 124, 124),
        Accent::Blue => slint::Color::from_rgb_u8(133, 187, 242),
        Accent::Neutral => slint::Color::from_rgb_u8(232, 232, 239),
    }
}

fn apply_summary(
    ui: &IslandWindow,
    host: &PluginHost,
    state: &RefCell<UiState>,
    id: &str,
    view: &str,
) {
    let summary = state
        .borrow()
        .summaries
        .get(&(id.into(), view.into()))
        .cloned();
    let summary = summary.unwrap_or_else(|| CompactSummary {
        title: host
            .list()
            .iter()
            .find(|p| p.manifest.id == id)
            .map(|p| p.manifest.name.clone())
            .unwrap_or_else(|| "Integration unavailable".into()),
        body: "Hover to open · no reported summary yet".into(),
        progress: None,
        accent: Accent::Neutral,
    });
    ui.set_plugin_title(summary.title.into());
    ui.set_plugin_body(summary.body.into());
    ui.set_plugin_has_progress(summary.progress.is_some());
    ui.set_plugin_progress(summary.progress.unwrap_or(0.) as f32);
    ui.set_plugin_accent(color(summary.accent));
}
fn text_style(node: &Node) -> (f32, i32) {
    use still_island_plugin_protocol::TextRole;
    match node.text_role {
        TextRole::Body => (12., 400),
        TextRole::Caption => (10., 400),
        TextRole::Title => (14., 600),
        TextRole::Metric => (28., 600),
    }
}
fn row_child_width(node: &Node, width: f32, child: &Node) -> f32 {
    let available =
        (width - 2. * node.padding - node.spacing * node.children.len().saturating_sub(1) as f32)
            .max(0.);
    let weights = node
        .children
        .iter()
        .map(|c| c.flex.max(1.))
        .sum::<f32>()
        .max(1.);
    available * child.flex.max(1.) / weights
}
fn natural_height(node: &Node, width: f32) -> f32 {
    let p = node.padding * 2.;
    match node.kind {
        NodeKind::Row => {
            node.children
                .iter()
                .map(|n| natural_height(n, row_child_width(node, width, n)))
                .fold(0., f32::max)
                + p
        }
        NodeKind::Column | NodeKind::List => {
            node.children
                .iter()
                .map(|n| natural_height(n, width - p))
                .sum::<f32>()
                + node.spacing * node.children.len().saturating_sub(1) as f32
                + p
        }
        NodeKind::Progress => 12.,
        NodeKind::Text => {
            let (size, _) = text_style(node);
            let lines = (node.text.chars().count() as f32 * size * 0.54 / width.max(12.))
                .ceil()
                .clamp(1., 2.);
            lines * (size * 1.35).ceil() + 3.
        }
        NodeKind::Image => 48.,
        _ => 30.,
    }
    .max(1.)
}
fn flatten(root: &Node, directory: &Path, width: f32) -> (Vec<PluginVisualNode>, f32) {
    fn visit(n: &Node, dir: &Path, x: f32, y: f32, w: f32, out: &mut Vec<PluginVisualNode>) {
        let p = n.padding;
        if matches!(n.kind, NodeKind::Row | NodeKind::Column | NodeKind::List) {
            let mut at = 0.;
            for child in &n.children {
                if n.kind == NodeKind::Row {
                    let cw = row_child_width(n, w, child);
                    visit(child, dir, x + p + at, y + p, cw, out);
                    at += cw + n.spacing;
                } else {
                    visit(child, dir, x + p, y + p + at, (w - 2. * p).max(0.), out);
                    at += natural_height(child, w - 2. * p) + n.spacing;
                }
            }
        } else {
            let image = if n.kind == NodeKind::Image && !n.asset.is_empty() {
                slint::Image::load_from_path(&dir.join(&n.asset)).unwrap_or_default()
            } else {
                slint::Image::default()
            };
            out.push(PluginVisualNode {
                id: n.id.as_str().into(),
                kind: format!("{:?}", n.kind).to_lowercase().into(),
                text: n.text.as_str().into(),
                action: n.action.as_str().into(),
                x,
                y,
                width: w,
                height: natural_height(n, w),
                value: n.value as f32,
                checked: n.checked,
                image,
                accent: if n.kind == NodeKind::Text
                    && n.text_role == still_island_plugin_protocol::TextRole::Caption
                    && n.accent == Accent::Neutral
                {
                    slint::Color::from_rgb_u8(156, 156, 169)
                } else {
                    color(n.accent)
                },
                font_size: text_style(n).0,
                font_weight: text_style(n).1,
            });
        }
    }
    let mut nodes = Vec::new();
    visit(root, directory, 0., 0., width, &mut nodes);
    (nodes, natural_height(root, width))
}
fn host(app: &Rc<RefCell<App>>) -> Option<Arc<PluginHost>> {
    app.borrow().plugin_host.clone()
}
fn feedback(window: &slint::Weak<PluginManagerWindow>, result: Result<(), String>) {
    if let Some(window) = window.upgrade() {
        window.set_status(result.err().unwrap_or_default().into());
    }
}
fn show_manager(window: &PluginManagerWindow) {
    let _ = window.show();
    let weak = window.as_weak();
    let _ = slint::spawn_local(async move {
        if let Some(window) = weak.upgrade() {
            use slint::winit_030::{
                WinitWindowAccessor,
                winit::{platform::windows::WindowExtWindows, window::WindowLevel},
            };
            if let Ok(native) = window.window().winit_window().await {
                native.set_skip_taskbar(false);
                native.set_window_level(WindowLevel::Normal);
                native.focus_window();
            }
        }
    });
}
fn refresh(
    app: &Rc<RefCell<App>>,
    ui: &IslandWindow,
    window: &PluginManagerWindow,
    state: &Rc<RefCell<UiState>>,
) {
    let Some(host) = host(app) else {
        return;
    };
    let installed = host.list();
    let pinned = host.pinned();
    let rows = installed
        .iter()
        .map(|p| PluginListEntry {
            id: p.manifest.id.as_str().into(),
            label: p.manifest.name.as_str().into(),
            status: p.status.as_str().into(),
            enabled: p.enabled,
            error: p.error.as_deref().unwrap_or("").into(),
        })
        .collect::<Vec<_>>();
    window.set_plugins(model(rows.clone()));
    ui.set_plugin_list(model(rows));
    let all_actions = installed
        .iter()
        .flat_map(|p| {
            p.manifest.actions.iter().map(|a| PluginActionEntry {
                plugin_id: p.manifest.id.as_str().into(),
                id: a.id.as_str().into(),
                label: format!("{} · {}", p.manifest.name, a.label).into(),
                pinned: pinned.contains(&(p.manifest.id.clone(), a.id.clone())),
            })
        })
        .collect::<Vec<_>>();
    ui.set_plugin_actions(model(
        all_actions.iter().filter(|a| a.pinned).cloned().collect(),
    ));
    let selected = state.borrow().selected.clone();
    window.set_selected_id(selected.as_str().into());
    if let Some(plugin) = installed.iter().find(|p| p.manifest.id == selected) {
        window.set_selected_label(
            format!("{} · v{}", plugin.manifest.name, plugin.manifest.version).into(),
        );
        if !ui.get_compact_plugin() {
            ui.set_plugin_title(plugin.manifest.name.as_str().into());
        }
        if let Some(error) = &plugin.error {
            let detail = host
                .recent_logs(&plugin.manifest.id)
                .last()
                .cloned()
                .unwrap_or_default();
            window.set_status(
                if detail.is_empty() {
                    error.clone()
                } else {
                    format!("{error}\n{detail}")
                }
                .into(),
            );
        }
        window.set_actions(model(
            all_actions
                .into_iter()
                .filter(|a| a.plugin_id.as_str() == selected)
                .collect(),
        ));
        if state.borrow().config_rendered != selected {
            let values = state.borrow().config.clone();
            window.set_fields(model(
                plugin
                    .manifest
                    .config
                    .iter()
                    .map(|f| {
                        let value = values.get(&f.id).unwrap_or(&f.default);
                        let text = value.as_str().map(str::to_owned).unwrap_or_else(|| {
                            if value.is_null() {
                                String::new()
                            } else {
                                value.to_string()
                            }
                        });
                        PluginConfigEntry {
                            id: f.id.as_str().into(),
                            label: f.label.as_str().into(),
                            kind: format!("{:?}", f.kind).to_lowercase().into(),
                            value: if f.kind == ConfigKind::Secret {
                                "".into()
                            } else {
                                text.as_str().into()
                            },
                            checked: value.as_bool().unwrap_or(false),
                            choices: model(f.choices.iter().map(|s| s.as_str().into()).collect()),
                            choice_index: f.choices.iter().position(|s| s == &text).unwrap_or(0)
                                as i32,
                        }
                    })
                    .collect(),
            ));
            state.borrow_mut().config_rendered = selected.clone();
        }
        let view_id = state.borrow().selected_view.clone();
        let root = state
            .borrow()
            .roots
            .get(&(selected.clone(), view_id.clone()))
            .cloned();
        if let Some(root) = root {
            let fingerprint = serde_json::to_string(&root).unwrap_or_default();
            let width = ui.get_plugin_layout_width().max(1.);
            let key = (
                selected.clone(),
                format!("{view_id}:{width}:{}:{fingerprint}", ui.get_text_scale()),
            );
            if state.borrow().rendered.as_ref() != Some(&key) {
                let (nodes, height) = flatten(&root, &plugin.directory, width);
                ui.set_plugin_nodes(model(nodes.clone()));
                ui.set_plugin_view_height(height);
                window.set_nodes(model(nodes));
                window.set_view_height(height);
                state.borrow_mut().rendered = Some(key);
            }
        }
    } else {
        window.set_selected_label("".into());
        window.set_fields(model(vec![]));
        window.set_actions(model(vec![]));
        window.set_nodes(model(vec![]));
        ui.set_plugin_title("Integrations".into());
    }
}
fn select(
    app: &Rc<RefCell<App>>,
    ui: &IslandWindow,
    window: &PluginManagerWindow,
    state: &Rc<RefCell<UiState>>,
    id: &str,
) {
    select_view(app, ui, window, state, id, None);
}

fn select_view(
    app: &Rc<RefCell<App>>,
    ui: &IslandWindow,
    window: &PluginManagerWindow,
    state: &Rc<RefCell<UiState>>,
    id: &str,
    requested_view: Option<&str>,
) {
    let Some(host) = host(app) else {
        return;
    };
    let previous = state.borrow().selected.clone();
    let _ = host.set_visible(&previous, false);
    state.borrow_mut().selected = id.to_owned();
    state.borrow_mut().rendered = None;
    state.borrow_mut().config_rendered.clear();
    state.borrow_mut().config = host.configuration(id).unwrap_or_else(|_| json!({}));
    ui.set_plugin_view_open(true);
    if let Some(p) = host.list().iter().find(|p| p.manifest.id == id)
        && let Some(view) = requested_view
            .and_then(|id| p.manifest.views.iter().find(|v| v.id == id))
            .or_else(|| p.manifest.views.first())
    {
        state.borrow_mut().selected_view = view.id.clone();
        if !state
            .borrow()
            .roots
            .contains_key(&(id.into(), view.id.clone()))
        {
            ui.set_plugin_nodes(model(vec![]));
        }
        feedback(&window.as_weak(), host.request_view(id, &view.id));
        // request_view creates an on-demand process; visibility sent before
        // that creation would be lost and leave the active view at idle cadence.
        let _ = host.set_visible(id, true);
    }
    refresh(app, ui, window, state);
}
fn invoke(
    app: &Rc<RefCell<App>>,
    state: &Rc<RefCell<UiState>>,
    window: &slint::Weak<PluginManagerWindow>,
    plugin: &str,
    action: &str,
    text: &str,
) {
    let Some(host) = host(app) else {
        return;
    };
    let value = {
        let state = state.borrow();
        action_value(
            state
                .roots
                .get(&(plugin.into(), state.selected_view.clone())),
            action,
            text,
        )
    };
    state
        .borrow_mut()
        .invoked
        .insert((plugin.into(), action.into()));
    feedback(window, host.invoke(plugin, action, value));
}
fn action_value(root: Option<&Node>, action: &str, text: &str) -> Value {
    fn is_toggle(n: &Node, action: &str) -> bool {
        n.action == action && n.kind == NodeKind::Toggle
            || n.children.iter().any(|c| is_toggle(c, action))
    }
    if root.is_some_and(|n| is_toggle(n, action)) {
        Value::Bool(text == "true")
    } else {
        Value::String(text.into())
    }
}
pub(super) fn wire(app: &Rc<RefCell<App>>, ui: &IslandWindow, prefs: &SettingsWindow) {
    let verifying = std::env::args().any(|arg| arg == "--verify-plugins");
    if app.borrow().verification && !verifying {
        return;
    }
    let events = Arc::new(Mutex::new(VecDeque::<PluginEvent>::new()));
    let inbox = events.clone();
    let weak = ui.as_weak();
    let callback = move |event| {
        let mut q = inbox.lock().unwrap_or_else(|e| e.into_inner());
        if q.len() == 128 {
            q.pop_front();
        }
        q.push_back(event);
        drop(q);
        let _ = weak.upgrade_in_event_loop(|ui| ui.invoke_plugins_updated());
    };
    let created = if verifying {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        PluginHost::with_root(
            std::env::current_dir()
                .unwrap_or_default()
                .join("target/plugin-ui-verification")
                .join(stamp.to_string()),
            callback,
        )
    } else {
        PluginHost::new(callback)
    };
    let plugin_host = match created {
        Ok(h) => h,
        Err(e) => {
            status(app, &e);
            return;
        }
    };
    app.borrow_mut().plugin_host = Some(plugin_host.clone());
    let window = match PluginManagerWindow::new() {
        Ok(w) => Rc::new(w),
        Err(e) => {
            status(app, &e.to_string());
            return;
        }
    };
    let state = Rc::new(RefCell::new(UiState::default()));
    {
        let weak = window.as_weak();
        window.window().on_close_requested(move || {
            if let Some(window) = weak.upgrade() {
                window.invoke_close_window();
            }
            slint::CloseRequestResponse::HideWindow
        });
    }
    {
        let w = window.clone();
        let state = state.clone();
        ui.on_manage_plugins(move || {
            state.borrow_mut().manager_open = true;
            show_manager(&w)
        });
    }
    {
        let w = window.clone();
        let state = state.clone();
        prefs.on_manage_plugins(move || {
            state.borrow_mut().manager_open = true;
            show_manager(&w)
        });
    }
    macro_rules! bind {
        ($target:expr,$method:ident,[$($name:ident:$typ:ty),*],$body:expr)=>{{
            let app=app.clone();let state=state.clone();let window=window.as_weak();let weak=ui.as_weak();
            let operation:fn(&Rc<RefCell<App>>,&Rc<RefCell<UiState>>,&PluginManagerWindow,&IslandWindow,$($typ),*)=$body;
            $target.$method(move |$($name),*|{if let (Some(window),Some(ui))=(window.upgrade(),weak.upgrade()){operation(&app,&state,&window,&ui,$($name),*);}});
        }};
    }
    bind!(ui, on_open_plugins, [], |app, state, window, ui| {
        state.borrow_mut().selected.clear();
        ui.set_plugin_view_open(false);
        refresh(app, ui, window, state);
        let mut a = app.borrow_mut();
        let now = a.now();
        a.coordinator.open(Panel::Plugins, now);
        drop(a);
        sync_presentation(app);
    });
    bind!(ui, on_plugin_home, [], |app, _, _, _| {
        pages::choose(app, crate::navigation::Page::Home);
    });
    bind!(ui,on_plugin_select,[id:slint::SharedString],|app,_,_,_,id|{
        if let Some(host) = host(app)
            && let Some(plugin) = host.list().iter().find(|p| p.manifest.id == id.as_str())
            && let Some(view) = plugin.manifest.views.first() {
            pages::choose(app, crate::navigation::Page::Plugin{id:id.into(),view:view.id.clone()});
        }
    });
    bind!(ui,on_plugin_page_select,[id:slint::SharedString,view:slint::SharedString],|app,state,window,ui,id,view|select_view(app,ui,window,state,&id,Some(&view)));
    bind!(ui,on_plugin_page_ensure,[id:slint::SharedString,view:slint::SharedString],|app,state,window,ui,id,view|{
        let differs = {let s=state.borrow(); s.selected != id.as_str() || s.selected_view != view.as_str() || !ui.get_plugin_view_open()};
        if differs {select_view(app,ui,window,state,&id,Some(&view));}
    });
    {
        // This callback can run while App is rendering under a mutable borrow.
        // It deliberately captures only plugin-owned state and the host handle.
        let host = plugin_host.clone();
        let state = state.clone();
        let weak = ui.as_weak();
        ui.on_plugin_summary_requested(move |id, view| {
            if let Some(ui) = weak.upgrade() {
                apply_summary(&ui, &host, &state, &id, &view);
            }
        });
    }
    bind!(ui,on_plugin_action,[id:slint::SharedString,action:slint::SharedString,value:slint::SharedString],|app,state,window,_,id,action,value|{let id=if id.is_empty(){state.borrow().selected.clone()}else{id.to_string()};invoke(app,state,&window.as_weak(),&id,&action,&value);});
    bind!(ui, on_plugin_edit, [], |app, state, window, ui| {
        state.borrow_mut().manager_open = true;
        refresh(app, ui, window, state);
        show_manager(window);
    });
    bind!(window, on_close_window, [], |app, state, window, _| {
        state.borrow_mut().manager_open = false;
        if let Some(h) = host(app) {
            let s = state.borrow();
            let _ = h.set_visible(&s.selected, s.panel_visible);
        }
        let _ = window.hide();
    });
    bind!(window,on_select_plugin,[id:slint::SharedString],|app,state,window,ui,id|select(app,ui,window,state,&id));
    bind!(window,on_enable_plugin,[id:slint::SharedString,enabled:bool],|app,state,window,ui,id,enabled|{if let Some(h)=host(app){feedback(&window.as_weak(),h.set_enabled(&id,enabled));if !enabled {let mut a=app.borrow_mut();let now=a.now();a.coordinator.clear_plugin(&id,now);drop(a);sync_presentation(app);}refresh(app,ui,window,state);}});
    bind!(window,on_restart_plugin,[id:slint::SharedString],|app,state,window,ui,id|{if let Some(h)=host(app){feedback(&window.as_weak(),h.restart(&id));refresh(app,ui,window,state);}});
    bind!(window,on_import_package,[path:slint::SharedString],|app,state,window,ui,path|{if let Some(h)=host(app){if window.get_busy(){return;}window.set_busy(true);window.set_status("Importing package…".into());let weak=ui.as_weak();let manager=window.as_weak();let _=std::thread::Builder::new().name("plugin-import".into()).spawn(move ||{let result=h.import(Path::new(path.as_str()));let _=weak.upgrade_in_event_loop(move |ui|{if let Some(window)=manager.upgrade(){window.set_busy(false);window.set_status(match result {Ok(m)=>format!("Imported {}. Enable only if you trust this program.",m.name),Err(e)=>e}.into());}ui.invoke_plugins_updated();});});}let _=state;});
    bind!(window,on_remove_plugin,[id:slint::SharedString],|app,_,window,ui,id|{if let Some(h)=host(app){if window.get_busy(){return;}window.set_busy(true);window.set_status("Removing plugin…".into());let weak=ui.as_weak();let manager=window.as_weak();let plugin=id.to_string();let _=std::thread::Builder::new().name("plugin-remove".into()).spawn(move ||{let result=h.remove(&plugin);let _=weak.upgrade_in_event_loop(move |ui|{if let Some(window)=manager.upgrade(){window.set_busy(false);feedback(&window.as_weak(),result);}ui.invoke_plugins_updated();});});let mut a=app.borrow_mut();let now=a.now();a.coordinator.clear_plugin(&id,now);drop(a);sync_presentation(app);}});
    bind!(window, on_browse, [], |_, _, window, _| {
        use slint::winit_030::WinitWindowAccessor;
        if window.get_busy() {
            return;
        }
        let mut owner = None;
        window.window().with_winit_window(|native| {
            if let Ok(handle) = native.window_handle()
                && let RawWindowHandle::Win32(handle) = handle.as_raw()
            {
                owner = Some(handle.hwnd.get());
            }
        });
        let Some(owner) = owner else {
            window.set_status("The integration window is not ready. Try Browse again.".into());
            return;
        };
        window.set_busy(true);
        window.set_status("Choose a plugin ZIP package…".into());
        let manager = window.as_weak();
        if let Err(error) = std::thread::Builder::new()
            .name("plugin-picker".into())
            .spawn(move || {
                let selected = platform::file_dialog::choose_plugin_package(owner);
                let _ = manager.upgrade_in_event_loop(move |window| {
                    window.set_busy(false);
                    match selected {
                        Ok(Some(path)) => {
                            if let Some(path) = path.to_str() {
                                window.set_package_path(path.into());
                                window.set_status(
                                    "Package selected. Choose Import to add it.".into(),
                                );
                            } else {
                                window.set_status(
                                    "Select a package with a Unicode file name.".into(),
                                );
                            }
                        }
                        Ok(None) => window.set_status("Selection cancelled.".into()),
                        Err(error) => window.set_status(error.into()),
                    }
                });
            })
        {
            window.set_busy(false);
            window.set_status(format!("Could not start the file picker: {error}").into());
        }
    });
    bind!(window,on_field_edit,[id:slint::SharedString,value:slint::SharedString],|app,state,_,_,id,value|{
        let Some(h)=host(app)else{return;};let selected=state.borrow().selected.clone();
        let kind=h.list().iter().find(|p|p.manifest.id==selected).and_then(|p|p.manifest.config.iter().find(|f|f.id==id.as_str()).map(|f|f.kind));
        let value=match kind {Some(ConfigKind::Bool)=>Value::Bool(value=="true"),Some(ConfigKind::Integer)=>match value.parse::<i64>(){Ok(n)=>json!(n),Err(_)=>return},Some(ConfigKind::Number)=>match value.parse::<f64>(){Ok(n)if n.is_finite()=>json!(n),_=>return},_=>Value::String(value.to_string())};
        if !state.borrow().config.is_object(){state.borrow_mut().config=json!({});}state.borrow_mut().config[id.as_str()]=value;
    });
    bind!(window, on_save_config, [], |app, state, window, ui| {
        if let Some(h) = host(app) {
            let id = state.borrow().selected.clone();
            let values = state.borrow().config.clone();
            feedback(&window.as_weak(), h.configure(&id, values));
            state.borrow_mut().config = h.configuration(&id).unwrap_or_else(|_| json!({}));
            state.borrow_mut().config_rendered.clear();
            refresh(app, ui, window, state);
        }
    });
    bind!(window,on_pin_action,[id:slint::SharedString,action:slint::SharedString,pinned:bool],|app,state,window,ui,id,action,pinned|{if let Some(h)=host(app){feedback(&window.as_weak(),h.set_pinned(&id,&action,pinned));refresh(app,ui,window,state);}});
    bind!(window,on_invoke,[action:slint::SharedString,value:slint::SharedString],|app,state,window,_,action,value|{let id=state.borrow().selected.clone();invoke(app,state,&window.as_weak(),&id,&action,&value);});
    bind!(window, on_open_view, [], |app, state, window, ui| {
        let id = state.borrow().selected.clone();
        select(app, ui, window, state, &id);
    });
    {
        let app = app.clone();
        let state = state.clone();
        let weak = ui.as_weak();
        let window = window.as_weak();
        ui.on_plugins_updated(move || {
            let (Some(ui), Some(window)) = (weak.upgrade(), window.upgrade()) else {
                return;
            };
            let batch = events
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .drain(..)
                .collect::<Vec<_>>();
            for event in batch {
                let plugin_id = event.plugin_id;
                match serde_json::from_value::<PluginMessage>(event.message) {
                    Ok(PluginMessage::View {
                        root,
                        view_id,
                        compact,
                        ..
                    }) => {
                        let mut state = state.borrow_mut();
                        if let Some(summary) = compact {
                            state
                                .summaries
                                .insert((plugin_id.clone(), view_id.clone()), summary);
                        } else {
                            state
                                .summaries
                                .remove(&(plugin_id.clone(), view_id.clone()));
                        }
                        state.roots.insert((plugin_id, view_id), root);
                    }
                    Ok(PluginMessage::Activity {
                        id,
                        title,
                        body,
                        accent,
                    }) => {
                        let mut a = app.borrow_mut();
                        let now = a.now();
                        a.coordinator.plugin_activity(
                            crate::presentation::PluginActivity {
                                plugin_id,
                                id,
                                title,
                                body,
                                accent: format!("{accent:?}"),
                            },
                            now,
                        );
                        drop(a);
                        sync_presentation(&app);
                    }
                    Ok(PluginMessage::ActionResult {
                        action_id,
                        success,
                        message,
                        open_uri,
                    }) => {
                        let explicit = state
                            .borrow_mut()
                            .invoked
                            .remove(&(plugin_id.clone(), action_id));
                        if explicit
                            && success
                            && let Some(uri) = open_uri
                        {
                            let scheme = uri
                                .split_once(':')
                                .map(|p| p.0.to_ascii_lowercase())
                                .unwrap_or_default();
                            let capability = format!("open_uri:{scheme}");
                            let declared = host(&app).is_some_and(|h| {
                                h.list().iter().any(|p| {
                                    p.manifest.id == plugin_id
                                        && p.manifest.capabilities.contains(&capability)
                                })
                            });
                            if !declared {
                                window.set_status(
                                    "Plugin did not declare this link capability".into(),
                                );
                            } else if let Err(e) = open_uri_action(&uri) {
                                window.set_status(e.into());
                            }
                        }
                        if !success {
                            window.set_status(message.into());
                        }
                    }
                    Ok(PluginMessage::Error { message }) => window.set_status(message.into()),
                    _ => {}
                }
            }
            if state.borrow().manager_open || state.borrow().panel_visible {
                refresh(&app, &ui, &window, &state);
            }
            let selected = app.borrow().settings.selected_page.clone();
            if let Some(crate::navigation::Page::Plugin { id, view }) = selected {
                let compact = app
                    .borrow()
                    .presentation
                    .as_ref()
                    .is_some_and(|p| matches!(p.content, Content::CompactPluginPage { .. }));
                if compact {
                    apply_summary(&ui, &host(&app).unwrap(), &state, &id, &view);
                }
            }
            pages::reconcile(&app);
        });
    }
    {
        let state = state.clone();
        let host = plugin_host;
        ui.on_plugin_visibility(move |visible| {
            let mut s = state.borrow_mut();
            s.panel_visible = visible;
            let _ = host.set_visible(&s.selected, visible || s.manager_open);
        });
    }
    refresh(app, ui, &window, &state);
    pages::reconcile(app);
    if let Some(crate::navigation::Page::Plugin { id, view }) =
        app.borrow().settings.selected_page.clone()
    {
        // Defer until the App borrow used to inspect the persisted preference ends.
        let weak = ui.as_weak();
        slint::Timer::single_shot(Duration::from_millis(1), move || {
            if let Some(ui) = weak.upgrade() {
                ui.invoke_plugin_page_select(id.into(), view.into());
            }
        });
    }
    if verifying {
        setup_verification(app, ui, &window);
    }
}

pub(super) fn sync_visibility(ui: &IslandWindow, view: &Presentation) {
    ui.invoke_plugin_visibility(matches!(view.content, Content::PluginPanel));
}

pub(super) fn apply_content_properties(ui: &IslandWindow, view: &Presentation) {
    ui.set_plugin_mode(matches!(view.content, Content::PluginPanel));
    ui.set_compact_plugin(matches!(
        view.content,
        Content::CompactPlugin(_) | Content::CompactPluginPage { .. }
    ));
    if let Content::CompactPluginPage { plugin_id, view_id } = &view.content {
        ui.invoke_plugin_summary_requested(plugin_id.as_str().into(), view_id.as_str().into());
    }
    if let Content::CompactPlugin(activity) = &view.content {
        ui.set_plugin_title(activity.title.as_str().into());
        ui.set_plugin_body(activity.body.as_str().into());
        ui.set_plugin_has_progress(false);
        ui.set_plugin_accent(color(Accent::Neutral));
    }
}
fn open_uri_action(uri: &str) -> Result<(), String> {
    if !valid_uri(uri) {
        return Err("Plugin requested an unsupported link".into());
    }
    use windows::{
        Win32::{
            Foundation::HWND,
            UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        },
        core::HSTRING,
    };
    // SAFETY: the URI is validated and lives across the synchronous call.
    // Only a response to an explicit user action reaches this operation.
    let result = unsafe {
        ShellExecuteW(
            Some(HWND::default()),
            None,
            &HSTRING::from(uri),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize <= 32 {
        Err("Windows could not open the integration link".into())
    } else {
        Ok(())
    }
}
fn valid_uri(uri: &str) -> bool {
    uri.len() <= 16000
        && !uri.chars().any(char::is_control)
        && uri.split_once("://").is_some_and(|(scheme, rest)| {
            !rest.is_empty()
                && ["https", "http", "obsidian"]
                    .iter()
                    .any(|allowed| scheme.eq_ignore_ascii_case(allowed))
        })
}
fn setup_verification(app: &Rc<RefCell<App>>, ui: &IslandWindow, window: &PluginManagerWindow) {
    if verification::has_arg("--verify-pages") {
        page_verification::setup(app, ui, window);
        return;
    }
    if verification::has_arg("--verify-plugin-picker") {
        let expected = std::env::args().find_map(|arg| {
            arg.strip_prefix("--verify-picker-package=")
                .map(str::to_owned)
        });
        if let Some(expected) = expected {
            picker_verification_step(
                Rc::downgrade(app),
                window.as_weak(),
                Rc::new(std::cell::Cell::new((0u8, 0u32, 0u32))),
                expected,
            );
        } else {
            record("picker FAILED missing --verify-picker-package");
            let _ = slint::quit_event_loop();
        }
        return;
    }
    let package = std::env::args().find_map(|a| {
        a.strip_prefix("--verify-plugin-package=")
            .map(str::to_owned)
    });
    let Some(package) = package else {
        record("plugin FAILED missing --verify-plugin-package");
        let _ = slint::quit_event_loop();
        return;
    };
    for (step, at) in [700, 1900, 3300, 4000, 5000, 6100, 7000, 8200, 9500]
        .into_iter()
        .enumerate()
    {
        let weak = Rc::downgrade(app);
        let ui = ui.as_weak();
        let window = window.as_weak();
        let package = package.clone();
        slint::Timer::single_shot(Duration::from_millis(at), move || {
            let (Some(app), Some(ui), Some(window)) =
                (weak.upgrade(), ui.upgrade(), window.upgrade())
            else {
                return;
            };
            let id = "org.stillisland.developer";
            let check = |ok: bool, text: &str| {
                record(&format!(
                    "plugin {} {text}",
                    if ok { "PASS" } else { "FAILED" }
                ))
            };
            match step {
                0 => window.invoke_import_package(package.into()),
                1 => {
                    check(
                        host(&app).is_some_and(|h| {
                            h.list().iter().any(|p| p.manifest.id == id && !p.enabled)
                        }),
                        "import remains disabled",
                    );
                    window.invoke_enable_plugin(id.into(), true);
                    ui.invoke_open_plugins();
                    ui.invoke_plugin_select(id.into());
                }
                2 => {
                    check(
                        ui.get_plugin_nodes().row_count() >= 5,
                        "custom native layout rendered",
                    );
                    match crate::verification::snapshot(&ui, "plugin-view") {
                        Ok(p) => record(&format!("snapshot {}", p.display())),
                        Err(e) => record(&format!("plugin FAILED {e}")),
                    };
                    ui.invoke_plugin_action(id.into(), "increment".into(), "".into());
                }
                3 => {
                    check(
                        (0..ui.get_plugin_nodes().row_count())
                            .filter_map(|i| ui.get_plugin_nodes().row_data(i))
                            .any(|n| n.kind == "progress" && n.value > 0.),
                        "user action updates progress",
                    );
                    ui.invoke_plugin_edit();
                }
                4 => window.invoke_invoke("input".into(), "Updated from native editor".into()),
                5 => {
                    check(
                        (0..window.get_nodes().row_count())
                            .filter_map(|i| window.get_nodes().row_data(i))
                            .any(|n| n.kind == "input" && n.text == "Updated from native editor"),
                        "editor input action roundtrip",
                    );
                    if let Ok(image) = window.window().take_snapshot() {
                        use std::io::Write;
                        let result = (|| -> std::io::Result<()> {
                            std::fs::create_dir_all("verification")?;
                            let mut file =
                                std::fs::File::create("verification/plugin-manager.ppm")?;
                            write!(file, "P6\n{} {}\n255\n", image.width(), image.height())?;
                            for p in image.as_slice() {
                                file.write_all(&[p.r, p.g, p.b])?;
                            }
                            Ok(())
                        })();
                        check(result.is_ok(), "manager snapshot");
                    }
                    window.invoke_close_window();
                    transition(&app, IslandState::Idle);
                    ui.invoke_plugin_action(id.into(), "notify".into(), "".into());
                }
                6 => {
                    check(
                        matches!(
                            app.borrow().presentation.as_ref().map(|p| &p.content),
                            Some(Content::CompactPlugin(_))
                        ),
                        "proactive preview arbitration",
                    );
                    window.invoke_enable_plugin(id.into(), false);
                }
                7 => check(
                    host(&app).is_some_and(|h| {
                        h.list().iter().any(|p| p.manifest.id == id && !p.enabled)
                    }),
                    "disable stops plugin",
                ),
                _ => {
                    check(
                        !app.borrow().timer.running(),
                        "animation scheduling settles",
                    );
                    record("plugin verification complete");
                    let _ = slint::quit_event_loop();
                }
            }
        });
    }
}

/// Finite native-dialog test. An external owned-window fixture drives Cancel and
/// Select; the pulses prove Slint continues dispatching while the Shell is open.
fn picker_verification_step(
    app: std::rc::Weak<RefCell<App>>,
    manager: slint::Weak<PluginManagerWindow>,
    progress: Rc<std::cell::Cell<(u8, u32, u32)>>,
    expected: String,
) {
    slint::Timer::single_shot(Duration::from_millis(200), move || {
        let (Some(app_live), Some(window)) = (app.upgrade(), manager.upgrade()) else {
            return;
        };
        let (mut phase, mut ticks, mut pulses) = progress.get();
        ticks += 1;
        let check = |passed: bool, label: &str| {
            record(&format!(
                "picker {} {label}",
                if passed { "PASS" } else { "FAILED" }
            ))
        };
        if ticks >= 150 {
            check(false, "native fixture timed out");
            let _ = slint::quit_event_loop();
            return;
        }
        if window.get_busy() {
            pulses += 1;
        }
        match phase {
            0 if ticks >= 3 => {
                window.set_package_path("previous-selection.zip".into());
                show_manager(&window);
                phase = 1;
            }
            1 if ticks >= 5 => {
                window.invoke_browse();
                phase = 2;
                record("picker awaiting first cancel");
            }
            2 if !window.get_busy() => {
                check(
                    window.get_package_path() == "previous-selection.zip",
                    "cancel preserves previous package path",
                );
                check(
                    window.get_status() == "Selection cancelled.",
                    "cancel is distinguished from native errors",
                );
                check(
                    pulses >= 2,
                    "Slint event loop dispatches while first native picker is open",
                );
                pulses = 0;
                window.invoke_browse();
                phase = 3;
                record("picker awaiting second selection");
            }
            3 if !window.get_busy() => {
                check(
                    Path::new(window.get_package_path().as_str()) == Path::new(&expected),
                    "selected ZIP path returns to integration manager",
                );
                check(
                    pulses >= 2,
                    "Slint event loop dispatches while second native picker is open",
                );
                check(
                    host(&app_live).is_some_and(|h| h.list().is_empty()),
                    "selection never imports or enables a plugin",
                );
                record("picker verification complete");
                let _ = slint::quit_event_loop();
                return;
            }
            _ => {}
        }
        progress.set((phase, ticks, pulses));
        picker_verification_step(app, manager, progress, expected);
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn input_strings_remain_strings_and_toggle_is_boolean() {
        let input = Node {
            id: "input".into(),
            kind: NodeKind::Input,
            action: "edit".into(),
            ..Node::default()
        };
        for text in ["123", "true", "false", "null"] {
            assert_eq!(
                action_value(Some(&input), "edit", text),
                Value::String(text.into())
            );
        }
        let toggle = Node {
            kind: NodeKind::Toggle,
            ..input
        };
        assert_eq!(
            action_value(Some(&toggle), "edit", "true"),
            Value::Bool(true)
        );
    }
    fn text(id: &str) -> Node {
        Node {
            id: id.into(),
            kind: NodeKind::Text,
            text: "Text".into(),
            ..Node::default()
        }
    }
    #[test]
    fn nested_lists_are_scrollable_and_bounded() {
        let root = Node {
            id: "root".into(),
            kind: NodeKind::List,
            padding: 6.,
            spacing: 4.,
            children: (0..128).map(|i| text(&format!("n{i}"))).collect(),
            ..Node::default()
        };
        for width in [180., 312., 500.] {
            let (nodes, height) = flatten(&root, Path::new("."), width);
            assert_eq!(nodes.len(), 128);
            assert!(height > 3000.);
            assert!(
                nodes
                    .iter()
                    .all(|n| n.x >= 0. && n.width <= width && n.y + n.height <= height)
            );
        }
    }
    #[test]
    fn rows_share_space_and_nested_columns_do_not_overlap() {
        let root = Node {
            id: "root".into(),
            kind: NodeKind::Column,
            spacing: 4.,
            children: vec![
                Node {
                    id: "row".into(),
                    kind: NodeKind::Row,
                    spacing: 10.,
                    children: vec![
                        Node {
                            flex: 2.,
                            ..text("left")
                        },
                        text("right"),
                    ],
                    ..Node::default()
                },
                text("footer"),
            ],
            ..Node::default()
        };
        let (n, h) = flatten(&root, Path::new("."), 312.);
        assert_eq!(n.len(), 3);
        assert!(n[0].width > n[1].width);
        assert!(n[0].x + n[0].width < n[1].x);
        assert!(n[2].y >= n[0].height);
        assert!(h >= n[2].y + n[2].height);
    }
    #[test]
    fn link_allowlist_rejects_command_and_file_schemes() {
        for uri in [
            "https://example.com",
            "HTTP://example.com",
            "obsidian://daily?vault=Notes",
        ] {
            assert!(valid_uri(uri));
        }
        for uri in [
            "file:///C:/x.exe",
            "javascript:alert(1)",
            "cmd://run",
            "\\\\server\\share",
            "https://example.com\ncmd",
            "https://",
        ] {
            assert!(!valid_uri(uri));
        }
    }
}

#[cfg(test)]
mod text_layout_tests {
    use super::*;
    #[test]
    fn styled_flex_columns_measure_the_same_widths_they_render() {
        let mut narrow = Node {
            id: "narrow".into(),
            kind: NodeKind::Text,
            text: "A long label within the narrow column".into(),
            ..Node::default()
        };
        narrow.text_role = still_island_plugin_protocol::TextRole::Title;
        let row = Node {
            id: "row".into(),
            kind: NodeKind::Row,
            padding: 5.,
            spacing: 12.,
            children: vec![
                Node {
                    id: "wide".into(),
                    kind: NodeKind::Text,
                    text: "12%".into(),
                    flex: 3.,
                    text_role: still_island_plugin_protocol::TextRole::Metric,
                    ..Node::default()
                },
                narrow,
            ],
            ..Node::default()
        };
        let root = Node {
            id: "root".into(),
            kind: NodeKind::Column,
            children: vec![
                row,
                Node {
                    id: "footer".into(),
                    kind: NodeKind::Text,
                    text: "Updated".into(),
                    ..Node::default()
                },
            ],
            ..Node::default()
        };
        let (nodes, height) = flatten(&root, Path::new("."), 312.);
        assert_eq!(nodes[0].font_size, 28.);
        assert_eq!(nodes[0].font_weight, 600);
        assert_eq!(nodes[1].font_size, 14.);
        assert!(nodes[2].y >= nodes[0].y + nodes[0].height);
        assert!(nodes[2].y >= nodes[1].y + nodes[1].height);
        assert!(height >= nodes[2].y + nodes[2].height);
    }
}
