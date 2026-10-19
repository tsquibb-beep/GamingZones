//! UI tests for the settings window: click and type in it headlessly (egui_kittest) and
//! check what it writes to its (test) settings.json. Hotkey capture gets its key presses
//! through `fake_keys` instead of the keyboard hook.

use eframe::egui::{self, Vec2};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use gz_core::config::HotkeyName;
use gz_core::{Hotkey, Settings};

use crate::settings::{SettingsWindow, Startup};
use crate::style;

const LWIN: u16 = 0x5B;
const LCTRL: u16 = 0xA2;
const LALT: u16 = 0xA4;
const LSHIFT: u16 = 0xA0;
const ESC: u16 = 0x1B;
const TAB: u16 = 0x09;

fn harness(settings: Settings) -> Harness<'static, SettingsWindow> {
    let window = SettingsWindow::for_test(settings);
    let mut fonts_set = false;
    let mut h = Harness::builder().with_size(Vec2::new(760.0, 1600.0)).with_step_dt(0.05).build_ui_state(
        move |ui, w: &mut SettingsWindow| {
            if !std::mem::replace(&mut fonts_set, true) {
                style::install(ui.ctx());
                return;
            }
            w.show(ui)
        },
        window,
    );
    h.run_steps(2);
    h
}

/// What the window has written to settings.json.
fn written(h: &Harness<'_, SettingsWindow>) -> Settings {
    let text = std::fs::read_to_string(h.state().path()).expect("settings.json was written");
    Settings::from_json(&text).unwrap()
}

fn hk(s: &str) -> Option<Hotkey> {
    Some(s.parse().unwrap())
}

fn click(h: &mut Harness<'_, SettingsWindow>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(2);
}

/// Presses (and lets go of) `keys` in order, the way the capture hook reports them.
fn press(h: &mut Harness<'_, SettingsWindow>, keys: &[u16]) {
    let downs = keys.iter().map(|&k| (k, true));
    let ups = keys.iter().rev().map(|&k| (k, false));
    h.state_mut().fake_keys = downs.chain(ups).collect();
    h.run_steps(2);
}

#[test]
fn a_checkbox_saves_straight_away() {
    let mut h = harness(Settings::default());
    click(&mut h, "Show the zones briefly after switching layout");
    assert!(!written(&h).overlay.flash_on_switch);
    click(&mut h, "Always (hold Shift to hide them)");
    assert!(!written(&h).snapping.require_shift);
    assert_eq!(h.state().status.as_ref().map(|s| s.0.as_str()), Some("Saved."));
}

#[test]
fn capturing_a_hotkey() {
    let mut h = harness(Settings::default());
    click(&mut h, "Win+Shift+`");
    assert_eq!(h.state().is_capturing(), Some(HotkeyName::LayoutSelector));
    h.get_by_label("Press the keys…");
    press(&mut h, &[LWIN, LCTRL, 0x4C]); // Win+Ctrl+L
    assert_eq!(h.state().is_capturing(), None);
    assert_eq!(written(&h).hotkeys.layout_selector, hk("Win+Ctrl+L"));
    h.get_by_label("Win+Ctrl+L");
}

#[test]
fn a_key_without_a_modifier_waits_for_more_and_esc_cancels() {
    let mut h = harness(Settings::default());
    click(&mut h, "Win+Ctrl+Alt+G");
    press(&mut h, &[0x4B]); // K alone
    assert_eq!(h.state().is_capturing(), Some(HotkeyName::GamingSnap));
    h.get_by_label("Add Win, Ctrl or Alt.");
    press(&mut h, &[ESC]);
    assert_eq!(h.state().is_capturing(), None);
    assert_eq!(h.state().settings.hotkeys.gaming_snap, hk("Win+Ctrl+Alt+G"));
}

#[test]
fn keys_already_used_by_another_hotkey_are_refused() {
    let mut h = harness(Settings::default());
    click(&mut h, "Win+Shift+`");
    press(&mut h, &[LWIN, LCTRL, LALT, 0x47]); // the gaming snap's keys
    assert_eq!(h.state().settings.hotkeys.layout_selector, hk("Win+Shift+`"));
    h.get_by_label("Win+Ctrl+Alt+G is already used for \"Snap to gaming zone\".");
    // Quick-switch's Win+Ctrl+Alt+3 too.
    click(&mut h, "Win+Shift+`");
    press(&mut h, &[LWIN, LCTRL, LALT, 0x33]);
    assert_eq!(h.state().settings.hotkeys.layout_selector, hk("Win+Shift+`"));
}

#[test]
fn turning_hotkeys_off() {
    let mut h = harness(Settings::default());
    // The × after the gaming snap box (the second hotkey row).
    let crosses: Vec<_> = h.get_all_by_label("×").collect();
    assert_eq!(crosses.len(), 3);
    crosses[1].click();
    h.run_steps(2);
    assert_eq!(written(&h).hotkeys.gaming_snap, None);
    h.get_by_label("Off");

    click(&mut h, "Alt");
    assert_eq!(written(&h).hotkeys.quick_switch.unwrap().to_string(), "Win+Ctrl");
    click(&mut h, "On");
    assert_eq!(written(&h).hotkeys.quick_switch, None);
    // Back on: the same modifiers as before.
    click(&mut h, "On");
    assert_eq!(written(&h).hotkeys.quick_switch.unwrap().to_string(), "Win+Ctrl");
}

#[test]
fn the_last_quick_switch_modifier_stays() {
    let mut s = Settings::default();
    s.hotkeys.quick_switch = Some("Ctrl".parse().unwrap());
    let mut h = harness(s);
    click(&mut h, "Ctrl");
    assert_eq!(h.state().settings.hotkeys.quick_switch.unwrap().to_string(), "Ctrl");
    // Shift alone isn't enough either.
    click(&mut h, "Shift");
    click(&mut h, "Ctrl");
    assert_eq!(h.state().settings.hotkeys.quick_switch.unwrap().to_string(), "Ctrl+Shift");
}

#[test]
fn windows_shortcuts_and_shift_alone_are_refused() {
    let mut h = harness(Settings::default());
    click(&mut h, "Win+Shift+`");
    press(&mut h, &[LALT, TAB]);
    h.get_by_label("Alt+Tab is a Windows shortcut (switches windows). Choose other keys.");
    press(&mut h, &[LSHIFT, 0x41]); // Shift+A
    h.get_by_label("Add Win, Ctrl or Alt.");
    // Still waiting: the next try is taken.
    assert_eq!(h.state().is_capturing(), Some(HotkeyName::LayoutSelector));
    press(&mut h, &[LWIN, LCTRL, 0x4C]);
    assert_eq!(written(&h).hotkeys.layout_selector, hk("Win+Ctrl+L"));
}

#[test]
fn keys_another_app_holds_are_refused() {
    let mut h = harness(Settings::default());
    h.state_mut().fake_taken = vec!["Win+D".parse().unwrap(), "Win+Ctrl+5".parse().unwrap()];
    click(&mut h, "Win+Shift+`");
    press(&mut h, &[LWIN, 0x44]); // Win+D
    assert_eq!(h.state().settings.hotkeys.layout_selector, hk("Win+Shift+`"));
    h.get_by_label("Win+D is already used by Windows or another app.");
    // Quick-switch: Win+Ctrl+1..9 would include the taken Win+Ctrl+5.
    click(&mut h, "Alt");
    assert_eq!(h.state().settings.hotkeys.quick_switch.unwrap().to_string(), "Win+Ctrl+Alt");
    h.get_by_label("Win+Ctrl+5 is already used by Windows or another app.");
    // The keys our tray app already holds are fine.
    click(&mut h, "Win+Ctrl+Alt+G");
    press(&mut h, &[LWIN, LCTRL, LALT, 0x47]);
    assert_eq!(h.state().refused, None);
}

#[test]
fn hotkeys_the_tray_app_could_not_register_are_marked() {
    let mut h = harness(Settings::default());
    h.state_mut().failed = HotkeyName::LayoutSelector.bit();
    h.run_steps(2);
    let notes: Vec<_> = h.get_all_by_label("In use by another app or by Windows. Choose other keys.").collect();
    assert_eq!(notes.len(), 1);
    // A new hotkey for it clears the mark (until the tray app says otherwise).
    click(&mut h, "Win+Shift+`");
    press(&mut h, &[LWIN, LALT, 0x4C]);
    assert_eq!(h.state().failed, 0);
}

/// Types into the `n`th text box (the colours' hex codes, then the exclusion box), replacing
/// what's in it.
fn type_into(h: &mut Harness<'_, SettingsWindow>, n: usize, text: &str) {
    text_box(h, n).click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.key_press(egui::Key::Backspace);
    h.run_steps(1);
    for c in text.chars() {
        text_box(h, n).type_text(&c.to_string());
        h.run_steps(1);
    }
}

/// The `n`th text box (the last if there are fewer).
fn text_box<'a>(h: &'a Harness<'_, SettingsWindow>, n: usize) -> egui_kittest::Node<'a> {
    let count = h.get_all_by_role(egui::accesskit::Role::TextInput).count();
    h.get_all_by_role(egui::accesskit::Role::TextInput).nth(n.min(count - 1)).unwrap()
}

fn type_exclusion(h: &mut Harness<'_, SettingsWindow>, text: &str) {
    type_into(h, usize::MAX, text);
}

#[test]
fn typing_a_colour() {
    let mut h = harness(Settings::default());
    // The first box is the zone colour's. A whole colour applies as soon as it's typed.
    type_into(&mut h, 0, "#3b4660");
    assert_eq!(written(&h).overlay.zone_color, gz_core::Color::rgb(0x3B, 0x46, 0x60));
    // Without the # too (the highlight colour, the third).
    type_into(&mut h, 2, "006ED0");
    assert_eq!(written(&h).overlay.highlight_color, gz_core::Color::rgb(0x00, 0x6E, 0xD0));
    // Half a colour changes nothing, and leaving the box shows the colour again.
    type_into(&mut h, 1, "#12");
    click(&mut h, "Show zone numbers");
    assert_eq!(written(&h).overlay.border_color, gz_core::Color::rgb(0xFF, 0xFF, 0xFF));
    assert_eq!(text_box(&h, 1).value().as_deref(), Some("#FFFFFF"));
}

#[test]
fn excluded_apps() {
    let mut h = harness(Settings::default());
    h.get_by_label("None.");
    type_exclusion(&mut h, "Game.exe");
    click(&mut h, "Add");
    assert_eq!(written(&h).snapping.excluded_apps, ["Game.exe"]);
    // Twice (any case) is once.
    type_exclusion(&mut h, "game.EXE");
    click(&mut h, "Add");
    assert_eq!(written(&h).snapping.excluded_apps, ["Game.exe"]);
    // The × of the first app is the fourth (after three hotkeys).
    h.get_all_by_label("×").nth(3).unwrap().click();
    h.run_steps(2);
    assert!(written(&h).snapping.excluded_apps.is_empty());
}

#[test]
fn choices_and_numbers() {
    let mut h = harness(Settings::default());
    click(&mut h, "Hidden");
    assert_eq!(written(&h).overlay.disabled_zones, gz_core::DisabledZones::Hidden);
    click(&mut h, "Centre it, keep its size");
    assert_eq!(written(&h).snapping.gaming_fit, gz_core::GamingFit::Centre);
    // The overlap rule is hidden until zones can overlap: the only drop-down is the log level.
    assert_eq!(h.get_all_by_role(egui::accesskit::Role::ComboBox).count(), 1);
    assert!(h.query_by_label("Overlapping zones").is_none());
    click(&mut h, "Snap to both zones, within");
    assert!(written(&h).snapping.edge_span);
    click(&mut h, "Start it when I sign in");
    assert_eq!(h.state().startup, Startup::SignIn);
}
