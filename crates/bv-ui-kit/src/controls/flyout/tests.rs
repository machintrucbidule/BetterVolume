use super::*;
use crate::controls::device_icon::DeviceIcon;
use crate::icons::VolumeLevel;
use crate::tokens::typography::TextStyle;

/// Every character is 7 DIPs wide; a line holds as many as fit.
struct Fixed;

impl TextMeasure for Fixed {
    fn width(&self, text: &str, _: TextStyle) -> f32 {
        text.chars().count() as f32 * 7.0
    }

    fn lines(&self, text: &str, _: TextStyle, width: f32) -> u32 {
        let per_line = (width / 7.0).floor().max(1.0) as usize;
        text.chars().count().div_ceil(per_line).max(1) as u32
    }
}

fn row(target: SliderTarget, label: &str, volume: u8) -> VolumeControl {
    VolumeControl {
        target,
        label: label.into(),
        icon: VolumeIcon::App(None),
        icon_source: None,
        volume,
        muted: false,
        enabled: true,
        mute_name: format!("Mute {label}"),
        mute_tooltip: "Mute".into(),
        slider_name: format!("{label} volume"),
    }
}

fn tile(key: u64, name: &str, current: bool) -> DeviceTile {
    DeviceTile {
        key: ItemKey(key),
        name: name.into(),
        icon: DeviceIcon::GenericOutput,
        current,
        calls: None,
    }
}

fn content(tiles: usize, apps: usize, expanded: bool) -> FlyoutContent {
    FlyoutContent {
        name: "BetterVolume".into(),
        tabs: Some(Tabs {
            name: "View".into(),
            selected: FlyoutTab::Output,
            output: "Output".into(),
            input: "Microphone".into(),
        }),
        devices: Devices {
            name: "Output devices".into(),
            tiles: (0..tiles).map(|i| tile(i as u64 + 1, &format!("Device {i}"), i == 0)).collect(),
            empty: "No device".into(),
        },
        apps: Some(AppsSection {
            title: "Applications".into(),
            expanded,
            rows: (0..apps)
                .map(|i| row(SliderTarget::App(ItemKey(100 + i as u64)), &format!("App {i}"), 50))
                .collect(),
        }),
        main: VolumeControl {
            icon: VolumeIcon::Level(VolumeLevel::Medium),
            ..row(SliderTarget::Main, "Speakers", 40)
        },
        footer: Footer {
            sound_settings: "Sound settings".into(),
            settings: "Settings".into(),
            settings_enabled: false,
        },
        update: None,
        wheel_step: 2,
        // Most tests check the panel fitting its content; the full height has its own.
        height: PanelHeight::Content,
    }
}

fn view(c: FlyoutContent) -> FlyoutView {
    FlyoutView::new(c, &Fixed, 1.0, 2000.0)
}

fn centre(view: &FlyoutView, key: NodeKey) -> Point {
    let r = view.node_rect(key).expect("node");
    Point::new(r.x + r.width / 2.0, r.y + r.height / 2.0)
}

#[test]
fn height_follows_the_content() {
    let small = view(content(2, 0, true)).height();
    let more_tiles = view(content(5, 0, true)).height();
    let with_apps = view(content(2, 3, true)).height();
    let collapsed = view(content(2, 3, false)).height();
    assert!(more_tiles > small, "a second row of tiles");
    assert!(with_apps > small);
    assert!(collapsed < with_apps, "collapsed rows take no room");
}

#[test]
fn tiles_fill_each_row() {
    let v = view(content(5, 0, true));
    let rects: Vec<Rect> =
        (1..=5).map(|k| v.node_rect(NodeKey::Tile(ItemKey(k))).unwrap()).collect();
    // 5 = 3 + 2: both rows span the same width.
    let row_width = |a: Rect, b: Rect| b.right() - a.x;
    assert!((row_width(rects[0], rects[2]) - row_width(rects[3], rects[4])).abs() < 0.01);
    assert!(rects[3].width > rects[0].width);
    assert!(
        rects[3].y >= rects[0].bottom() + crate::tokens::tile::GAP - 0.01,
        "rows never overlap"
    );
}

#[test]
fn a_click_on_a_tile_asks_for_the_device() {
    let mut v = view(content(3, 0, true));
    let p = centre(&v, NodeKey::Tile(ItemKey(2)));
    v.pointer_move(p);
    v.press(p);
    let r = v.release(p);
    assert_eq!(r.action, Some(FlyoutAction::SelectDevice(ItemKey(2))));
    // Pressed on one tile, released on another: nothing.
    let q = centre(&v, NodeKey::Tile(ItemKey(3)));
    v.press(p);
    assert_eq!(v.release(q).action, None);
}

#[test]
fn a_slider_jumps_to_the_click_and_follows_the_drag() {
    let mut v = view(content(1, 0, true));
    let rect = v.node_rect(NodeKey::Slider(SliderTarget::Main)).unwrap();
    let start = Point::new(rect.x + THUMB / 2.0, rect.y + rect.height / 2.0);
    let r = v.press(start);
    assert_eq!(r.capture, Some(true));
    assert_eq!(r.action, Some(FlyoutAction::SetVolume(SliderTarget::Main, 0)));
    let end = Point::new(rect.right() - THUMB / 2.0, start.y);
    assert_eq!(v.pointer_move(end).action, Some(FlyoutAction::SetVolume(SliderTarget::Main, 100)));
    // Far outside the slider: still the drag, clamped.
    assert_eq!(v.pointer_move(Point::new(9999.0, 0.0)).action, None, "already at 100");
    let r = v.release(end);
    assert_eq!(r.capture, Some(false));
    assert_eq!(v.content().main.volume, 100);
}

#[test]
fn the_wheel_acts_only_on_sliders() {
    let mut v = view(content(1, 1, true));
    let p = centre(&v, NodeKey::Slider(SliderTarget::Main));
    assert_eq!(v.wheel(p, 1).action, Some(FlyoutAction::SetVolume(SliderTarget::Main, 42)));
    assert_eq!(v.wheel(p, -3).action, Some(FlyoutAction::SetVolume(SliderTarget::Main, 36)));
    let tile = centre(&v, NodeKey::Tile(ItemKey(1)));
    assert_eq!(v.wheel(tile, 1).action, None);
}

#[test]
fn rows_do_not_move_under_a_dragged_slider() {
    let mut v = view(content(1, 2, true));
    let target = SliderTarget::App(ItemKey(101));
    let p = centre(&v, NodeKey::Slider(target));
    v.press(p);
    let before = v.node_rect(NodeKey::Slider(target));
    // A new application appears first in the list, and another row's value changes.
    let mut next = content(1, 2, true);
    let apps = next.apps.as_mut().unwrap();
    apps.rows[0].volume = 10;
    apps.rows.insert(0, row(SliderTarget::App(ItemKey(99)), "New", 30));
    assert!(!v.set_content(next, &Fixed, 1.0, 2000.0).resized);
    assert_eq!(v.node_rect(NodeKey::Slider(target)), before, "the dragged row stayed put");
    assert_eq!(v.content().control(SliderTarget::App(ItemKey(100))).unwrap().volume, 10);
    let r = v.release(p);
    assert!(r.resized, "the waiting content is applied after the drag");
    let pending = v.take_pending().expect("pending");
    assert!(v.set_content(pending, &Fixed, 1.0, 2000.0).resized);
    assert!(v.content().control(SliderTarget::App(ItemKey(99))).is_some());
}

#[test]
fn newer_content_replaces_the_one_waiting_for_the_drag() {
    let mut v = view(content(1, 2, true));
    let target = SliderTarget::App(ItemKey(101));
    let p = centre(&v, NodeKey::Slider(target));
    v.press(p);
    // An application starts (structure changes: kept aside) then stops (same structure
    // as shown: applied at once).
    let mut started = content(1, 2, true);
    started.apps.as_mut().unwrap().rows.push(row(SliderTarget::App(ItemKey(7)), "Gone", 30));
    v.set_content(started, &Fixed, 1.0, 2000.0);
    v.set_content(content(1, 2, true), &Fixed, 1.0, 2000.0);
    let r = v.release(p);
    assert!(!r.resized, "nothing waits any more");
    assert!(v.take_pending().is_none());
    assert!(v.content().control(SliderTarget::App(ItemKey(7))).is_none());
}

#[test]
fn tab_moves_through_the_stops_and_wraps() {
    let mut v = view(content(3, 1, true));
    let expected = [
        NodeKey::Tab(FlyoutTab::Output),
        NodeKey::Tile(ItemKey(1)), // the current tile
        NodeKey::Header,
        NodeKey::Mute(SliderTarget::App(ItemKey(100))),
        NodeKey::Slider(SliderTarget::App(ItemKey(100))),
        NodeKey::Mute(SliderTarget::Main),
        NodeKey::Slider(SliderTarget::Main),
        NodeKey::SoundSettings, // the disabled settings button is skipped
        NodeKey::Tab(FlyoutTab::Output),
    ];
    for key in expected {
        let r = v.key(FlyoutKey::Tab { back: false });
        assert!(r.focus_moved);
        assert_eq!(v.focus, Some(key));
        assert!(v.focus_visible);
    }
    v.key(FlyoutKey::Tab { back: true });
    assert_eq!(v.focus, Some(NodeKey::SoundSettings));
}

#[test]
fn arrows_move_among_tiles_and_enter_selects() {
    let mut v = view(content(5, 0, true)); // 3 + 2
    v.key(FlyoutKey::Tab { back: false });
    v.key(FlyoutKey::Tab { back: false });
    assert_eq!(v.focus, Some(NodeKey::Tile(ItemKey(1))));
    v.key(FlyoutKey::Right);
    assert_eq!(v.focus, Some(NodeKey::Tile(ItemKey(2))));
    v.key(FlyoutKey::Down);
    // Below the middle tile of three: the nearer of the two wider tiles.
    assert!(matches!(v.focus, Some(NodeKey::Tile(ItemKey(4 | 5)))));
    v.key(FlyoutKey::Up);
    assert!(matches!(v.focus, Some(NodeKey::Tile(_))));
    v.key(FlyoutKey::End);
    assert_eq!(v.focus, Some(NodeKey::Tile(ItemKey(5))));
    assert_eq!(v.key(FlyoutKey::Activate).action, Some(FlyoutAction::SelectDevice(ItemKey(5))));
}

#[test]
fn slider_keys_follow_windows() {
    let mut v = view(content(1, 0, true));
    v.set_focus(NodeKey::Slider(SliderTarget::Main), true);
    let set = |r: Response| match r.action {
        Some(FlyoutAction::SetVolume(_, value)) => value,
        other => panic!("{other:?}"),
    };
    assert_eq!(set(v.key(FlyoutKey::Right)), 41);
    assert_eq!(set(v.key(FlyoutKey::Down)), 40);
    assert_eq!(set(v.key(FlyoutKey::PageUp)), 50);
    assert_eq!(set(v.key(FlyoutKey::PageDown)), 40);
    assert_eq!(set(v.key(FlyoutKey::End)), 100);
    assert_eq!(set(v.key(FlyoutKey::Home)), 0);
    assert_eq!(v.key(FlyoutKey::Left).action, None, "already at 0");
}

#[test]
fn left_and_right_switch_tabs() {
    let mut v = view(content(1, 0, true));
    v.key(FlyoutKey::Tab { back: false });
    let r = v.key(FlyoutKey::Right);
    assert_eq!(r.action, Some(FlyoutAction::SelectTab(FlyoutTab::Input)));
}

#[test]
fn the_header_toggles_the_section() {
    let mut v = view(content(1, 2, true));
    let p = centre(&v, NodeKey::Header);
    v.press(p);
    assert_eq!(v.release(p).action, Some(FlyoutAction::SetAppsExpanded(false)));
}

#[test]
fn collapsing_keeps_the_old_rows_painted_until_the_end() {
    let mut v = view(content(1, 2, true));
    assert!(v.set_content(content(1, 2, false), &Fixed, 1.0, 2000.0).resized);
    assert!(v.ghost.is_some());
    v.end_transition();
    assert!(v.ghost.is_none());
}

#[test]
fn a_panel_too_tall_scrolls_only_the_application_list() {
    let mut v = FlyoutView::new(content(3, 30, true), &Fixed, 1.0, 600.0);
    assert_eq!(v.height(), 600.0);
    v.set_shown_height(600.0);
    assert!(v.scrolls());
    let master = v.node_rect(NodeKey::Slider(SliderTarget::Main)).unwrap();
    assert!(master.bottom() <= 600.0, "the master volume stays visible");
    let inside = Point::new(10.0, v.layout.top_height + 20.0);
    assert!(v.wheel(inside, -1).redraw);
    assert!(v.scroll > 0.0);
    // Focusing the last row brings it into view.
    v.set_focus(NodeKey::Slider(SliderTarget::App(ItemKey(129))), true);
    let last = v.node_rect(NodeKey::Slider(SliderTarget::App(ItemKey(129)))).unwrap();
    assert!(last.bottom() <= 600.0 - v.layout.bottom_height + 0.01);
}

#[test]
fn tooltips_show_cut_names_and_the_calls_marker() {
    let mut c = content(3, 0, true);
    c.devices.tiles[1].name = "A very long device name that cannot fit on two lines".into();
    c.devices.tiles[2].calls = Some("Used for calls".into());
    let mut v = view(c);
    v.pointer_move(centre(&v, NodeKey::Tile(ItemKey(1))));
    assert_eq!(v.tooltip(), None, "a short name needs no tooltip");
    v.pointer_move(centre(&v, NodeKey::Tile(ItemKey(2))));
    assert!(v.tooltip().unwrap().0.starts_with("A very long"));
    v.pointer_move(centre(&v, NodeKey::Tile(ItemKey(3))));
    assert_eq!(v.tooltip().unwrap().0, "Used for calls");
}

#[test]
fn a_disabled_button_shows_no_highlight_but_its_tooltip() {
    let mut v = view(content(1, 0, true));
    let p = centre(&v, NodeKey::Settings);
    let r = v.pointer_move(p);
    assert!(!r.redraw, "no highlight on a disabled button");
    assert_eq!(v.tooltip().map(|t| t.0), Some("Settings".to_owned()));
    v.pointer_leave();
    assert_eq!(v.tooltip(), None);
    // Pressing it does nothing.
    v.press(p);
    assert_eq!(v.release(p).action, None);
}

#[test]
fn the_accessibility_tree_describes_every_control() {
    let v = view(content(2, 1, true));
    let tree = v.access_tree();
    assert_eq!(tree.role, AccessRole::Pane);
    let roles: Vec<AccessRole> = tree.children.iter().map(|c| c.role).collect();
    assert_eq!(
        roles,
        [
            AccessRole::TabList,
            AccessRole::List,
            AccessRole::Expander,
            AccessRole::Group,
            AccessRole::Group,
            AccessRole::Button,
            AccessRole::Button,
        ]
    );
    let list = &tree.children[1];
    assert_eq!(list.children.len(), 2);
    assert_eq!(list.children[0].selected, Some(true));
    let master = &tree.children[4];
    assert_eq!(master.children[1].range.map(|r| r.value), Some(40.0));
    assert!(!tree.children[6].enabled, "settings not available yet");
    // Identifiers are unique.
    let mut ids = Vec::new();
    fn collect(node: &AccessNode, ids: &mut Vec<u64>) {
        ids.push(node.id);
        node.children.iter().for_each(|c| collect(c, ids));
    }
    collect(&tree, &mut ids);
    let count = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), count);
}

#[test]
fn assistive_tools_act_like_the_user() {
    let mut v = view(content(2, 0, true));
    let tile = v.access_tree().children[1].children[1].id;
    assert_eq!(
        v.access_action(tile, AccessAction::Select).action,
        Some(FlyoutAction::SelectDevice(ItemKey(2)))
    );
    let slider = FlyoutView::slider_access_id(SliderTarget::Main);
    assert_eq!(
        v.access_action(slider, AccessAction::SetValue(73.4)).action,
        Some(FlyoutAction::SetVolume(SliderTarget::Main, 73))
    );
    let r = v.access_action(slider, AccessAction::Focus);
    assert!(r.focus_moved);
    assert_eq!(v.focused_access_id(), Some(slider));
}

#[test]
fn values_map_to_the_thumb_and_back() {
    let rect = Rect::new(10.0, 0.0, 220.0, 32.0);
    for value in [0u8, 1, 37, 50, 99, 100] {
        assert_eq!(value_at(rect, thumb_x(rect, value)), value);
    }
    assert_eq!(value_at(rect, -50.0), 0);
    assert_eq!(value_at(rect, 1000.0), 100);
}

#[test]
fn a_view_is_at_rest_only_without_interaction() {
    let mut v = view(content(2, 1, true));
    assert!(v.at_rest());
    v.pointer_move(centre(&v, NodeKey::Tile(ItemKey(1))));
    assert!(!v.at_rest(), "a highlighted tile");
    v.pointer_leave();
    assert!(v.at_rest());
    v.key(FlyoutKey::Tab { back: false });
    assert!(!v.at_rest(), "the focus indicator shows");
    let mut v = view(content(2, 1, true));
    v.set_content(content(2, 1, false), &Fixed, 1.0, 2000.0);
    assert!(!v.at_rest(), "the height is changing");
    v.set_shown_height(v.height());
    v.end_transition();
    assert!(v.at_rest());
}

fn full(tiles: usize, apps: usize, expanded: bool) -> FlyoutContent {
    FlyoutContent { height: PanelHeight::Full, ..content(tiles, apps, expanded) }
}

#[test]
fn a_full_height_panel_keeps_its_size_and_the_tiles_never_move() {
    // Whatever the content, the panel is as tall as it is allowed to be.
    let mut v = view(full(2, 3, true));
    assert_eq!(v.height(), 2000.0);
    let tile = v.node_rect(NodeKey::Tile(ItemKey(1)));
    let master = v.node_rect(NodeKey::Slider(SliderTarget::Main));
    for next in [full(2, 0, true), full(2, 6, true), full(2, 3, false)] {
        assert!(!v.set_content(next, &Fixed, 1.0, 2000.0).resized, "same size");
        assert_eq!(v.height(), 2000.0);
        assert_eq!(v.node_rect(NodeKey::Tile(ItemKey(1))), tile, "the tiles stay");
        assert_eq!(v.node_rect(NodeKey::Slider(SliderTarget::Main)), master, "and the master");
        assert!(v.at_rest(), "nothing to animate");
    }
}

#[test]
fn in_a_full_height_panel_the_list_stands_right_above_the_master_volume() {
    // The free space is between the tiles and the list, not under the list.
    let v = view(full(2, 3, true));
    let last_row = v.node_rect(NodeKey::Slider(SliderTarget::App(ItemKey(102)))).unwrap();
    let master = v.node_rect(NodeKey::Slider(SliderTarget::Main)).unwrap();
    let tiles_bottom = v.node_rect(NodeKey::Tile(ItemKey(1))).unwrap().bottom();
    let header = v.node_rect(NodeKey::Header).unwrap();
    assert!(master.y - last_row.bottom() < 120.0, "the list ends near the master volume");
    assert!(header.y - tiles_bottom > 1000.0, "the free space is above the list");
    assert!(!v.scrolls());
}

#[test]
fn a_full_height_list_scrolls_only_when_it_does_not_fit() {
    let mut v = FlyoutView::new(full(3, 30, true), &Fixed, 1.0, 600.0);
    assert_eq!(v.height(), 600.0);
    assert!(v.scrolls());
    let tile = v.node_rect(NodeKey::Tile(ItemKey(1))).unwrap();
    let master = v.node_rect(NodeKey::Slider(SliderTarget::Main)).unwrap();
    assert!(tile.y < 100.0 && master.bottom() <= 600.0, "tiles and master stay visible");
    let inside = Point::new(10.0, v.middle_clip().y + 20.0);
    assert!(v.wheel(inside, -1).redraw);
    assert!(v.scroll > 0.0);
}

#[test]
fn switching_the_height_setting_restructures_the_panel() {
    let mut v = view(content(2, 3, true));
    let fitted = v.height();
    let change = v.set_content(full(2, 3, true), &Fixed, 1.0, 2000.0);
    assert!(change.restructured && change.resized);
    assert!(v.height() > fitted);
}

#[test]
fn rows_that_vanish_for_a_moment_are_kept_in_place() {
    let previous = content(2, 3, true);
    // The middle application is missing from the new content (its sound has not reached
    // the new output yet); another one appeared.
    let mut next = content(2, 3, true);
    let apps = next.apps.as_mut().unwrap();
    apps.rows.remove(2);
    apps.rows.push(row(SliderTarget::App(ItemKey(200)), "New", 20));
    assert!(next.keep_rows_of(&previous));
    let targets: Vec<SliderTarget> = next.visible_rows().iter().map(|r| r.target).collect();
    assert_eq!(
        targets,
        [
            SliderTarget::App(ItemKey(100)),
            SliderTarget::App(ItemKey(101)),
            SliderTarget::App(ItemKey(102)),
            SliderTarget::App(ItemKey(200)),
        ],
        "kept at its place, the new one added"
    );
    // Nothing missing: nothing kept. Another view or a collapsed list: nothing kept.
    let mut same = content(2, 3, true);
    assert!(!same.keep_rows_of(&previous));
    let mut collapsed = content(2, 0, false);
    assert!(!collapsed.keep_rows_of(&previous));
}

fn with_update(close: bool) -> FlyoutContent {
    FlyoutContent {
        update: Some(UpdateLine {
            text: "Version 2.1.0 available".into(),
            action: Some("Install".into()),
            close: close.then(|| "Close".into()),
        }),
        ..content(2, 0, false)
    }
}

#[test]
fn the_update_line_sits_above_the_footer_and_reports_its_buttons() {
    let plain = view(content(2, 0, false));
    let mut v = view(with_update(true));
    // The line adds its own band; the footer buttons stay at the bottom.
    assert_eq!(v.height(), plain.height() + crate::tokens::footer::UPDATE_HEIGHT);
    let install = v.node_rect(NodeKey::UpdateAction).expect("install button");
    let close = v.node_rect(NodeKey::UpdateClose).expect("close button");
    let settings = v.node_rect(NodeKey::Settings).expect("settings button");
    assert!(install.bottom() <= settings.y && close.bottom() <= settings.y);
    assert!(install.right() <= close.x, "the close button is at the far right");
    assert!(close.right() <= crate::tokens::flyout::WIDTH);
    let text = v.layout.update_text.expect("text");
    assert!(text.right() <= install.x && text.width > 0.0);
    for (key, action) in [
        (NodeKey::UpdateAction, FlyoutAction::InstallUpdate),
        (NodeKey::UpdateClose, FlyoutAction::DismissUpdate),
    ] {
        let p = centre(&v, key);
        v.pointer_move(p);
        v.press(p);
        assert_eq!(v.release(p).action, Some(action));
    }
    // The close button explains itself.
    let p = centre(&v, NodeKey::UpdateClose);
    v.pointer_move(p);
    assert_eq!(v.tooltip().map(|(t, _)| t), Some("Close".to_owned()));
}

#[test]
fn the_update_line_is_reached_by_keyboard_and_screen_readers() {
    let mut v = view(with_update(false));
    assert!(v.node_rect(NodeKey::UpdateClose).is_none(), "no close button while downloading");
    let stops = v.stops();
    let main = stops.iter().position(|s| *s == NodeKey::Slider(SliderTarget::Main)).unwrap();
    let install = stops.iter().position(|s| *s == NodeKey::UpdateAction).expect("a stop");
    let link = stops.iter().position(|s| *s == NodeKey::SoundSettings).unwrap();
    assert!(main < install && install < link, "{stops:?}");
    v.set_focus(NodeKey::UpdateAction, true);
    assert_eq!(v.key(FlyoutKey::Activate).action, Some(FlyoutAction::InstallUpdate));
    let tree = v.access_tree();
    let names: Vec<&str> = tree.children.iter().map(|c| c.name.as_str()).collect();
    let text = names.iter().position(|n| *n == "Version 2.1.0 available").expect("text node");
    let button = names.iter().position(|n| *n == "Install").expect("button node");
    assert!(text < button && button < names.iter().position(|n| *n == "Sound settings").unwrap());
}

#[test]
fn the_update_line_appearing_is_a_change_of_structure() {
    assert!(!content(2, 0, false).same_structure(&with_update(true)));
    assert!(!with_update(false).same_structure(&with_update(true)));
    let mut newer = with_update(true);
    if let Some(u) = newer.update.as_mut() {
        u.text = "Version 2.2.0 available".into();
    }
    assert!(with_update(true).same_structure(&newer));
}

#[test]
fn the_update_line_grows_with_large_text_and_lines_up_with_the_gear() {
    let big = |c| FlyoutView::new(c, &Fixed, 2.25, 2000.0);
    let (plain, with) = (big(content(2, 0, false)), big(with_update(true)));
    let grown = with.height() - plain.height();
    assert!(grown > crate::tokens::footer::UPDATE_HEIGHT, "{grown}");
    let close = with.node_rect(NodeKey::UpdateClose).unwrap();
    let settings = with.node_rect(NodeKey::Settings).unwrap();
    assert_eq!(close.x + close.width / 2.0, settings.x + settings.width / 2.0);
    let install = with.node_rect(NodeKey::UpdateAction).unwrap();
    assert!(install.height >= crate::tokens::typography::CAPTION_STRONG.line_height * 2.25);
}
