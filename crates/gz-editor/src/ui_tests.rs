//! UI tests: drive the real editor UI headlessly (egui_kittest), clicking and typing the way
//! a person would, on a made-up 6144×2560 monitor like Tom's. Nothing is saved or shown.

use eframe::egui::accesskit::Role;
use eframe::egui::{self, Event, Key, Modifiers, PointerButton, Pos2, Vec2};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use gz_core::edit;
use gz_core::layout::{Axis, Layout};
use gz_core::store::LayoutsFile;
use gz_core::Rect;
use gz_win::monitor::MonitorInfo;

use crate::app::{Dialog, Editor, Selection};
use crate::canvas;
use crate::monitors::Monitor;
use crate::style;

const WORK: Rect = Rect::new(0, 0, 6144, 2500);
const MONITOR: Rect = Rect::new(0, 0, 6144, 2560);

fn dell() -> Monitor {
    Monitor {
        info: MonitorInfo { device: r"\\.\DISPLAY1".into(), rect: MONITOR, work: WORK, dpi: 120, primary: true },
        id: "TEST-DELL".into(),
        name: "DELL".into(),
    }
}

/// Three columns made by clicking (split at 2000, then at 4000): dividers g1 and g2.
fn three_columns() -> Layout {
    let mut l = edit::new_layout("cols", "Columns");
    edit::split(&mut l, 0, Axis::Vertical, 2000, WORK, MONITOR).unwrap();
    edit::split(&mut l, 1, Axis::Vertical, 4000, WORK, MONITOR).unwrap();
    l
}

fn harness(layout: Layout) -> Harness<'static, Editor> {
    harness_with(vec![layout])
}

fn harness_with(layouts: Vec<Layout>) -> Harness<'static, Editor> {
    let editor = Editor::for_test(vec![dell()], LayoutsFile { layouts }, 300);
    // The fonts apply from the frame after they're set, so the first frame only sets them.
    let mut fonts_set = false;
    let mut h = Harness::builder().with_size(Vec2::new(1760.0, 760.0)).with_step_dt(0.05).build_ui_state(
        move |ui, ed: &mut Editor| {
            if !std::mem::replace(&mut fonts_set, true) {
                style::install(ui.ctx());
                return;
            }
            ed.show(ui)
        },
        editor,
    );
    h.run_steps(2);
    h
}

/// Three columns, and another layout ("Other", the same zones).
fn two_layouts() -> Vec<Layout> {
    let mut other = three_columns();
    other.id = "other".into();
    other.name = "Other".into();
    vec![three_columns(), other]
}

/// What the editor has written to its (test) layouts.json.
fn written(h: &Harness<'_, Editor>) -> LayoutsFile {
    let text = std::fs::read_to_string(h.state().files.layouts_path()).expect("layouts.json was written");
    LayoutsFile::from_json(&text).unwrap()
}

/// Presses and releases the left button at `p` (screen points).
fn click_at(h: &mut Harness<'_, Editor>, p: Pos2) {
    h.hover_at(p);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    }
    h.run_steps(1);
}

/// Types `text` one key per frame.
fn type_text(h: &mut Harness<'_, Editor>, text: &str) {
    for c in text.chars() {
        h.event(Event::Text(c.to_string()));
        h.run_steps(1);
    }
}

/// Where zone `i` of the open layout is on the canvas (points).
fn zone_on_canvas(h: &Harness<'_, Editor>, i: usize) -> egui::Rect {
    let r = h.state().current().resolve(WORK, MONITOR, 0).unwrap()[i].rect;
    canvas::map_rect(r, MONITOR, h.state().canvas_monitor)
}

/// The layout card named `name`.
fn card<'h>(h: &'h Harness<'_, Editor>, name: &'h str) -> egui_kittest::Node<'h> {
    h.get_by_role_and_label(Role::Button, name)
}

/// Opens the right-click menu of the layout card named `name`.
fn card_menu(h: &mut Harness<'_, Editor>, name: &str) {
    card(h, name).click_secondary();
    h.run_steps(2);
}

fn widths(h: &Harness<'_, Editor>) -> Vec<i32> {
    let ed = h.state();
    ed.current().resolve(WORK, MONITOR, 0).unwrap().iter().filter(|z| !z.gaming).map(|z| z.rect.width()).collect()
}

/// Clicks the number box labelled `label`, replaces its text with `text` typed one key at a
/// time, and presses Enter (or not).
fn type_into(h: &mut Harness<'_, Editor>, label: &str, text: &str, enter: bool) {
    h.get_by_label(label).click();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    // One key at a time, with a frame between, as a person types.
    for c in text.chars() {
        h.get_by_label(label).type_text(&c.to_string());
        h.run_steps(1);
    }
    if enter {
        h.key_press(Key::Enter);
    }
    h.run_steps(2);
}

fn select(h: &mut Harness<'_, Editor>, selection: Selection) {
    h.state_mut().selection = selection;
    h.run_steps(2);
}

#[test]
fn typing_a_divider_percentage_lands_exactly() {
    // Tom's case: typing 79 jumped to 99.837 %, because each keystroke applied on its own.
    let mut h = harness(three_columns());
    select(&mut h, Selection::Divider("g2".into()));
    type_into(&mut h, "% of the width", "79", true);
    let positions = h.state().current().guide_positions(WORK, MONITOR).unwrap();
    assert_eq!(positions["g2"].1, 4854); // 79 % of 6144 = 4853.76
    assert_eq!(positions["g1"].1, 2000, "the other divider stays put");
    assert_eq!(h.state().status(), None);
}

#[test]
fn nothing_applies_until_enter() {
    let mut h = harness(three_columns());
    select(&mut h, Selection::Divider("g2".into()));
    type_into(&mut h, "% of the width", "7", false);
    assert_eq!(h.state().current().guide_positions(WORK, MONITOR).unwrap()["g2"].1, 4000);
    h.key_press(Key::Enter);
    h.run_steps(2);
    // 7 % (430 px) would squash zone 2 to nothing, so it stops 10 px past the first divider.
    assert_eq!(h.state().current().guide_positions(WORK, MONITOR).unwrap()["g2"].1, 2010);
}

#[test]
fn typing_a_divider_position_in_px() {
    let mut h = harness(three_columns());
    select(&mut h, Selection::Divider("g1".into()));
    type_into(&mut h, "From the left", "1290", true);
    assert_eq!(widths(&h), [1290, 2710, 2144]);
}

#[test]
fn thirds_with_a_locked_zone() {
    // Tom's 33 % / 34 % / 33 %: type the outer zones' widths, locking the first.
    let mut h = harness(three_columns());
    select(&mut h, Selection::Zones(vec![0]));
    type_into(&mut h, "% of the width", "33", true);
    assert_eq!(widths(&h), [2028, 1972, 2144]);
    h.get_by_label("Lock").click();
    h.run_steps(2);
    assert!(h.state().is_locked(0));
    select(&mut h, Selection::Zones(vec![2]));
    type_into(&mut h, "% of the width", "33", true);
    assert_eq!(widths(&h), [2028, 2088, 2028]); // 33 % / 33.98 % / 33 %
}

#[test]
fn locked_zones_are_pushed() {
    // Tom: lock a zone, resize its neighbour, and the locked zone is pushed into the next one.
    let mut h = harness(three_columns());
    select(&mut h, Selection::Zones(vec![1]));
    h.get_by_label("Lock").click();
    h.run_steps(2);
    assert!(h.state().is_locked(1));
    // Its own size boxes are off while it's locked.
    assert!(h.get_by_label("Width").accesskit_node().is_disabled());
    select(&mut h, Selection::Zones(vec![0]));
    type_into(&mut h, "% of the width", "33", true);
    assert_eq!(widths(&h), [2028, 2000, 2116]);
    // Dragging the divider the other way pulls it back.
    h.state_mut().move_divider("g1", 1500);
    h.run_steps(2);
    assert_eq!(widths(&h), [1500, 2000, 2644]);
    assert_eq!(h.state().status(), None);
    // A locked zone against the screen's edge has nowhere to go.
    h.state_mut().set_locked(0, true);
    h.state_mut().move_divider("g1", 1700);
    h.run_steps(2);
    assert_eq!(widths(&h), [1500, 2000, 2644]);
    let (text, error) = h.state().status().cloned().unwrap();
    assert!(error && text.contains("Zone 1 is locked"), "{text}");
}

#[test]
fn making_a_zone_the_gaming_zone() {
    // Tom's Slay test: the middle zone and all its space becomes the gaming zone.
    let mut h = harness(three_columns());
    assert!(h.query_by_label("Make zone 2 the gaming zone").is_none());
    select(&mut h, Selection::Zones(vec![1]));
    h.get_by_label("Make zone 2 the gaming zone").click();
    h.run_steps(2);
    let rects = |h: &Harness<'_, Editor>| -> Vec<Rect> {
        h.state().current().resolve(WORK, MONITOR, 0).unwrap().iter().map(|z| z.rect).collect()
    };
    let r = rects(&h);
    assert_eq!(r.last(), Some(&Rect::new(2000, 0, 4000, 2500)));
    assert_eq!((r[0], r[5]), (Rect::new(0, 0, 2000, 2500), Rect::new(4000, 0, 6144, 2500)));
    assert_eq!(h.state().selection, Selection::Gaming);
    // The other columns are zones 1 and 2.
    assert_eq!((h.state().zone_name(0), h.state().zone_name(5)), ("1".into(), "2".into()));

    // A preset size: the rest of the column becomes zones around it.
    h.get_by_value("Presets").click();
    h.run_steps(2);
    h.get_by_label("1920 × 1080").click();
    h.run_steps(2);
    let r = rects(&h);
    assert_eq!(r[6], Rect::new(2040, 710, 3960, 1790));
    assert_eq!(r[1], Rect::new(2000, 0, 4000, 710)); // above
    assert_eq!(r[2], Rect::new(2000, 710, 2040, 1790)); // left
    assert_eq!(r[3], Rect::new(3960, 710, 4000, 1790)); // right
    assert_eq!(r[4], Rect::new(2000, 1790, 4000, 2500)); // below
    h.get_by_label(
        "Its space: 2000 × 2500. What it doesn't cover is normal zones; the dividers around it move with it.",
    );

    // Back to filling its space, then back to a normal zone: the original layout.
    h.get_by_label("Fill its space").click();
    h.run_steps(2);
    assert_eq!(rects(&h)[6], Rect::new(2000, 0, 4000, 2500));
    h.get_by_label("Turn it back into a normal zone").click();
    h.run_steps(2);
    assert_eq!(
        rects(&h),
        three_columns().resolve(WORK, MONITOR, 0).unwrap().iter().map(|z| z.rect).collect::<Vec<_>>()
    );
}

#[test]
fn the_gaming_zone_is_pushed_not_resized() {
    // Tom: sizing up the zones around it resized it; moving it didn't push its neighbours.
    let mut h = harness(three_columns());
    select(&mut h, Selection::Zones(vec![1]));
    h.get_by_label("Make zone 2 the gaming zone").click();
    h.run_steps(2);
    let gaming = |h: &Harness<'_, Editor>| h.state().current().gaming_rects(WORK, MONITOR).unwrap()[0];
    // Typing the left zone wider pushes it along, same size.
    select(&mut h, Selection::Zones(vec![0]));
    type_into(&mut h, "Width", "2300", true);
    assert_eq!(gaming(&h), Rect::new(2300, 0, 4300, 2500));
    assert_eq!((widths(&h)[0], widths(&h)[5]), (2300, 1844));
    // Dragging it back left: the left zone shrinks, the right one grows.
    h.state_mut().move_gaming_zone(1800, 0);
    h.run_steps(2);
    assert_eq!(gaming(&h), Rect::new(1800, 0, 3800, 2500));
    assert_eq!((widths(&h)[0], widths(&h)[5]), (1800, 2344));
    assert_eq!(h.state().status(), None);
}

#[test]
fn undo_and_redo() {
    let mut h = harness(three_columns());
    select(&mut h, Selection::Divider("g1".into()));
    type_into(&mut h, "From the left", "1000", true);
    assert_eq!(widths(&h)[0], 1000);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    assert_eq!(widths(&h)[0], 2000);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    h.run_steps(2);
    assert_eq!(widths(&h)[0], 1000);
}

#[test]
fn deleting_layouts() {
    let mut h = harness_with(two_layouts());
    // "Columns" is the default layout, so the monitor (which has no entry of its own) uses it.
    h.get_by_label("Used on:");
    assert!(h.query_by_label("Use on this monitor").is_none());
    // Deleting it is refused...
    card_menu(&mut h, "Columns");
    h.get_by_label("Delete…").click();
    h.run_steps(2);
    h.get_by_label("Delete").click();
    h.run_steps(2);
    assert_eq!(h.state().doc.layouts.layouts.len(), 2);
    let (text, error) = h.state().status().cloned().unwrap();
    assert!(error && text.contains("in use on 1 · DELL"), "{text}");
    // ...but an unused one goes, and straight into the file.
    card_menu(&mut h, "Other");
    h.get_by_label("Delete…").click();
    h.run_steps(2);
    h.get_by_label("Delete").click();
    h.run_steps(2);
    assert_eq!(h.state().doc.layouts.layouts.len(), 1);
    assert_eq!(written(&h).layouts.len(), 1);
    assert_eq!(h.state().current().name, "Columns");
}

#[test]
fn switching_layouts_with_unsaved_changes_asks_first() {
    let mut h = harness_with(two_layouts());
    h.state_mut().move_divider("g1", 1000);
    h.run_steps(2);
    assert!(h.state().dirty());
    // Cancel: stay, changes kept.
    card(&h, "Other").click();
    h.run_steps(2);
    assert!(matches!(h.state().dialog, Some(Dialog::UnsavedOnSwitch(_))));
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert_eq!((h.state().layout, widths(&h)[0]), (0, 1000));
    // Discard: on to the other layout; the change is gone.
    card(&h, "Other").click();
    h.run_steps(2);
    // The dialog's button, not the one under the canvas.
    h.get_all_by_label("Discard").last().unwrap().click();
    h.run_steps(2);
    assert_eq!(h.state().layout, 1);
    assert_eq!(h.state().doc.layouts.layouts[0], three_columns());
    assert!(!h.state().dirty());
    // Save: written, then on.
    h.state_mut().move_divider("g2", 5000);
    h.run_steps(2);
    card(&h, "Columns").click();
    h.run_steps(2);
    h.get_all_by_label("Save").last().unwrap().click();
    h.run_steps(2);
    assert_eq!(h.state().layout, 0);
    let saved = &written(&h).layouts[1];
    assert_eq!(saved.guide_positions(WORK, MONITOR).unwrap()["g2"].1, 5000);
    assert!(!h.state().dirty());
}

#[test]
fn save_and_discard_under_the_canvas() {
    let mut h = harness(three_columns());
    assert!(h.query_by_label("Save").is_none(), "nothing to save yet");
    h.state_mut().move_divider("g1", 1000);
    h.run_steps(2);
    h.get_by_label("Discard").click();
    h.run_steps(2);
    assert_eq!(widths(&h)[0], 2000);
    assert!(!h.state().dirty());
    assert!(h.query_by_label("Discard").is_none());
    // Nothing to undo after a discard.
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    assert_eq!(widths(&h)[0], 2000);

    h.state_mut().move_divider("g1", 1500);
    h.run_steps(2);
    h.get_by_label("Save").click();
    h.run_steps(2);
    assert!(!h.state().dirty());
    assert_eq!(written(&h).layouts[0].guide_positions(WORK, MONITOR).unwrap()["g1"].1, 1500);
}

#[test]
fn renaming_a_layout() {
    let mut h = harness_with(two_layouts());
    // Double-click its name.
    let r = card(&h, "Other").rect();
    let name = egui::pos2(r.left() + 30.0, r.bottom() - 12.0);
    h.hover_at(name);
    h.run_steps(1);
    for pressed in [true, false, true, false] {
        h.event(Event::PointerButton {
            pos: name,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    h.run_steps(2);
    assert!(h.state().renaming.is_some());
    type_text(&mut h, "Wide");
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state().doc.layouts.layouts[1].name, "Wide");
    assert_eq!(written(&h).layouts[1].name, "Wide");
    assert_eq!(written(&h).layouts[1].id, "other", "the id doesn't change");
    // From the menu; Esc changes nothing.
    card_menu(&mut h, "Wide");
    h.get_by_label("Rename").click();
    h.run_steps(2);
    type_text(&mut h, "Nope");
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().doc.layouts.layouts[1].name, "Wide");
    assert!(h.state().renaming.is_none());
}

#[test]
fn duplicating_and_using_a_layout_on_a_monitor() {
    let mut h = harness_with(two_layouts());
    card_menu(&mut h, "Other");
    h.get_by_label("Duplicate").click();
    h.run_steps(2);
    assert_eq!(h.state().current().name, "Other copy");
    assert_eq!(written(&h).layouts.len(), 3);
    card_menu(&mut h, "Other");
    let menu = h
        .get_all_by_label_contains("Use on")
        .find(|n| n.accesskit_node().label().as_deref() != Some("Use on this monitor"));
    menu.expect("the Use on menu").hover();
    h.run_steps(2);
    // The submenu's entry (the monitor's button in the top bar is the first).
    h.get_all_by_label_contains("1 · DELL").last().unwrap().click();
    h.run_steps(2);
    assert_eq!(h.state().doc.applied.layout_for("TEST-DELL"), Some("other"));
    let applied = std::fs::read_to_string(h.state().files.applied_path()).unwrap();
    assert!(applied.contains("\"other\""), "{applied}");
}

#[test]
fn a_zones_padlock_locks_it() {
    let mut h = harness(three_columns());
    let zone = zone_on_canvas(&h, 1);
    let lock = canvas::lock_rect(zone).expect("room for a padlock");
    click_at(&mut h, lock.center());
    assert!(h.state().is_locked(1));
    assert_eq!(widths(&h), [2000, 2000, 2144], "clicking the padlock doesn't split the zone");
    click_at(&mut h, lock.center());
    assert!(!h.state().is_locked(1));
    // Elsewhere in the zone, a click splits it.
    click_at(&mut h, zone.center());
    assert_eq!(widths(&h).len(), 4);
}

#[test]
fn gaming_zone_options_only_while_it_is_selected() {
    let mut l = three_columns();
    edit::make_gaming_zone(&mut l, 1, WORK, MONITOR).unwrap();
    let mut h = harness(l);
    assert!(h.query_by_label("Turn it back into a normal zone").is_none());
    select(&mut h, Selection::Zones(vec![0]));
    assert!(h.query_by_label("Turn it back into a normal zone").is_none());
    assert!(h.query_by_label_contains("the gaming zone").is_none(), "there's one already");
    select(&mut h, Selection::Gaming);
    h.get_by_label("Turn it back into a normal zone");
}
