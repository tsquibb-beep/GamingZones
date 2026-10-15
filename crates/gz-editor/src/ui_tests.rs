//! UI tests: drive the real editor UI headlessly (egui_kittest), clicking and typing the way
//! a person would, on a made-up 6144×2560 monitor like Tom's. Nothing is saved or shown.

use eframe::egui::{Key, Modifiers, Vec2};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use gz_core::edit;
use gz_core::layout::{Axis, Layout};
use gz_core::store::LayoutsFile;
use gz_core::Rect;
use gz_win::monitor::MonitorInfo;

use crate::app::{Editor, Selection};
use crate::monitors::Monitor;

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
    let editor = Editor::for_test(vec![dell()], LayoutsFile { layouts: vec![layout] }, 300);
    Harness::builder().with_size(Vec2::new(1600.0, 1000.0)).build_ui_state(|ui, ed: &mut Editor| ed.show(ui), editor)
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
    h.get_by_label("🔒 Lock size").click();
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
    h.get_by_label("🔒 Lock size").click();
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
    let mut other = three_columns();
    other.id = "other".into();
    other.name = "Other".into();
    let editor = Editor::for_test(vec![dell()], LayoutsFile { layouts: vec![three_columns(), other] }, 300);
    let mut h = Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .build_ui_state(|ui, ed: &mut Editor| ed.show(ui), editor);
    h.run_steps(2);
    // "Columns" is the default layout, so the monitor (which has no entry of its own) uses it.
    h.get_by_label("✔ In use on this monitor");
    // Deleting it is refused...
    h.get_by_label("Delete").click();
    h.run_steps(2);
    h.get_all_by_label("Delete").last().unwrap().click();
    h.run_steps(2);
    assert_eq!(h.state().doc.layouts.layouts.len(), 2);
    let (text, error) = h.state().status().cloned().unwrap();
    assert!(error && text.contains("in use on 1 · DELL"), "{text}");
    // ...but an unused one goes, and Ctrl+Z brings it back.
    h.state_mut().layout = 1;
    h.run_steps(2);
    h.get_by_label("Delete").click();
    h.run_steps(2);
    h.get_all_by_label("Delete").last().unwrap().click();
    h.run_steps(2);
    assert_eq!(h.state().doc.layouts.layouts.len(), 1);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    assert_eq!(h.state().doc.layouts.layouts.len(), 2);
}
