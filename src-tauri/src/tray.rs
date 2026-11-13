//! The menu bar icon (issue 022): recording state and start/stop from
//! anywhere on the Mac.
//!
//! What the icon, its title and its menu show for a state is decided by the
//! pure functions at the top, which the tests cover. The rest applies that
//! to the real tray icon and runs the menu's actions through the same
//! [`AppCore`] calls as the window's commands.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use serde::Serialize;
use tauri::image::Image;
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, include_image};

use crate::app::AppCore;
use crate::commands::AppState;
use crate::dto;

/// Event carrying a [`RecordingError`]: a start or stop asked for outside
/// the window (menu bar, shortcut) failed. The window is shown first.
pub const RECORDING_ERROR_EVENT: &str = "recording-error";

/// Label of the main window in `tauri.conf.json`.
pub const MAIN_WINDOW: &str = "main";

pub const TOGGLE_ID: &str = "toggle-recording";
pub const OPEN_ID: &str = "open-mneme";
pub const QUIT_ID: &str = "quit-mneme";

const IDLE_ICON: Image<'static> = include_image!("./icons/tray-idle.png");
const RECORDING_ICON: Image<'static> = include_image!("./icons/tray-recording.png");

// ---- What to show: pure ----

/// The recording state as the menu bar tells it apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Recording,
    Stopping,
}

impl Phase {
    /// From the state the window gets. A status this code does not know
    /// counts as idle.
    pub fn of(state: &dto::RecordingState) -> Self {
        match state.status {
            "recording" => Self::Recording,
            "stopping" => Self::Stopping,
            _ => Self::Idle,
        }
    }
}

/// What a toggle does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggle {
    Start,
    Stop,
}

impl Toggle {
    const fn name(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
        }
    }
}

/// What a toggle does in this phase: nothing while a stop is under way.
pub const fn toggle_action(phase: Phase) -> Option<Toggle> {
    match phase {
        Phase::Idle => Some(Toggle::Start),
        Phase::Recording => Some(Toggle::Stop),
        Phase::Stopping => None,
    }
}

/// What a click on the menu's first line does. The line was drawn for
/// `shown`; if the state has moved on since (the meeting ended by itself
/// while the menu was open), the click does nothing rather than the opposite
/// of what the line said.
pub fn menu_click_action(shown: Phase, current: Phase) -> Option<Toggle> {
    toggle_action(shown).filter(|action| toggle_action(current) == Some(*action))
}

/// Which of the two icons shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconKind {
    Idle,
    Recording,
}

/// The recording icon stays until the meeting is closed.
pub const fn icon_kind(phase: Phase) -> IconKind {
    match phase {
        Phase::Idle => IconKind::Idle,
        Phase::Recording | Phase::Stopping => IconKind::Recording,
    }
}

/// One line of the menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuEntry {
    pub id: &'static str,
    pub text: &'static str,
    pub enabled: bool,
}

/// The menu, top to bottom. Start is disabled, with the reason in its text,
/// when no model is installed.
pub const fn menu_model(phase: Phase, model_installed: bool) -> [MenuEntry; 3] {
    let (text, enabled) = match phase {
        Phase::Idle if model_installed => ("Start Recording", true),
        Phase::Idle => ("Start Recording (no model installed)", false),
        Phase::Recording => ("Stop Recording", true),
        Phase::Stopping => ("Stopping…", false),
    };
    [
        MenuEntry {
            id: TOGGLE_ID,
            text,
            enabled,
        },
        MenuEntry {
            id: OPEN_ID,
            text: "Open Mneme",
            enabled: true,
        },
        MenuEntry {
            id: QUIT_ID,
            text: "Quit Mneme",
            enabled: true,
        },
    ]
}

/// The text next to the icon: the elapsed time while recording, in the
/// window's format (`0:07`, `12:34`, `1:02:03` from an hour on), otherwise
/// none.
pub fn title_text(phase: Phase, elapsed_secs: u64) -> Option<String> {
    if phase != Phase::Recording {
        return None;
    }
    let (hours, minutes, seconds) = (
        elapsed_secs / 3600,
        elapsed_secs / 60 % 60,
        elapsed_secs % 60,
    );
    Some(if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    })
}

// ---- The real tray icon ----

/// Payload of [`RECORDING_ERROR_EVENT`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingError {
    /// `start` or `stop`.
    pub action: &'static str,
    pub message: String,
}

/// What the tray icon shows now, to change only what differs.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Shown {
    phase: Phase,
    model_installed: bool,
    title: Option<String>,
}

/// Managed by Tauri once the tray icon exists.
struct Tray {
    icon: TrayIcon,
    toggle: MenuItem<tauri::Wry>,
    /// Whether Start can work. Read when drawing, set by [`refresh`].
    model_installed: AtomicBool,
    shown: Mutex<Option<Shown>>,
}

/// A start or stop asked for here is under way. Further toggles are dropped
/// until it ends, so a second click during a model load does nothing.
static BUSY: AtomicBool = AtomicBool::new(false);

fn core(app: &AppHandle) -> Option<Arc<AppCore>> {
    app.try_state::<AppState>()?.core.clone().ok()
}

/// Creates the tray icon and its menu. Call once, in `setup`, after the
/// [`AppState`] is managed.
pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let [toggle, open, quit] = menu_model(Phase::Idle, true);
    let item = |entry: MenuEntry| {
        MenuItem::with_id(app, entry.id, entry.text, entry.enabled, None::<&str>)
    };
    let toggle = item(toggle)?;
    let menu = Menu::with_items(
        app,
        &[
            &toggle,
            &PredefinedMenuItem::separator(app)?,
            &item(open)?,
            &item(quit)?,
        ],
    )?;
    let icon = TrayIconBuilder::with_id("mneme")
        .icon(IDLE_ICON)
        .icon_as_template(true)
        .tooltip("Mneme")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(on_menu_event)
        // The pointer reaches the icon before the menu opens: the moment to
        // notice a model installed or deleted since the last look.
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Enter { .. } | TrayIconEvent::Click { .. }
            ) {
                refresh(tray.app_handle());
            }
        })
        .build(app)?;
    app.manage(Tray {
        icon,
        toggle,
        model_installed: AtomicBool::new(true),
        shown: Mutex::new(None),
    });
    refresh(app);
    Ok(())
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    match event.id().as_ref() {
        TOGGLE_ID => {
            let shown = app.try_state::<Tray>().and_then(|tray| {
                let shown = tray.shown.lock().unwrap_or_else(PoisonError::into_inner);
                shown.as_ref().map(|shown| shown.phase)
            });
            toggle(app, shown);
        }
        OPEN_ID => show_main_window(app),
        // `RunEvent::Exit` stops a running recording before the process ends.
        QUIT_ID => app.exit(0),
        _ => {}
    }
}

/// Starts a meeting when idle, stops the running one when recording, and
/// does nothing while a stop is under way. Returns at once; the work runs on
/// the blocking pool. Opens no window unless it fails: then the main window
/// is shown and gets a [`RECORDING_ERROR_EVENT`].
// The entry point for the global shortcut (`shortcut.rs`).
pub fn toggle_recording(app: &AppHandle) {
    toggle(app, None);
}

/// `shown` is the phase the menu line was drawn for, when the toggle comes
/// from the menu.
fn toggle(app: &AppHandle, shown: Option<Phase>) {
    let core = match app.try_state::<AppState>().map(|state| state.core.clone()) {
        Some(Ok(core)) => core,
        Some(Err(error)) => return report_error(app, Toggle::Start, error),
        None => return,
    };
    if BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let current = Phase::of(&core.recording_state());
        let action = menu_click_action(shown.unwrap_or(current), current);
        let result = match action {
            Some(Toggle::Start) => core.start_meeting().map(drop),
            Some(Toggle::Stop) => core.stop_meeting().map(drop),
            None => Ok(()),
        };
        BUSY.store(false, Ordering::SeqCst);
        if let (Some(action), Err(error)) = (action, result) {
            report_error(&app, action, error);
        }
    });
}

/// Never silent: the window comes up and is told what went wrong.
fn report_error(app: &AppHandle, action: Toggle, message: String) {
    eprintln!("mneme: {} from the menu bar: {message}", action.name());
    show_main_window(app);
    let error = RecordingError {
        action: action.name(),
        message,
    };
    if let Err(error) = app.emit(RECORDING_ERROR_EVENT, error) {
        eprintln!("mneme: sending the recording error: {error}");
    }
}

/// Shows the main window and gives it the focus.
pub fn show_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        eprintln!("mneme: the main window is gone");
        return;
    };
    let steps = [window.unminimize(), window.show(), window.set_focus()];
    for error in steps.into_iter().filter_map(Result::err) {
        eprintln!("mneme: showing the main window: {error}");
    }
}

/// The recording state changed or ticked: the host calls this with every
/// state it sends to the window. Never blocks, whatever thread it is on.
pub fn recording_state_changed(app: &AppHandle, state: &dto::RecordingState) {
    if Phase::of(state) == Phase::Idle {
        // Start is on offer again: look at the models too.
        refresh(app);
    } else {
        redraw(app);
    }
}

/// Looks again at whether a model is installed, then redraws. Returns at
/// once; the look runs on the blocking pool.
pub fn refresh(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(tray) = app.try_state::<Tray>() {
            // With no core, Start stays on offer: choosing it shows why the
            // app could not open its state.
            let installed =
                core(&app).is_none_or(|core| core.list_models().iter().any(|m| m.installed));
            tray.model_installed.store(installed, Ordering::SeqCst);
        }
        redraw(&app);
    });
}

/// Queues a redraw on the main thread and returns. It must not wait for the
/// main thread: at quit that thread waits for the recording to stop, and the
/// stop sends states through here.
fn redraw(app: &AppHandle) {
    let handle = app.clone();
    if let Err(error) = app.run_on_main_thread(move || draw(&handle)) {
        eprintln!("mneme: updating the menu bar icon: {error}");
    }
}

/// Runs on the main thread. Reads the state when it draws, so a redraw that
/// was queued earlier can never put an older state back.
fn draw(app: &AppHandle) {
    let Some(tray) = app.try_state::<Tray>() else {
        return;
    };
    let state = core(app).map_or_else(dto::RecordingState::idle, |core| core.recording_state());
    let phase = Phase::of(&state);
    let next = Shown {
        phase,
        model_installed: tray.model_installed.load(Ordering::SeqCst),
        title: title_text(phase, state.elapsed_secs),
    };
    let mut shown = tray.shown.lock().unwrap_or_else(PoisonError::into_inner);
    if shown.as_ref() == Some(&next) {
        return;
    }
    let previous = shown.replace(next.clone());
    if let Err(error) = show(&tray, previous.as_ref(), &next) {
        eprintln!("mneme: updating the menu bar icon: {error}");
    }
}

fn show(tray: &Tray, previous: Option<&Shown>, next: &Shown) -> tauri::Result<()> {
    let icon = icon_kind(next.phase);
    if previous.map(|p| icon_kind(p.phase)) != Some(icon) {
        let image = match icon {
            IconKind::Idle => IDLE_ICON,
            IconKind::Recording => RECORDING_ICON,
        };
        tray.icon.set_icon_with_as_template(Some(image), true)?;
    }
    if previous.map(|p| &p.title) != Some(&next.title) {
        tray.icon.set_title(next.title.as_deref())?;
    }
    let [toggle, ..] = menu_model(next.phase, next.model_installed);
    if previous.map(|p| menu_model(p.phase, p.model_installed)[0]) != Some(toggle) {
        tray.toggle.set_text(toggle.text)?;
        tray.toggle.set_enabled(toggle.enabled)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(status: &'static str, elapsed_secs: u64) -> dto::RecordingState {
        dto::RecordingState {
            status,
            meeting_id: Some("m1".to_owned()),
            elapsed_secs,
        }
    }

    fn toggle_entry(phase: Phase, model_installed: bool) -> MenuEntry {
        menu_model(phase, model_installed)[0]
    }

    #[test]
    fn phase_follows_the_status_the_window_gets() {
        assert_eq!(Phase::of(&dto::RecordingState::idle()), Phase::Idle);
        assert_eq!(Phase::of(&state("idle", 0)), Phase::Idle);
        assert_eq!(Phase::of(&state("recording", 3)), Phase::Recording);
        assert_eq!(Phase::of(&state("stopping", 3)), Phase::Stopping);
        assert_eq!(Phase::of(&state("something new", 3)), Phase::Idle);
    }

    #[test]
    fn idle_menu_offers_start() {
        assert_eq!(
            menu_model(Phase::Idle, true),
            [
                MenuEntry {
                    id: TOGGLE_ID,
                    text: "Start Recording",
                    enabled: true
                },
                MenuEntry {
                    id: OPEN_ID,
                    text: "Open Mneme",
                    enabled: true
                },
                MenuEntry {
                    id: QUIT_ID,
                    text: "Quit Mneme",
                    enabled: true
                },
            ]
        );
    }

    #[test]
    fn start_is_disabled_with_a_hint_when_no_model_is_installed() {
        let entry = toggle_entry(Phase::Idle, false);
        assert_eq!(entry.id, TOGGLE_ID);
        assert!(!entry.enabled);
        assert_eq!(entry.text, "Start Recording (no model installed)");
    }

    #[test]
    fn recording_menu_offers_stop_whatever_the_models() {
        for model_installed in [true, false] {
            assert_eq!(
                toggle_entry(Phase::Recording, model_installed),
                MenuEntry {
                    id: TOGGLE_ID,
                    text: "Stop Recording",
                    enabled: true
                }
            );
        }
    }

    #[test]
    fn stopping_menu_offers_neither_start_nor_stop() {
        for model_installed in [true, false] {
            let entry = toggle_entry(Phase::Stopping, model_installed);
            assert_eq!(entry.text, "Stopping…");
            assert!(!entry.enabled);
        }
    }

    #[test]
    fn open_and_quit_are_always_there_and_enabled() {
        for phase in [Phase::Idle, Phase::Recording, Phase::Stopping] {
            for model_installed in [true, false] {
                let [_, open, quit] = menu_model(phase, model_installed);
                assert_eq!(
                    (open.id, open.text, open.enabled),
                    (OPEN_ID, "Open Mneme", true)
                );
                assert_eq!(
                    (quit.id, quit.text, quit.enabled),
                    (QUIT_ID, "Quit Mneme", true)
                );
            }
        }
    }

    #[test]
    fn toggle_starts_when_idle_and_stops_when_recording() {
        assert_eq!(toggle_action(Phase::Idle), Some(Toggle::Start));
        assert_eq!(toggle_action(Phase::Recording), Some(Toggle::Stop));
        assert_eq!(toggle_action(Phase::Stopping), None);
    }

    #[test]
    fn a_menu_click_does_what_its_line_said_or_nothing() {
        use Phase::{Idle, Recording, Stopping};
        assert_eq!(menu_click_action(Idle, Idle), Some(Toggle::Start));
        assert_eq!(menu_click_action(Recording, Recording), Some(Toggle::Stop));
        // The line said Stop, the meeting has ended since: no new meeting.
        assert_eq!(menu_click_action(Recording, Idle), None);
        assert_eq!(menu_click_action(Recording, Stopping), None);
        // The line said Start, a meeting is recording since: it is not stopped.
        assert_eq!(menu_click_action(Idle, Recording), None);
        assert_eq!(menu_click_action(Idle, Stopping), None);
        for current in [Idle, Recording, Stopping] {
            assert_eq!(menu_click_action(Stopping, current), None);
        }
    }

    #[test]
    fn title_is_the_elapsed_time_in_the_windows_format() {
        let title = |secs| title_text(Phase::Recording, secs);
        assert_eq!(title(0).as_deref(), Some("0:00"));
        assert_eq!(title(7).as_deref(), Some("0:07"));
        assert_eq!(title(59).as_deref(), Some("0:59"));
        assert_eq!(title(60).as_deref(), Some("1:00"));
        assert_eq!(title(754).as_deref(), Some("12:34"));
        assert_eq!(title(3599).as_deref(), Some("59:59"));
        assert_eq!(title(3600).as_deref(), Some("1:00:00"));
        assert_eq!(title(3723).as_deref(), Some("1:02:03"));
        assert_eq!(title(36_000 + 61).as_deref(), Some("10:01:01"));
    }

    #[test]
    fn no_title_unless_recording() {
        assert_eq!(title_text(Phase::Idle, 0), None);
        assert_eq!(title_text(Phase::Idle, 42), None);
        assert_eq!(title_text(Phase::Stopping, 42), None);
    }

    #[test]
    fn recording_icon_shows_until_the_meeting_is_closed() {
        assert_eq!(icon_kind(Phase::Idle), IconKind::Idle);
        assert_eq!(icon_kind(Phase::Recording), IconKind::Recording);
        assert_eq!(icon_kind(Phase::Stopping), IconKind::Recording);
    }

    /// Template images: black with an alpha channel only, so macOS can tint
    /// them. The two states must differ in shape, which here is the alpha.
    #[test]
    fn icons_are_templates_that_differ_in_shape() {
        let alpha = |image: &Image<'_>| -> Vec<u8> {
            assert_eq!((image.width(), image.height()), (36, 36));
            let (pixels, rest) = image.rgba().as_chunks::<4>();
            assert!(rest.is_empty());
            assert!(pixels.iter().all(|p| p[..3] == [0, 0, 0]));
            pixels.iter().map(|p| p[3]).collect()
        };
        let (idle, recording) = (alpha(&IDLE_ICON), alpha(&RECORDING_ICON));
        for shape in [&idle, &recording] {
            assert!(shape.contains(&255) && shape.contains(&0));
        }
        let differing = idle
            .iter()
            .zip(&recording)
            .filter(|(a, b)| a.abs_diff(**b) > 127)
            .count();
        assert!(differing > 200, "only {differing} pixels differ");
    }
}
