//! The system-wide shortcut that toggles recording (issue 023).
//!
//! Nothing is registered until the user picks a shortcut in Settings. The
//! Settings screen saves it as `[Control+][Alt+][Shift+][Command+]<code>`,
//! where `<code>` is a `KeyboardEvent.code` (`Shift+Command+KeyR`). The
//! plugin's own parser reads that format as it is, so nothing is converted.
//!
//! Pure part: [`parse`], [`wanted`], [`press`], and [`register_saved`] and
//! [`change`], which decide what to register and unregister through a
//! [`Registrar`]. Real part: [`setup`] and [`update_settings`], which hand
//! them the plugin. Shortcuts are registered from Rust only: the window has
//! no permission for the plugin's commands.

use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use mneme_store::{Patch, Settings, SettingsPatch};
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcut, Shortcut, ShortcutState};

use crate::app::AppCore;
use crate::commands::AppState;
use crate::tray;

/// The error for a combination the system would not give us.
const NOT_REGISTERED: &str = "That shortcut could not be registered. macOS or another app may \
     already be using it. Try a different combination.";
/// The error when the shortcut in use could not be given back.
const NOT_REMOVED: &str =
    "The current shortcut could not be removed, so the shortcut was not changed.";
const NOT_AVAILABLE: &str = "Global shortcuts are not available in this run of Mneme.";

/// The shortcut that is registered with the system right now. Held for the
/// whole of a change, so two changes cannot interleave.
static ACTIVE: Mutex<Option<Shortcut>> = Mutex::new(None);
/// Whether the shortcut's keys are down, as far as the events say.
static HELD: AtomicBool = AtomicBool::new(false);

/// Reads a shortcut in the format the Settings screen saves, with the
/// plugin's own parser. A shortcut must have at least one modifier:
/// without one it would take an ordinary key away from every app.
pub fn parse(text: &str) -> Result<Shortcut, String> {
    let shortcut = Shortcut::from_str(text).map_err(|_| {
        format!(
            "\"{text}\" is not a shortcut Mneme can use. Hold Control, Alt, Shift or Command \
             and press one key."
        )
    })?;
    if shortcut.mods.is_empty() {
        return Err(format!(
            "\"{text}\" has no modifier key. Hold Control, Alt, Shift or Command and press \
             one key."
        ));
    }
    Ok(shortcut)
}

/// What a settings patch asks of the shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wanted {
    /// The patch does not mention the shortcut.
    Keep,
    /// No shortcut.
    Clear,
    Set(Shortcut),
}

/// Reads the patch the way the settings store does: text is trimmed and
/// empty text means none. An invalid shortcut is an error, so it is never
/// saved.
pub fn wanted(patch: &Patch<String>) -> Result<Wanted, String> {
    match patch {
        Patch::Keep => Ok(Wanted::Keep),
        Patch::Clear => Ok(Wanted::Clear),
        Patch::Set(text) if text.trim().is_empty() => Ok(Wanted::Clear),
        Patch::Set(text) => parse(text.trim()).map(Wanted::Set),
    }
}

/// One event from the system for the shortcut. `held` is whether its keys
/// were already down. Returns whether they are down now, and whether this
/// event toggles recording: only the first press does, not a repeat while
/// the keys stay down and not the release.
pub const fn press(held: bool, state: ShortcutState) -> (bool, bool) {
    match state {
        ShortcutState::Pressed => (true, !held),
        ShortcutState::Released => (false, false),
    }
}

/// Registers and unregisters with the system. The errors are for the log.
pub trait Registrar {
    fn register(&mut self, shortcut: Shortcut) -> Result<(), String>;
    fn unregister(&mut self, shortcut: Shortcut) -> Result<(), String>;
}

/// At launch: registers the shortcut saved in settings, if there is one.
/// `active` is what is registered afterwards.
pub fn register_saved(
    registrar: &mut impl Registrar,
    active: &mut Option<Shortcut>,
    saved: Option<&str>,
) -> Result<(), String> {
    let Some(saved) = saved else {
        return Ok(());
    };
    let shortcut = parse(saved)?;
    registrar.register(shortcut)?;
    *active = Some(shortcut);
    Ok(())
}

/// Changes the shortcut from `active` to `wanted` and saves with `save`.
///
/// The new one is registered first, then the old one is unregistered, then
/// the setting is saved. If the new one cannot be registered nothing else
/// happens: the old one stays registered and saved. If the save fails the
/// old one is put back. `active` is what is registered afterwards.
pub fn change<T>(
    registrar: &mut impl Registrar,
    active: &mut Option<Shortcut>,
    wanted: Option<Shortcut>,
    save: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let old = *active;
    if wanted == old {
        return save();
    }
    if let Some(new) = wanted
        && let Err(error) = registrar.register(new)
    {
        log(format_args!("registering: {error}"));
        return Err(NOT_REGISTERED.to_owned());
    }
    if let Some(old) = old
        && let Err(error) = registrar.unregister(old)
    {
        log(format_args!("unregistering: {error}"));
        if let Some(new) = wanted {
            undo(registrar.unregister(new));
        }
        return Err(NOT_REMOVED.to_owned());
    }
    *active = wanted;
    match save() {
        Ok(saved) => Ok(saved),
        Err(error) => {
            // The settings file still names the old shortcut.
            if let Some(new) = wanted {
                undo(registrar.unregister(new));
            }
            *active = None;
            if let Some(old) = old {
                match registrar.register(old) {
                    Ok(()) => *active = Some(old),
                    Err(error) => log(format_args!("restoring: {error}")),
                }
            }
            Err(error)
        }
    }
}

fn undo(result: Result<(), String>) {
    if let Err(error) = result {
        log(format_args!("undoing: {error}"));
    }
}

fn log(message: std::fmt::Arguments<'_>) {
    eprintln!("mneme: global shortcut: {message}");
}

/// The plugin as a [`Registrar`]. Its calls run on the main thread and
/// return when they are done there.
struct System<'a>(&'a AppHandle);

impl System<'_> {
    fn plugin(&self) -> Result<tauri::State<'_, GlobalShortcut<Wry>>, String> {
        self.0
            .try_state::<GlobalShortcut<Wry>>()
            .ok_or_else(|| NOT_AVAILABLE.to_owned())
    }
}

impl Registrar for System<'_> {
    fn register(&mut self, shortcut: Shortcut) -> Result<(), String> {
        self.plugin()?
            .register(shortcut)
            .map_err(|error| error.to_string())
    }

    fn unregister(&mut self, shortcut: Shortcut) -> Result<(), String> {
        self.plugin()?
            .unregister(shortcut)
            .map_err(|error| error.to_string())
    }
}

/// Adds the plugin and registers the saved shortcut, if any. Called once at
/// launch, after the app state is managed. A failure is logged and the app
/// carries on without the shortcut.
pub fn setup(app: &AppHandle) {
    let plugin = tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, _shortcut, event| {
            let (held, toggle) = press(HELD.load(Ordering::SeqCst), event.state);
            HELD.store(held, Ordering::SeqCst);
            if toggle {
                tray::toggle_recording(app);
            }
        })
        .build();
    if let Err(error) = app.plugin(plugin) {
        log(format_args!("{error}"));
        return;
    }
    let saved = app
        .try_state::<AppState>()
        .and_then(|state| state.core.as_ref().ok().map(|core| core.get_settings()))
        .and_then(|settings| settings.global_shortcut);
    let mut active = ACTIVE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Err(error) = register_saved(&mut System(app), &mut active, saved.as_deref()) {
        log(format_args!(
            "the saved shortcut {} is not in use: {error}",
            saved.as_deref().unwrap_or_default()
        ));
    }
}

/// `update_settings` for the window: a patch that changes the shortcut
/// registers it before it is saved, and fails with a sentence, with nothing
/// changed, if that cannot be done.
pub fn update_settings(
    app: &AppHandle,
    core: &Arc<AppCore>,
    patch: &SettingsPatch,
) -> Result<Settings, String> {
    let wanted = match wanted(&patch.global_shortcut)? {
        Wanted::Keep => return core.update_settings(patch),
        Wanted::Clear => None,
        Wanted::Set(shortcut) => Some(shortcut),
    };
    let mut active = ACTIVE.lock().unwrap_or_else(PoisonError::into_inner);
    let result = change(&mut System(app), &mut active, wanted, || {
        core.update_settings(patch)
    });
    // A release that an unregistered shortcut never reports must not leave
    // the next shortcut looking held.
    HELD.store(false, Ordering::SeqCst);
    result
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use tauri_plugin_global_shortcut::{Code, Modifiers};

    use super::*;

    fn key(mods: Modifiers, code: Code) -> Shortcut {
        Shortcut::new(Some(mods), code)
    }

    fn shift_cmd_r() -> Shortcut {
        key(Modifiers::SHIFT | Modifiers::SUPER, Code::KeyR)
    }

    fn alt_f5() -> Shortcut {
        key(Modifiers::ALT, Code::F5)
    }

    /// The format of the Settings recorder (issue 019), read by the
    /// plugin's own parser with no conversion.
    #[test]
    fn the_settings_format_parses_with_the_plugins_parser() {
        let cases = [
            ("Shift+Command+KeyR", shift_cmd_r()),
            (
                "Control+Alt+Space",
                key(Modifiers::CONTROL | Modifiers::ALT, Code::Space),
            ),
            ("Alt+F5", alt_f5()),
            (
                "Control+Alt+Shift+Command+Digit1",
                key(
                    Modifiers::CONTROL | Modifiers::ALT | Modifiers::SHIFT | Modifiers::SUPER,
                    Code::Digit1,
                ),
            ),
            (
                "Control+Backquote",
                key(Modifiers::CONTROL, Code::Backquote),
            ),
            ("Command+ArrowUp", key(Modifiers::SUPER, Code::ArrowUp)),
            ("Shift+NumpadAdd", key(Modifiers::SHIFT, Code::NumpadAdd)),
        ];
        for (text, expected) in cases {
            let by_plugin = Shortcut::from_str(text)
                .unwrap_or_else(|error| panic!("the plugin does not parse {text}: {error}"));
            assert_eq!(by_plugin, expected, "{text}");
            assert_eq!(parse(text), Ok(expected), "{text}");
        }
    }

    #[test]
    fn an_invalid_shortcut_is_refused_with_a_sentence() {
        let invalid = [
            "",
            "Shift+",
            "Shift+Command",
            "KeyR+Shift",
            "Shift+KeyR+KeyT",
            "Shift+NotAKey",
            "Hyper+KeyR",
            // No modifier: it would take the key away from every app.
            "KeyR",
            "F5",
        ];
        for text in invalid {
            let error = parse(text).expect_err(text);
            assert!(error.ends_with('.'), "{error}");
            assert!(error.contains("Control, Alt, Shift or Command"), "{error}");
            assert!(!error.contains("HotKey") && !error.contains("github"));
        }
    }

    #[test]
    fn a_patch_keeps_clears_or_sets_the_shortcut() {
        let patch = |json: &str| {
            serde_json::from_str::<SettingsPatch>(json)
                .unwrap()
                .global_shortcut
        };
        assert_eq!(wanted(&patch(r#"{"language":"en"}"#)), Ok(Wanted::Keep));
        assert_eq!(
            wanted(&patch(r#"{"globalShortcut":null}"#)),
            Ok(Wanted::Clear)
        );
        assert_eq!(
            wanted(&patch(r#"{"globalShortcut":"  "}"#)),
            Ok(Wanted::Clear)
        );
        assert_eq!(
            wanted(&patch(r#"{"globalShortcut":" Shift+Command+KeyR "}"#)),
            Ok(Wanted::Set(shift_cmd_r()))
        );
        assert!(wanted(&patch(r#"{"globalShortcut":"Shift+Nope"}"#)).is_err());
    }

    #[test]
    fn only_the_first_press_toggles() {
        use ShortcutState::{Pressed, Released};
        // (held before, event) -> (held after, toggles)
        assert_eq!(press(false, Pressed), (true, true));
        // Key repeat: pressed again with no release in between.
        assert_eq!(press(true, Pressed), (true, false));
        assert_eq!(press(true, Released), (false, false));
        assert_eq!(press(false, Released), (false, false));

        let mut held = false;
        let mut toggles = 0;
        for state in [Pressed, Pressed, Pressed, Released, Pressed, Released] {
            let (now, toggle) = press(held, state);
            held = now;
            toggles += u32::from(toggle);
        }
        assert_eq!(toggles, 2);
    }

    /// A system where the shortcuts in `taken` belong to someone else.
    #[derive(Default)]
    struct Fake {
        registered: Vec<Shortcut>,
        taken: Vec<Shortcut>,
        stuck: Vec<Shortcut>,
        calls: Vec<(&'static str, Shortcut)>,
    }

    impl Registrar for Fake {
        fn register(&mut self, shortcut: Shortcut) -> Result<(), String> {
            self.calls.push(("register", shortcut));
            if self.taken.contains(&shortcut) || self.registered.contains(&shortcut) {
                return Err(format!("HotKey already registered: {shortcut:?}"));
            }
            self.registered.push(shortcut);
            Ok(())
        }

        fn unregister(&mut self, shortcut: Shortcut) -> Result<(), String> {
            self.calls.push(("unregister", shortcut));
            if self.stuck.contains(&shortcut) {
                return Err(format!("Failed to unregister hotkey: {shortcut:?}"));
            }
            self.registered.retain(|other| *other != shortcut);
            Ok(())
        }
    }

    /// A system with `old` registered, as after a launch with it saved.
    fn with(old: Shortcut) -> (Fake, Option<Shortcut>) {
        let mut fake = Fake::default();
        let mut active = None;
        register_saved(&mut fake, &mut active, Some("Alt+F5")).unwrap();
        assert_eq!(active, Some(old));
        fake.calls.clear();
        (fake, active)
    }

    #[test]
    fn nothing_is_registered_at_launch_without_a_saved_shortcut() {
        let mut fake = Fake::default();
        let mut active = None;
        assert_eq!(register_saved(&mut fake, &mut active, None), Ok(()));
        assert!(fake.calls.is_empty());
        assert_eq!(active, None);
    }

    #[test]
    fn the_saved_shortcut_is_registered_at_launch() {
        let mut fake = Fake::default();
        let mut active = None;
        register_saved(&mut fake, &mut active, Some("Shift+Command+KeyR")).unwrap();
        assert_eq!(fake.registered, [shift_cmd_r()]);
        assert_eq!(active, Some(shift_cmd_r()));
    }

    #[test]
    fn a_saved_shortcut_that_cannot_be_used_is_an_error_at_launch_and_no_more() {
        let mut fake = Fake {
            taken: vec![shift_cmd_r()],
            ..Fake::default()
        };
        let mut active = None;
        assert!(register_saved(&mut fake, &mut active, Some("Shift+Command+KeyR")).is_err());
        assert!(register_saved(&mut fake, &mut active, Some("not a shortcut")).is_err());
        assert!(fake.registered.is_empty());
        assert_eq!(active, None);
    }

    #[test]
    fn a_first_shortcut_is_registered_then_saved() {
        let mut fake = Fake::default();
        let mut active = None;
        let saved = change(&mut fake, &mut active, Some(shift_cmd_r()), || Ok("saved"));
        assert_eq!(saved, Ok("saved"));
        assert_eq!(fake.registered, [shift_cmd_r()]);
        assert_eq!(active, Some(shift_cmd_r()));
    }

    #[test]
    fn a_new_shortcut_is_registered_before_the_old_one_goes() {
        let (mut fake, mut active) = with(alt_f5());
        let order = RefCell::new(Vec::new());
        let saved = change(&mut fake, &mut active, Some(shift_cmd_r()), || {
            order.borrow_mut().push("save");
            Ok(())
        });
        assert_eq!(saved, Ok(()));
        assert_eq!(
            fake.calls,
            [("register", shift_cmd_r()), ("unregister", alt_f5())]
        );
        // Saved once; the two tests below show it comes after both steps.
        assert_eq!(*order.borrow(), ["save"]);
        assert_eq!(fake.registered, [shift_cmd_r()]);
        assert_eq!(active, Some(shift_cmd_r()));
    }

    #[test]
    fn a_failed_registration_leaves_the_old_shortcut_registered_and_saved() {
        let (mut fake, mut active) = with(alt_f5());
        fake.taken.push(shift_cmd_r());
        // The setting as the file has it.
        let setting = RefCell::new(Some("Alt+F5"));
        let result = change(&mut fake, &mut active, Some(shift_cmd_r()), || {
            *setting.borrow_mut() = Some("Shift+Command+KeyR");
            Ok(())
        });
        assert_eq!(result, Err(NOT_REGISTERED.to_owned()));
        assert_eq!(*setting.borrow(), Some("Alt+F5"));
        assert_eq!(fake.registered, [alt_f5()]);
        assert_eq!(fake.calls, [("register", shift_cmd_r())]);
        assert_eq!(active, Some(alt_f5()));
    }

    #[test]
    fn clearing_unregisters_and_saves() {
        let (mut fake, mut active) = with(alt_f5());
        assert_eq!(
            change(&mut fake, &mut active, None, || Ok("saved")),
            Ok("saved")
        );
        assert_eq!(fake.calls, [("unregister", alt_f5())]);
        assert!(fake.registered.is_empty());
        assert_eq!(active, None);
    }

    #[test]
    fn the_same_shortcut_again_is_only_saved() {
        let (mut fake, mut active) = with(alt_f5());
        assert_eq!(
            change(&mut fake, &mut active, Some(alt_f5()), || Ok("saved")),
            Ok("saved")
        );
        assert!(fake.calls.is_empty());
        assert_eq!(active, Some(alt_f5()));

        let mut none = None;
        assert_eq!(
            change(&mut fake, &mut none, None, || Ok("saved")),
            Ok("saved")
        );
        assert!(fake.calls.is_empty());
    }

    #[test]
    fn a_failed_save_puts_the_old_shortcut_back() {
        let (mut fake, mut active) = with(alt_f5());
        let result: Result<(), String> =
            change(&mut fake, &mut active, Some(shift_cmd_r()), || {
                Err("The settings could not be saved.".to_owned())
            });
        assert_eq!(result, Err("The settings could not be saved.".to_owned()));
        assert_eq!(fake.registered, [alt_f5()]);
        assert_eq!(active, Some(alt_f5()));

        // The same for a failed save of "none".
        let result: Result<(), String> =
            change(&mut fake, &mut active, None, || Err("No.".to_owned()));
        assert!(result.is_err());
        assert_eq!(fake.registered, [alt_f5()]);
        assert_eq!(active, Some(alt_f5()));
    }

    #[test]
    fn an_old_shortcut_that_cannot_be_removed_stays_and_nothing_is_saved() {
        let (mut fake, mut active) = with(alt_f5());
        fake.stuck.push(alt_f5());
        let result: Result<(), String> =
            change(&mut fake, &mut active, Some(shift_cmd_r()), || {
                panic!("must not save")
            });
        assert_eq!(result, Err(NOT_REMOVED.to_owned()));
        assert_eq!(fake.registered, [alt_f5()]);
        assert_eq!(active, Some(alt_f5()));
    }
}
