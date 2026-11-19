//! Tests of the settings view (behaviour, layout, accessibility).

use super::*;
use crate::controls::flyout::TextMeasure;
use crate::icons::Icon;
use crate::tokens::typography::TextStyle;

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

const GENERAL: PageKey = PageKey(1);
const ABOUT: PageKey = PageKey(2);
const AUTOSTART: SettingKey = SettingKey(1);
const LANGUAGE: SettingKey = SettingKey(2);
const LINK: SettingKey = SettingKey(3);
const INFO: SettingKey = SettingKey(4);

fn card(key: SettingKey, title: &str, control: CardControl) -> Block {
    Block::Card(Card {
        key,
        icon: Some(Icon::Power),
        title: title.into(),
        description: Some("A description".into()),
        control,
        enabled: true,
    })
}

fn toggle(on: bool) -> CardControl {
    CardControl::Toggle { on, on_label: "On".into(), off_label: "Off".into() }
}

fn choice(selected: usize, count: usize) -> CardControl {
    CardControl::Choice { options: (0..count).map(|i| format!("Option {i}")).collect(), selected }
}

fn content(page: PageKey, blocks: Vec<Block>) -> SettingsContent {
    SettingsContent {
        name: "Settings".into(),
        nav_name: "Navigation".into(),
        nav: vec![
            NavEntry { key: GENERAL, icon: Icon::Settings, label: "General".into() },
            NavEntry { key: ABOUT, icon: Icon::Info, label: "About".into() },
        ],
        page,
        title: "General".into(),
        blocks,
        devices: texts(),
        close_question: None,
        question: None,
        reveal: None,
    }
}

fn general(on: bool, selected: usize) -> SettingsContent {
    content(
        GENERAL,
        vec![
            card(AUTOSTART, "Start with Windows", toggle(on)),
            Block::Header("Group".into()),
            card(LANGUAGE, "Language", choice(selected, 3)),
            card(LINK, "Log", CardControl::Link { label: "Open folder".into() }),
            card(INFO, "Version", CardControl::None),
        ],
    )
}

fn view(content: SettingsContent) -> SettingsView {
    SettingsView::new(content, Size::new(900.0, 600.0), &Fixed, 1.0)
}

fn center(r: Rect) -> Point {
    Point::new(r.x + r.width / 2.0, r.y + r.height / 2.0)
}

fn control(v: &SettingsView, key: SettingKey) -> Rect {
    v.layout.on_screen(v.card_layout(key).unwrap().control.unwrap())
}

fn click(v: &mut SettingsView, p: Point) -> Response {
    v.pointer_move(p, &Fixed);
    let down = v.press(p, &Fixed);
    let up = v.release(p, &Fixed);
    assert_eq!(up.capture, Some(false));
    Response { focus_moved: down.focus_moved || up.focus_moved, ..up }
}

#[test]
fn cards_stack_under_the_title_with_controls_at_the_right() {
    let v = view(general(true, 0));
    let l = &v.layout;
    assert_eq!(l.cards.len(), 4);
    assert!(l.title.x >= t::NAV_WIDTH + t::PAGE_PADDING_X);
    for pair in l.cards.windows(2) {
        assert!(pair[1].rect.y >= pair[0].rect.bottom() + t::CARD_GAP - 0.01);
        assert!(pair[0].rect.height >= t::CARD_MIN_HEIGHT);
    }
    // The header adds its gap between the first and the second card.
    assert!(l.cards[1].rect.y - l.cards[0].rect.bottom() > t::HEADER_GAP_TOP);
    for laid in &l.cards[..3] {
        let c = laid.control.unwrap();
        assert!((c.right() - (laid.rect.right() - t::CARD_PADDING.0)).abs() < 0.01);
        assert!(
            laid.title.right() <= c.x - t::CARD_CONTROL_GAP + 0.01,
            "text never runs under the control"
        );
    }
    assert!(l.cards[3].control.is_none());
    // The page content is never wider than the design allows.
    let wide = SettingsView::new(general(true, 0), Size::new(3000.0, 600.0), &Fixed, 1.0);
    assert_eq!(wide.layout.cards[0].rect.width, t::PAGE_MAX_WIDTH);
}

#[test]
fn a_narrow_card_puts_its_control_under_its_text() {
    let field = CardControl::TextField(TextField {
        value: String::new(),
        placeholder: String::new(),
        password: false,
        digits_only: false,
        max_chars: 10,
    });
    let mut page = general(true, 0);
    page.blocks.push(card(SettingKey(9), "Address", field));
    // The narrowest window: cards narrower than the wrap threshold.
    let (min_width, min_height) = t::MIN_SIZE;
    let narrow = SettingsView::new(page.clone(), Size::new(min_width, min_height), &Fixed, 1.0);
    let wide = view(page);
    for (n, w) in narrow.layout.cards.iter().zip(&wide.layout.cards) {
        assert!(n.rect.width < t::CARD_WRAP_THRESHOLD.0);
        let Some(c) = n.control else {
            assert!(w.control.is_none());
            continue;
        };
        // Under the text, aligned with it, inside the card.
        let text_bottom = n.description.map_or(n.title.bottom(), |(d, _)| d.bottom());
        assert!(c.y >= text_bottom + t::CARD_WRAP_GAP - 0.01, "control under the text");
        assert!((c.x - n.title.x).abs() < 0.01, "aligned with the text");
        assert!(c.right() <= n.rect.right() - t::CARD_PADDING.0 + 0.01, "inside the card");
        assert!(c.bottom() <= n.rect.bottom() - t::CARD_PADDING.1 + 0.01);
        // The text takes the whole width again.
        assert!((n.title.right() - (n.rect.right() - t::CARD_PADDING.0)).abs() < 0.01);
        // Wide cards keep the control on the right.
        let wc = w.control.unwrap();
        assert!((wc.right() - (w.rect.right() - t::CARD_PADDING.0)).abs() < 0.01);
    }
    // The text field, wider than the room, is narrowed to it.
    let field = narrow.layout.cards.last().unwrap();
    assert!(field.control.unwrap().width < t::FIELD_WIDTH);
    // A long title wraps instead of being cut; the description follows it.
    let mut long = general(true, 0);
    long.blocks[0] = card(AUTOSTART, &"Long title ".repeat(8), toggle(true));
    let v = SettingsView::new(long, Size::new(min_width, min_height), &Fixed, 1.0);
    let first = &v.layout.cards[0];
    assert!(first.title_lines > 1);
    assert!(first.description.unwrap().0.y >= first.title.bottom() - 0.01);
    assert_eq!(narrow.layout.cards[0].title_lines, 1);
}

#[test]
fn text_scaling_grows_the_rows() {
    let small = view(general(true, 0));
    let large = SettingsView::new(general(true, 0), Size::new(900.0, 600.0), &Fixed, 2.0);
    assert!(large.layout.nav[0].height > small.layout.nav[0].height);
    assert!(
        large.layout.cards[1].control.unwrap().height
            > small.layout.cards[1].control.unwrap().height
    );
}

#[test]
fn clicking_a_toggle_asks_for_the_other_state() {
    let mut v = view(general(false, 0));
    let r = click_control(&mut v, AUTOSTART);
    assert_eq!(r.action, Some(SettingsAction::Toggle(AUTOSTART, true)));
    assert!(r.focus_moved);
    // A press released elsewhere does nothing.
    let p = center(control(&v, AUTOSTART));
    v.press(p, &Fixed);
    assert_eq!(v.release(Point::new(1.0, 500.0), &Fixed).action, None);
}

#[test]
fn navigation_reports_another_page_only() {
    let mut v = view(general(true, 0));
    assert_eq!(click_nav(&mut v, 0).action, None, "already shown");
    assert_eq!(click_nav(&mut v, 1).action, Some(SettingsAction::SelectPage(ABOUT)));
}

#[test]
fn a_choice_opens_its_list_and_reports_the_pick() {
    let mut v = view(general(true, 1));
    assert_eq!(click_control(&mut v, LANGUAGE).action, None);
    assert!(v.list_open());
    let (_, rect, item, count) = v.list_geometry().unwrap();
    assert_eq!(count, 3);
    // The selected entry lies over the box.
    let selected_center = rect.y + t::LIST_PADDING + 1.5 * item;
    assert!((selected_center - center(control(&v, LANGUAGE)).y).abs() < 1.0);
    let third = Point::new(rect.x + 20.0, rect.y + t::LIST_PADDING + 2.5 * item);
    assert_eq!(click(&mut v, third).action, Some(SettingsAction::Choose(LANGUAGE, 2)));
    assert!(!v.list_open());
    // Picking the current entry only closes the list.
    click_control(&mut v, LANGUAGE);
    let (_, rect, item, _) = v.list_geometry().unwrap();
    let second = Point::new(rect.x + 20.0, rect.y + t::LIST_PADDING + 1.5 * item);
    assert_eq!(click(&mut v, second).action, None);
    assert!(!v.list_open());
}

#[test]
fn a_click_outside_the_open_list_only_closes_it() {
    let mut v = view(general(false, 0));
    click_control(&mut v, LANGUAGE);
    let r = click_control(&mut v, AUTOSTART);
    assert!(!v.list_open());
    assert_eq!(r.action, None, "the toggle under the click is not changed");
}

#[test]
fn a_long_list_stays_in_the_window_and_scrolls() {
    let mut v = SettingsView::new(
        content(GENERAL, vec![card(LANGUAGE, "Step", choice(9, 10))]),
        Size::new(900.0, 300.0),
        &Fixed,
        1.0,
    );
    v.key(SettingsKey::Tab { back: false }, &Fixed);
    v.key(SettingsKey::Tab { back: false }, &Fixed);
    v.key(SettingsKey::Open, &Fixed);
    let (list, rect, item, _) = v.list_geometry().unwrap();
    assert!(rect.y >= t::LIST_MARGIN && rect.bottom() <= 300.0 - t::LIST_MARGIN + 0.01);
    // The selected (last) entry is scrolled into view.
    assert!(list.scroll > 0.0);
    assert!(9.0 * item - list.scroll + item <= rect.height);
    v.key(SettingsKey::Home { select: false }, &Fixed);
    assert_eq!(v.list.unwrap().scroll, 0.0);
    assert_eq!(v.key(SettingsKey::Enter, &Fixed).action, Some(SettingsAction::Choose(LANGUAGE, 0)));
}

#[test]
fn keyboard_walks_the_stops_and_drives_the_controls() {
    let mut v = view(general(false, 0));
    // First stop: the navigation list, on the page shown.
    assert!(v.key(SettingsKey::Tab { back: false }, &Fixed).focus_moved);
    assert_eq!(v.focus, Some(Target::Nav(0)));
    v.key(SettingsKey::Down, &Fixed);
    assert_eq!(v.focus, Some(Target::Nav(1)));
    assert_eq!(v.key(SettingsKey::Enter, &Fixed).action, Some(SettingsAction::SelectPage(ABOUT)));
    v.key(SettingsKey::Up, &Fixed);
    v.key(SettingsKey::Tab { back: false }, &Fixed);
    assert_eq!(v.focus, Some(Target::Control(AUTOSTART)));
    assert_eq!(
        v.key(SettingsKey::Space, &Fixed).action,
        Some(SettingsAction::Toggle(AUTOSTART, true))
    );
    v.key(SettingsKey::Tab { back: false }, &Fixed);
    assert_eq!(v.focus, Some(Target::Control(LANGUAGE)));
    // Closed: arrows step through the options.
    assert_eq!(v.key(SettingsKey::Down, &Fixed).action, Some(SettingsAction::Choose(LANGUAGE, 1)));
    assert_eq!(v.key(SettingsKey::Up, &Fixed).action, None, "already the first");
    // Open, move, pick.
    v.key(SettingsKey::Open, &Fixed);
    assert!(v.list_open());
    v.key(SettingsKey::End { select: false }, &Fixed);
    assert_eq!(v.key(SettingsKey::Enter, &Fixed).action, Some(SettingsAction::Choose(LANGUAGE, 2)));
    v.key(SettingsKey::Enter, &Fixed);
    assert_eq!(v.key(SettingsKey::Escape, &Fixed).action, None);
    assert!(!v.list_open());
    v.key(SettingsKey::Tab { back: false }, &Fixed);
    assert_eq!(v.key(SettingsKey::Enter, &Fixed).action, Some(SettingsAction::Invoke(LINK)));
    // The information card is no stop: Tab wraps to the navigation list.
    v.key(SettingsKey::Tab { back: false }, &Fixed);
    assert_eq!(v.focus, Some(Target::Nav(0)));
    v.key(SettingsKey::Tab { back: true }, &Fixed);
    assert_eq!(v.focus, Some(Target::Control(LINK)));
}

#[test]
fn disabled_controls_are_skipped_and_inert() {
    let mut c = general(false, 0);
    if let Block::Card(card) = &mut c.blocks[0] {
        card.enabled = false;
    }
    let mut v = view(c);
    assert_eq!(click_control(&mut v, AUTOSTART).action, None);
    v.key(SettingsKey::Tab { back: false }, &Fixed);
    v.key(SettingsKey::Tab { back: false }, &Fixed);
    assert_eq!(v.focus, Some(Target::Control(LANGUAGE)));
}

#[test]
fn a_tall_page_scrolls_and_keeps_the_focus_in_view() {
    let blocks: Vec<Block> =
        (0..20).map(|i| card(SettingKey(10 + i), &format!("Card {i}"), toggle(false))).collect();
    let mut v = SettingsView::new(content(GENERAL, blocks), Size::new(900.0, 400.0), &Fixed, 1.0);
    assert!(v.layout.max_scroll() > 0.0);
    assert!(v.layout.scroll_thumb().is_some());
    assert!(v.wheel(Point::new(500.0, 200.0), -2, &Fixed).redraw);
    assert_eq!(v.scroll(), 2.0 * t::WHEEL_STEP);
    v.wheel(Point::new(500.0, 200.0), 100, &Fixed);
    assert_eq!(v.scroll(), 0.0, "never above the top");
    v.key(SettingsKey::Tab { back: true }, &Fixed);
    let last = v.card_layout(SettingKey(29)).unwrap().rect;
    assert!(last.bottom() - v.scroll() <= 400.0, "the last card was scrolled into view");
    // Dragging the thumb to the bottom shows the end of the page.
    let thumb = v.layout.scroll_thumb().unwrap();
    v.key(SettingsKey::Home { select: false }, &Fixed);
    let thumb_now = v.layout.scroll_thumb().unwrap();
    assert!(thumb_now.y < thumb.y);
    let p = center(thumb_now);
    v.pointer_move(p, &Fixed);
    assert_eq!(v.press(p, &Fixed).capture, Some(true));
    v.pointer_move(Point::new(p.x, 1000.0), &Fixed);
    assert_eq!(v.scroll(), v.layout.max_scroll());
    v.release(Point::new(p.x, 1000.0), &Fixed);
    // A new page starts at its top.
    let mut other = content(ABOUT, vec![card(INFO, "x", CardControl::None)]);
    other.title = "About".into();
    v.set_content(other, &Fixed);
    assert_eq!(v.scroll(), 0.0);
}

#[test]
fn content_updates_keep_the_interaction_state() {
    let mut v = view(general(false, 0));
    v.key(SettingsKey::Tab { back: false }, &Fixed);
    v.key(SettingsKey::Tab { back: false }, &Fixed);
    v.key(SettingsKey::Tab { back: false }, &Fixed);
    v.key(SettingsKey::Open, &Fixed);
    v.set_content(general(true, 1), &Fixed);
    assert!(v.list_open(), "a live update does not close the open list");
    assert_eq!(v.focus, Some(Target::Control(LANGUAGE)));
    // The choice disappears: its list closes and the focus is forgotten.
    v.set_content(content(GENERAL, vec![card(AUTOSTART, "Start", toggle(true))]), &Fixed);
    assert!(!v.list_open());
    assert_eq!(v.focus, None);
    assert!(v.focus_lost() == Response::default());
}

#[test]
fn the_window_is_described_for_screen_readers() {
    let mut v = view(general(true, 2));
    let tree = v.access_tree(&Fixed);
    let nav = tree.find(ids::NAV).unwrap();
    assert_eq!(nav.role, AccessRole::List);
    assert_eq!(nav.children.len(), 2);
    assert_eq!(nav.children[0].selected, Some(true));
    let toggle = tree.find(ids::CONTROL | (AUTOSTART.0 << 4)).unwrap();
    assert_eq!((toggle.role, toggle.toggled), (AccessRole::ToggleButton, Some(true)));
    assert_eq!(toggle.name, "Start with Windows");
    assert_eq!(toggle.help.as_deref(), Some("A description"));
    let combo = tree.find(ids::CONTROL | (LANGUAGE.0 << 4)).unwrap();
    assert_eq!((combo.role, combo.expanded), (AccessRole::ComboBox, Some(false)));
    assert_eq!(combo.children.iter().filter(|c| c.selected == Some(true)).count(), 1);
    assert_eq!(combo.children[2].selected, Some(true));
    let link = tree.find(ids::CONTROL | (LINK.0 << 4)).unwrap();
    assert_eq!(link.role, AccessRole::Hyperlink);
    assert_eq!(tree.find(ids::CARD | (INFO.0 << 4)).unwrap().role, AccessRole::Text);
    // Identifiers are unique.
    let mut all = Vec::new();
    fn walk(n: &AccessNode, all: &mut Vec<u64>) {
        all.push(n.id);
        n.children.iter().for_each(|c| walk(c, all));
    }
    walk(&tree, &mut all);
    let count = all.len();
    all.sort_unstable();
    all.dedup();
    assert_eq!(all.len(), count);

    // Requests from assistive tools.
    let toggle_id = ids::CONTROL | (AUTOSTART.0 << 4);
    assert_eq!(
        v.access_action(toggle_id, AccessAction::Toggle, &Fixed).action,
        Some(SettingsAction::Toggle(AUTOSTART, false))
    );
    let combo_id = ids::CONTROL | (LANGUAGE.0 << 4);
    v.access_action(combo_id, AccessAction::Expand, &Fixed);
    assert!(v.list_open());
    assert_eq!(v.focused_access_id(), Some(ids::OPTION | (LANGUAGE.0 << 8) | 2));
    assert_eq!(v.access_tree(&Fixed).find(combo_id).unwrap().expanded, Some(true));
    let option = ids::OPTION | (LANGUAGE.0 << 8);
    assert_eq!(
        v.access_action(option, AccessAction::Select, &Fixed).action,
        Some(SettingsAction::Choose(LANGUAGE, 0))
    );
    assert!(!v.list_open());
    assert_eq!(v.focused_access_id(), Some(combo_id));
    assert_eq!(
        v.access_action(ids::NAV_ITEM + 1, AccessAction::Select, &Fixed).action,
        Some(SettingsAction::SelectPage(ABOUT))
    );
    assert_eq!(
        v.access_action(ids::CONTROL | (LINK.0 << 4), AccessAction::Invoke, &Fixed).action,
        Some(SettingsAction::Invoke(LINK))
    );
    assert_eq!(v.access_action(999, AccessAction::Invoke, &Fixed), Response::default());
}

fn click_control(v: &mut SettingsView, key: SettingKey) -> Response {
    let p = center(control(v, key));
    click(v, p)
}

fn click_nav(v: &mut SettingsView, index: usize) -> Response {
    let p = center(v.layout.nav[index]);
    click(v, p)
}

// ----- Device rows -----

use crate::controls::device_icon::DeviceIcon;

fn texts() -> DeviceTexts {
    DeviceTexts {
        change_icon: "Change icon".into(),
        rename: "Rename".into(),
        hide: "Hide".into(),
        show: "Show".into(),
        forget: "Forget".into(),
        move_help: "Alt+Up, Alt+Down".into(),
        name_box: "Name".into(),
        duplicate: "Already used".into(),
        automatic: "Automatic".into(),
        icon_names: DeviceIcon::ALL.iter().map(|i| i.id().to_owned()).collect(),
        forget_question: "Forget?".into(),
        forget_detail: "Lost.".into(),
        forget_confirm: "Forget".into(),
        cancel: "Cancel".into(),
    }
}

fn row(key: u64, list: u32, name: &str, connected: bool) -> Block {
    Block::Device(DeviceRow {
        key: SettingKey(key),
        list,
        picture: RowPicture::Device {
            shown: DeviceIcon::Headset,
            chosen: None,
            automatic: DeviceIcon::Headset,
        },
        name: name.into(),
        detail: "Windows name · connected".into(),
        hidden: false,
        renamable: true,
        forgettable: !connected,
    })
}

const A: SettingKey = SettingKey(10);
const B: SettingKey = SettingKey(11);
const C: SettingKey = SettingKey(12);
const MIC: SettingKey = SettingKey(20);

fn devices() -> SettingsView {
    let blocks = vec![
        Block::Text("Drag a row to change the order.".into()),
        Block::Header("Outputs".into()),
        row(10, 0, "Speakers", true),
        row(11, 0, "Headset", true),
        row(12, 0, "TV", false),
        Block::Header("Microphones".into()),
        row(20, 1, "Microphone", true),
    ];
    view(content(GENERAL, blocks))
}

fn part(v: &SettingsView, key: SettingKey, part: Part) -> Rect {
    let laid = v.layout.row(key).unwrap();
    let r = match part {
        Part::Body => laid.grip,
        Part::Icon => laid.icon,
        Part::Rename => laid.rename.unwrap(),
        Part::Visibility => laid.visibility,
        Part::Forget => laid.forget.unwrap(),
        Part::TextBox => laid.text_box,
    };
    v.layout.on_screen(r)
}

fn click_part(v: &mut SettingsView, key: SettingKey, which: Part) -> Response {
    let p = center(part(v, key, which));
    click(v, p)
}

fn type_text(v: &mut SettingsView, text: &str) {
    v.text_input(text, &Fixed);
}

#[test]
fn rows_have_their_buttons_and_disconnected_ones_can_be_forgotten() {
    let v = devices();
    assert_eq!(v.layout.rows.len(), 4);
    assert_eq!(v.layout.texts.len(), 1);
    let speakers = v.layout.row(A).unwrap();
    assert!(speakers.forget.is_none());
    assert!(v.layout.row(C).unwrap().forget.is_some());
    assert!(speakers.rect.height >= t::ROW_MIN_HEIGHT);
    assert!(
        speakers.name.right() <= speakers.rename.unwrap().x,
        "the name never runs under the buttons"
    );
    assert!(speakers.icon.x > speakers.grip.right());
}

fn app_row(key: u64, name: &str, picture: RowPicture, forgettable: bool) -> Block {
    Block::Device(DeviceRow {
        key: SettingKey(key),
        list: 0,
        picture,
        name: name.into(),
        detail: "player.exe · In use".into(),
        hidden: false,
        renamable: false,
        forgettable,
    })
}

const PLAYER: SettingKey = SettingKey(30);
const SYSTEM: SettingKey = SettingKey(31);

fn applications() -> SettingsView {
    let blocks = vec![
        Block::Text("Applications that played sound.".into()),
        app_row(30, "Player", RowPicture::App { pixels: None, source: None }, true),
        app_row(31, "System sounds", RowPicture::SystemSounds, false),
    ];
    view(content(GENERAL, blocks))
}

#[test]
fn application_rows_have_no_icon_button_nor_rename() {
    let mut v = applications();
    let player = v.layout.row(PLAYER).unwrap().clone();
    assert!(player.rename.is_none());
    assert!(player.forget.is_some(), "an application can be forgotten");
    assert!(v.layout.row(SYSTEM).unwrap().forget.is_none(), "System sounds cannot");
    assert!(player.name.right() <= player.forget.unwrap().x);
    // The bin takes the place of the pencil: next to the eye.
    let (button, _, gap) = t::ROW_BUTTON;
    assert_eq!(player.forget.unwrap().x, player.visibility.x - gap - button);
    // The picture is not a button.
    let picture = center(v.layout.on_screen(player.icon));
    assert_eq!(click(&mut v, picture).action, None);
    assert!(v.picker.is_none());
    // Tab goes from the navigation to the eye and the bin, then to the next row.
    let stops = v.stops();
    let rows: Vec<Target> = stops.into_iter().filter(|s| matches!(s, Target::Row(..))).collect();
    assert_eq!(
        rows,
        [
            Target::Row(PLAYER, Part::Visibility),
            Target::Row(PLAYER, Part::Forget),
            Target::Row(SYSTEM, Part::Visibility),
        ]
    );
    // Moved with the keyboard like devices.
    v.focus = Some(Target::Row(SYSTEM, Part::Visibility));
    assert_eq!(
        v.key(SettingsKey::AltUp, &Fixed).action,
        Some(SettingsAction::MoveDevice(SYSTEM, 0))
    );
    // The bin asks first, then forgets.
    click_part(&mut v, PLAYER, Part::Forget);
    assert_eq!(v.confirm, Some(PLAYER));
    assert_eq!(v.focus, Some(Target::ConfirmCancel));
    assert_eq!(
        v.key(SettingsKey::Tab { back: false }, &Fixed).action,
        None,
        "Tab moves to the confirm button"
    );
    assert_eq!(v.key(SettingsKey::Enter, &Fixed).action, Some(SettingsAction::Forget(PLAYER)));
}

#[test]
fn application_rows_expose_only_their_buttons() {
    let v = applications();
    let tree = v.access_tree(&Fixed);
    let mut groups = Vec::new();
    fn collect<'a>(node: &'a AccessNode, out: &mut Vec<&'a AccessNode>) {
        if node.role == AccessRole::Group && node.name == "Player" {
            out.push(node);
        }
        for child in &node.children {
            collect(child, out);
        }
    }
    collect(&tree, &mut groups);
    let player = groups.first().expect("the row is a group");
    let names: Vec<&str> = player.children.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Hide", "Forget"]);
    assert!(player.children.iter().all(|c| c.role == AccessRole::Button));
}

#[test]
fn the_eye_hides_and_shows() {
    let mut v = devices();
    assert_eq!(
        click_part(&mut v, A, Part::Visibility).action,
        Some(SettingsAction::SetHidden(A, true))
    );
}

#[test]
fn rows_move_with_the_keyboard_within_their_list() {
    let mut v = devices();
    click_part(&mut v, B, Part::Rename);
    v.key(SettingsKey::Escape, &Fixed);
    assert_eq!(v.focus, Some(Target::Row(B, Part::Rename)));
    assert_eq!(v.key(SettingsKey::AltUp, &Fixed).action, Some(SettingsAction::MoveDevice(B, 0)));
    assert_eq!(v.key(SettingsKey::AltDown, &Fixed).action, Some(SettingsAction::MoveDevice(B, 2)));
    // The last of its list cannot go further down; the microphone list is separate.
    v.focus = Some(Target::Row(C, Part::Rename));
    assert_eq!(v.key(SettingsKey::AltDown, &Fixed).action, None);
    v.focus = Some(Target::Row(MIC, Part::Rename));
    assert_eq!(v.key(SettingsKey::AltUp, &Fixed).action, None);
}

#[test]
fn rows_move_by_drag_and_drop() {
    let mut v = devices();
    let start = center(part(&v, A, Part::Body));
    v.pointer_move(start, &Fixed);
    assert_eq!(v.press(start, &Fixed).capture, Some(true));
    // A small move is not a drag yet.
    v.pointer_move(Point::new(start.x, start.y + 2.0), &Fixed);
    assert!(!v.drag.unwrap().active);
    // Dragged below the headset: second place.
    let headset = v.layout.on_screen(v.layout.row(B).unwrap().rect);
    let to = Point::new(start.x, start.y + (headset.bottom() - start.y) + 4.0);
    v.pointer_move(to, &Fixed);
    assert!(v.drag.unwrap().active);
    let r = v.release(to, &Fixed);
    assert_eq!(r.action, Some(SettingsAction::MoveDevice(A, 1)));
    assert_eq!(r.capture, Some(false));
    // Dropped where it was: nothing.
    v.press(start, &Fixed);
    v.pointer_move(Point::new(start.x, start.y + 10.0), &Fixed);
    assert_eq!(v.release(Point::new(start.x, start.y + 10.0), &Fixed).action, None);
    // A plain click on the row does nothing either.
    assert_eq!(click(&mut v, start).action, None);
}

#[test]
fn renaming_commits_trims_and_refuses_duplicates() {
    let mut v = devices();
    click_part(&mut v, A, Part::Rename);
    assert!(v.editing());
    assert_eq!(v.focus, Some(Target::Row(A, Part::TextBox)));
    // The whole name is selected: typing replaces it.
    type_text(&mut v, "  Desk  ");
    assert_eq!(
        v.key(SettingsKey::Enter, &Fixed).action,
        Some(SettingsAction::Rename(A, "Desk".into()))
    );
    assert!(!v.editing());

    // A name another row shows (case ignored) is refused and the box stays.
    click_part(&mut v, A, Part::Rename);
    type_text(&mut v, "headset");
    let r = v.key(SettingsKey::Enter, &Fixed);
    assert_eq!(r.action, None);
    assert!(v.editing());
    assert!(v.layout.row(A).unwrap().error.is_some(), "the message shows");
    // The microphone list is another list: its names do not count.
    v.key(SettingsKey::SelectAll, &Fixed);
    type_text(&mut v, "Microphone");
    assert!(v.layout.row(A).unwrap().error.is_none(), "editing removes the message");
    assert_eq!(
        v.key(SettingsKey::Enter, &Fixed).action,
        Some(SettingsAction::Rename(A, "Microphone".into()))
    );

    // Empty restores the Windows name; Escape cancels; unchanged does nothing.
    click_part(&mut v, A, Part::Rename);
    v.key(SettingsKey::Delete, &Fixed);
    assert_eq!(
        v.key(SettingsKey::Enter, &Fixed).action,
        Some(SettingsAction::Rename(A, String::new()))
    );
    click_part(&mut v, A, Part::Rename);
    type_text(&mut v, "Other");
    assert_eq!(v.key(SettingsKey::Escape, &Fixed).action, None);
    assert!(!v.editing());
    click_part(&mut v, A, Part::Rename);
    assert_eq!(v.key(SettingsKey::Enter, &Fixed).action, None);
}

#[test]
fn leaving_the_text_box_commits_a_valid_name_and_drops_a_refused_one() {
    let mut v = devices();
    click_part(&mut v, A, Part::Rename);
    type_text(&mut v, "Desk");
    let r = v.key(SettingsKey::Tab { back: false }, &Fixed);
    assert_eq!(r.action, Some(SettingsAction::Rename(A, "Desk".into())));
    assert!(!v.editing());
    click_part(&mut v, A, Part::Rename);
    type_text(&mut v, "TV");
    let r = click_part(&mut v, MIC, Part::Visibility);
    assert!(!v.editing());
    assert_eq!(r.action, Some(SettingsAction::SetHidden(MIC, true)));
}

#[test]
fn the_text_box_edits_like_windows() {
    let mut v = devices();
    click_part(&mut v, A, Part::Rename);
    type_text(&mut v, "abc");
    v.key(SettingsKey::Left { select: false }, &Fixed);
    v.key(SettingsKey::Backspace, &Fixed);
    assert_eq!(v.edit_of(A).unwrap().text, "ac");
    v.key(SettingsKey::Home { select: false }, &Fixed);
    v.key(SettingsKey::End { select: true }, &Fixed);
    let copy = v.key(SettingsKey::Copy, &Fixed);
    assert_eq!(copy.clipboard.as_deref(), Some("ac"));
    let cut = v.key(SettingsKey::Cut, &Fixed);
    assert_eq!(cut.clipboard.as_deref(), Some("ac"));
    assert_eq!(v.edit_of(A).unwrap().text, "");
    v.paste("line one\nline two", &Fixed);
    assert_eq!(v.edit_of(A).unwrap().text, "line one");
    // Space is a typed character, not a button press.
    assert_eq!(v.key(SettingsKey::Space, &Fixed).action, None);
    // Never longer than the longest name.
    type_text(&mut v, &"x".repeat(200));
    assert_eq!(v.edit_of(A).unwrap().len(), t::NAME_MAX_CHARS);
    // A click in the box places the caret.
    let text_box = part(&v, A, Part::TextBox);
    let p = Point::new(text_box.x + t::TEXT_BOX.1 + 7.0 * 2.0 + 1.0, text_box.y + 10.0);
    v.key(SettingsKey::SelectAll, &Fixed);
    type_text(&mut v, "abcd");
    v.press(p, &Fixed);
    v.release(p, &Fixed);
    assert_eq!(v.edit_of(A).unwrap().caret, 2);
}

#[test]
fn the_icon_picker_opens_under_its_row_and_reports_the_choice() {
    let mut v = devices();
    let before = v.layout.row(B).unwrap().rect.y;
    click_part(&mut v, A, Part::Icon);
    let picker = v.layout.picker.clone().expect("open");
    assert_eq!(picker.cells.len(), DeviceIcon::ALL.len() + 1);
    assert!(v.layout.row(B).unwrap().rect.y > before, "the next rows make room");
    assert_eq!(v.focus, Some(Target::Cell(0)), "automatic is the current choice");
    // Keyboard: right then Enter picks the first catalogue icon.
    v.key(SettingsKey::Right { select: false }, &Fixed);
    let r = v.key(SettingsKey::Enter, &Fixed);
    assert_eq!(r.action, Some(SettingsAction::SetIcon(A, Some(DeviceIcon::ALL[0]))));
    assert!(v.layout.picker.is_none());
    assert_eq!(v.focus, Some(Target::Row(A, Part::Icon)));
    // Mouse: a click on a cell picks it; on "Automatic", back to automatic... unchanged.
    click_part(&mut v, A, Part::Icon);
    let cell = v.layout.on_screen(v.layout.picker.as_ref().unwrap().cells[3]);
    assert_eq!(
        click(&mut v, center(cell)).action,
        Some(SettingsAction::SetIcon(A, Some(DeviceIcon::ALL[2])))
    );
    click_part(&mut v, A, Part::Icon);
    let auto = v.layout.on_screen(v.layout.picker.as_ref().unwrap().cells[0]);
    assert_eq!(click(&mut v, center(auto)).action, None, "already automatic");
    // Escape closes without a choice.
    click_part(&mut v, A, Part::Icon);
    assert_eq!(v.key(SettingsKey::Escape, &Fixed).action, None);
    assert!(v.layout.picker.is_none());
}

#[test]
fn forgetting_asks_first() {
    let mut v = devices();
    assert_eq!(click_part(&mut v, C, Part::Forget).action, None);
    assert!(v.confirm.is_some());
    assert_eq!(v.focus, Some(Target::ConfirmCancel), "the safe choice has the focus");
    // A click outside only cancels.
    assert_eq!(click_part(&mut v, A, Part::Visibility).action, None);
    assert!(v.confirm.is_none());
    // Escape cancels; the confirm button forgets.
    click_part(&mut v, C, Part::Forget);
    v.key(SettingsKey::Escape, &Fixed);
    assert!(v.confirm.is_none());
    click_part(&mut v, C, Part::Forget);
    let ok = v.confirm_geometry(&Fixed).unwrap().ok;
    assert_eq!(click(&mut v, center(ok)).action, Some(SettingsAction::Forget(C)));
    // Keyboard: Tab to the confirm button, Enter.
    click_part(&mut v, C, Part::Forget);
    v.key(SettingsKey::Tab { back: false }, &Fixed);
    assert_eq!(v.focus, Some(Target::ConfirmOk));
    assert_eq!(v.key(SettingsKey::Enter, &Fixed).action, Some(SettingsAction::Forget(C)));
}

#[test]
fn rows_are_described_for_screen_readers() {
    let mut v = devices();
    let tree = v.access_tree(&Fixed);
    let group = tree.find(ids::ROW | (A.0 << 4)).unwrap();
    assert_eq!(group.name, "Speakers");
    assert!(group.help.as_deref().unwrap().contains("Alt+Up"));
    let names: Vec<&str> = group.children.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Change icon", "Rename", "Hide"]);
    let forget = tree.find(ids::ROW | (C.0 << 4) | 4).unwrap();
    assert_eq!(forget.name, "Forget");
    // The rename box exposes its text.
    let rename = ids::ROW | (A.0 << 4) | 2;
    v.access_action(rename, AccessAction::Invoke, &Fixed);
    type_text(&mut v, "Desk");
    let tree = v.access_tree(&Fixed);
    let edit = tree.find(ids::ROW | (A.0 << 4) | 5).unwrap();
    assert_eq!((edit.role, edit.value.as_deref()), (AccessRole::Edit, Some("Desk")));
    assert_eq!(v.focused_access_id(), Some(ids::ROW | (A.0 << 4) | 5));
    // The picker is a list whose current choice is selected.
    v.key(SettingsKey::Escape, &Fixed);
    let icon_button = ids::ROW | (A.0 << 4) | 1;
    assert!(v.access_action(icon_button, AccessAction::Collapse, &Fixed) == Response::default());
    assert!(v.access_action(icon_button, AccessAction::Expand, &Fixed).focus_moved);
    assert_eq!(v.access_tree(&Fixed).find(icon_button).unwrap().expanded, Some(true));
    let tree = v.access_tree(&Fixed);
    let picker = tree.find(ids::PICKER).unwrap();
    assert_eq!(picker.children.len(), DeviceIcon::ALL.len() + 1);
    assert_eq!(picker.children[0].selected, Some(true));
    let r = v.access_action(ids::CELL | (A.0 << 8) | 1, AccessAction::Select, &Fixed);
    assert_eq!(r.action, Some(SettingsAction::SetIcon(A, Some(DeviceIcon::ALL[0]))));
    // The confirmation is described while open.
    v.access_action(ids::ROW | (C.0 << 4) | 4, AccessAction::Invoke, &Fixed);
    let tree = v.access_tree(&Fixed);
    assert!(tree.find(ids::CONFIRM_OK).is_some());
    let r = v.access_action(ids::CONFIRM_OK, AccessAction::Invoke, &Fixed);
    assert_eq!(r.action, Some(SettingsAction::Forget(C)));
}

#[test]
fn content_updates_keep_or_drop_the_row_state() {
    let mut v = devices();
    click_part(&mut v, A, Part::Rename);
    type_text(&mut v, "Desk");
    // A live update (another device plugged in) keeps the rename in progress.
    let mut c = v.content().clone();
    c.blocks.push(row(13, 0, "New", true));
    v.set_content(c, &Fixed);
    assert!(v.editing());
    assert_eq!(v.edit_of(A).unwrap().text, "Desk");
    // The renamed row goes away: everything about it is forgotten.
    let mut c = v.content().clone();
    c.blocks.retain(|b| !matches!(b, Block::Device(r) if r.key == A));
    v.set_content(c, &Fixed);
    assert!(!v.editing());
    assert_eq!(v.focus, None);
}

#[test]
fn review_fixes_of_the_rename_and_drag_paths() {
    // A click on the pencil of the row being renamed commits it (mouse path).
    let mut v = devices();
    click_part(&mut v, A, Part::Rename);
    type_text(&mut v, "Desk");
    let r = click_part(&mut v, A, Part::Rename);
    assert_eq!(r.action, Some(SettingsAction::Rename(A, "Desk".into())));
    assert!(!v.editing(), "the box does not reopen");

    // Leaving a refused name lays the row out again (no message line left behind).
    let normal = v.layout.row(A).unwrap().rect.height;
    click_part(&mut v, A, Part::Rename);
    type_text(&mut v, "Headset");
    v.key(SettingsKey::Enter, &Fixed);
    assert!(v.layout.row(A).unwrap().rect.height >= normal);
    click_part(&mut v, MIC, Part::Icon);
    assert!(!v.editing());
    assert_eq!(v.layout.row(A).unwrap().rect.height, normal);
    assert!(v.layout.row(A).unwrap().error.is_none());

    // Closing the window commits a rename in progress; a refused one is dropped.
    v.key(SettingsKey::Escape, &Fixed);
    click_part(&mut v, A, Part::Rename);
    type_text(&mut v, "Bureau");
    assert_eq!(v.end_edit(&Fixed).action, Some(SettingsAction::Rename(A, "Bureau".into())));
    click_part(&mut v, A, Part::Rename);
    type_text(&mut v, "TV");
    assert_eq!(v.end_edit(&Fixed).action, None);

    // An assistive tool moving the focus elsewhere commits the rename.
    click_part(&mut v, A, Part::Rename);
    type_text(&mut v, "Salon");
    let r = v.access_action(ids::ROW | (B.0 << 4) | 3, AccessAction::Focus, &Fixed);
    assert_eq!(r.action, Some(SettingsAction::Rename(A, "Salon".into())));
    assert!(!v.editing());

    // Escape cancels a drag: the release then moves nothing.
    let start = center(part(&v, A, Part::Body));
    v.press(start, &Fixed);
    v.pointer_move(Point::new(start.x, start.y + 200.0), &Fixed);
    v.key(SettingsKey::Escape, &Fixed);
    assert_eq!(v.release(Point::new(start.x, start.y + 200.0), &Fixed).action, None);

    // Pasting skips empty leading lines.
    click_part(&mut v, A, Part::Rename);
    v.paste("\n\n  Kitchen \nmore", &Fixed);
    assert_eq!(v.edit_of(A).unwrap().text, "Kitchen");
}

// ----- Text fields, buttons, notices, close question -----

const HOST: SettingKey = SettingKey(30);
const PORT: SettingKey = SettingKey(31);
const PASSWORD: SettingKey = SettingKey(32);
const TEST: SettingKey = SettingKey(33);
const SAVE: SettingKey = SettingKey(34);
const NOTICE: SettingKey = SettingKey(35);

fn field(value: &str, password: bool, digits_only: bool) -> CardControl {
    CardControl::TextField(TextField {
        value: value.into(),
        placeholder: "Placeholder".into(),
        password,
        digits_only,
        max_chars: 12,
    })
}

fn form(value: &str, question: bool) -> SettingsContent {
    let mut content = content(
        GENERAL,
        vec![
            card(HOST, "Address", field(value, false, false)),
            card(PORT, "Port", field("1883", false, true)),
            card(PASSWORD, "Password", field("", true, false)),
            Block::Notice(Notice {
                key: NOTICE,
                severity: Severity::Caution,
                title: "Unknown certificate".into(),
                message: "Windows does not trust it".into(),
                action: Some("Trust".into()),
            }),
            Block::Buttons(vec![
                Button { key: TEST, label: "Test".into(), accent: true, enabled: true },
                Button { key: SAVE, label: "Save".into(), accent: false, enabled: false },
            ]),
        ],
    );
    content.close_question = question.then(|| CloseQuestion {
        title: "Save the changes?".into(),
        text: "They are not saved yet".into(),
        save: "Save".into(),
        discard: "Don't save".into(),
        cancel: "Cancel".into(),
    });
    content
}

fn typed(v: &mut SettingsView, text: &str) -> Option<SettingsAction> {
    v.text_input(text, &Fixed).action
}

#[test]
fn a_field_edits_its_text_and_reports_every_change() {
    let mut v = view(form("broker", false));
    let at = center(control(&v, HOST));
    click(&mut v, at);
    assert!(v.editing(), "typed characters reach the field");
    v.key(SettingsKey::End { select: false }, &Fixed);
    assert_eq!(typed(&mut v, ".lan"), Some(SettingsAction::SetText(HOST, "broker.lan".into())));
    let back = v.key(SettingsKey::Backspace, &Fixed);
    assert_eq!(back.action, Some(SettingsAction::SetText(HOST, "broker.la".into())));
    // Longer than its limit: cut.
    assert_eq!(
        typed(&mut v, "xxxxxxxx"),
        Some(SettingsAction::SetText(HOST, "broker.laxxx".into()))
    );
    // Moving the caret changes nothing.
    assert_eq!(v.key(SettingsKey::Home { select: false }, &Fixed).action, None);
    // The application echoes the text; a different one (reset) replaces it.
    v.set_content(form("other", false), &Fixed);
    assert_eq!(v.field_of(HOST).unwrap().text, "other");
    // Leaving the field ends the editing.
    v.key(SettingsKey::Tab { back: false }, &Fixed);
    assert_eq!(v.focus, Some(Target::Control(PORT)));
    assert_eq!(v.field_of(PORT).unwrap().text, "1883", "the next field takes over");
    assert_eq!(v.field_of(HOST), None);
}

#[test]
fn a_port_takes_digits_and_a_password_is_never_copied() {
    let mut v = view(form("broker", false));
    let at = center(control(&v, PORT));
    click(&mut v, at);
    v.key(SettingsKey::SelectAll, &Fixed);
    assert_eq!(typed(&mut v, "8a8b83"), Some(SettingsAction::SetText(PORT, "8883".into())));

    let at = center(control(&v, PASSWORD));
    click(&mut v, at);
    assert_eq!(typed(&mut v, "secret"), Some(SettingsAction::SetText(PASSWORD, "secret".into())));
    let edit = v.field_of(PASSWORD).unwrap();
    assert_eq!(edit.shown(), "●●●●●●");
    v.key(SettingsKey::SelectAll, &Fixed);
    let copy = v.key(SettingsKey::Copy, &Fixed);
    assert_eq!(copy.clipboard, None, "WinUI PasswordBox: no copy");
    assert_eq!(v.key(SettingsKey::Cut, &Fixed).action, None, "nor cut");
    // Screen readers get the field, never its text.
    let tree = v.access_tree(&Fixed);
    let node = find(&tree, ids::CONTROL | (PASSWORD.0 << 4)).expect("password field");
    assert_eq!(node.role, AccessRole::Edit);
    assert_eq!(node.value, None);
    let host = find(&tree, ids::CONTROL | (HOST.0 << 4)).expect("address field");
    assert_eq!(host.value.as_deref(), Some("broker"));
}

fn find(node: &AccessNode, id: u64) -> Option<&AccessNode> {
    if node.id == id {
        return Some(node);
    }
    node.children.iter().find_map(|c| find(c, id))
}

#[test]
fn buttons_and_the_notice_button_are_invoked_and_reached_by_tab() {
    let mut v = view(form("broker", false));
    let test = v.layout.on_screen(v.layout.button(TEST).unwrap());
    assert_eq!(click(&mut v, center(test)).action, Some(SettingsAction::Invoke(TEST)));
    let save = v.layout.on_screen(v.layout.button(SAVE).unwrap());
    assert_eq!(click(&mut v, center(save)).action, None, "disabled");
    let trust = v.layout.on_screen(v.layout.button(NOTICE).unwrap());
    assert_eq!(click(&mut v, center(trust)).action, Some(SettingsAction::Invoke(NOTICE)));
    // Tab order: navigation, the three fields, the notice button, the enabled button.
    let stops = v.stops();
    assert_eq!(
        stops[1..],
        [
            Target::Control(HOST),
            Target::Control(PORT),
            Target::Control(PASSWORD),
            Target::Control(NOTICE),
            Target::Control(TEST)
        ]
    );
    v.focus = Some(Target::Control(TEST));
    assert_eq!(v.key(SettingsKey::Enter, &Fixed).action, Some(SettingsAction::Invoke(TEST)));
    // The notice sits between the fields and the buttons, never over them.
    let notice = &v.layout.notices[0];
    assert!(notice.rect.bottom() <= v.layout.button(TEST).unwrap().y);
    assert!(notice.action.unwrap().right() <= notice.rect.right());
}

#[test]
fn closing_with_unsaved_changes_asks_first() {
    let mut v = view(form("broker", false));
    assert!(!v.request_close(&Fixed).keep_open, "nothing to lose: it closes");

    let mut v = view(form("broker", true));
    let asked = v.request_close(&Fixed);
    assert!(asked.keep_open && v.dialog_open());
    assert_eq!(v.focus, Some(Target::DialogSave), "the default button");
    // Modal: the page does not react.
    let at = center(control(&v, HOST));
    assert_eq!(click(&mut v, at).action, None);
    assert!(v.dialog_open());
    // Escape is "Cancel": the window stays, nothing is answered.
    assert_eq!(v.key(SettingsKey::Escape, &Fixed).action, None);
    assert!(!v.dialog_open());

    v.request_close(&Fixed);
    v.key(SettingsKey::Tab { back: false }, &Fixed);
    assert_eq!(v.key(SettingsKey::Enter, &Fixed).action, Some(SettingsAction::CloseAnswer(false)));

    v.request_close(&Fixed);
    let (d, _, _, buttons) = v.dialog_geometry(&Fixed).unwrap();
    assert!(d.rect.x > 0.0 && d.rect.right() < 900.0, "centred in the window");
    assert_eq!(buttons.len(), 3);
    let save = buttons[0].3;
    assert_eq!(click(&mut v, center(save)).action, Some(SettingsAction::CloseAnswer(true)));
    // Screen readers see the question and its buttons.
    v.request_close(&Fixed);
    let tree = v.access_tree(&Fixed);
    assert!(find(&tree, ids::DIALOG_DISCARD).is_some());
    let response = v.access_action(ids::DIALOG_DISCARD, AccessAction::Invoke, &Fixed);
    assert_eq!(response.action, Some(SettingsAction::CloseAnswer(false)));
}

fn with_question(mut content: SettingsContent) -> SettingsContent {
    content.question = Some(Question {
        title: "Hide the Windows volume icon?".into(),
        text: "BetterVolume adds its own volume icon to the taskbar.".into(),
        primary: "Hide the Windows icon".into(),
        secondary: "Keep both".into(),
    });
    content
}

#[test]
fn a_question_of_the_content_shows_at_once_with_two_buttons() {
    let mut v = view(with_question(form("broker", false)));
    assert!(v.dialog_open(), "shown when the window opens with it");
    assert_eq!(v.focus, Some(Target::DialogSave), "the primary button");
    let (_, title, _, buttons) = v.dialog_geometry(&Fixed).unwrap();
    assert_eq!(title, "Hide the Windows volume icon?");
    assert_eq!(buttons.len(), 2, "no Cancel");
    assert!(buttons[0].2 && !buttons[1].2, "the first is the accent one");
    assert!(buttons[0].3.right() < buttons[1].3.x, "side by side");
    // Modal: the page does not react.
    let at = center(control(&v, HOST));
    assert_eq!(click(&mut v, at).action, None);
    // Tab moves between the two buttons only.
    v.key(SettingsKey::Tab { back: false }, &Fixed);
    assert_eq!(v.focus, Some(Target::DialogDiscard));
    v.key(SettingsKey::Tab { back: false }, &Fixed);
    assert_eq!(v.focus, Some(Target::DialogSave));
    let primary = v.dialog_geometry(&Fixed).unwrap().3[0].3;
    assert_eq!(click(&mut v, center(primary)).action, Some(SettingsAction::QuestionAnswer(true)));
    assert!(!v.dialog_open());
}

#[test]
fn dismissing_the_question_answers_its_secondary_button() {
    // Escape.
    let mut v = view(with_question(form("broker", false)));
    assert_eq!(
        v.key(SettingsKey::Escape, &Fixed).action,
        Some(SettingsAction::QuestionAnswer(false))
    );
    // The secondary button, through a screen reader.
    let mut v = view(with_question(form("broker", false)));
    let response = v.access_action(ids::DIALOG_DISCARD, AccessAction::Invoke, &Fixed);
    assert_eq!(response.action, Some(SettingsAction::QuestionAnswer(false)));
    // Closing the window: answered, and the window may close.
    let mut v = view(with_question(form("broker", false)));
    let closing = v.request_close(&Fixed);
    assert_eq!(closing.action, Some(SettingsAction::QuestionAnswer(false)));
    assert!(!closing.keep_open && !v.dialog_open());
    // Closing with unsaved changes: answered, then the close question takes over.
    let mut v = view(with_question(form("broker", true)));
    let closing = v.request_close(&Fixed);
    assert_eq!(closing.action, Some(SettingsAction::QuestionAnswer(false)));
    assert!(closing.keep_open && v.dialog_open());
    assert_eq!(v.dialog_geometry(&Fixed).unwrap().3.len(), 3, "the close question");
}

#[test]
fn a_notice_the_window_was_opened_for_is_scrolled_into_view_once() {
    let mut long = form("broker", false);
    for i in 0..12 {
        long.blocks.insert(0, card(SettingKey(100 + i), "Filler", CardControl::None));
    }
    long.reveal = Some(NOTICE);
    let mut v = SettingsView::new(long.clone(), Size::new(900.0, 400.0), &Fixed, 1.0);
    let notice = v.layout.notices[0].rect;
    assert!(v.scroll() > 0.0, "scrolled down to it");
    assert!(notice.y >= v.scroll() && notice.bottom() <= v.scroll() + 400.0, "fully shown");
    // The user scrolls back up: the same content does not pull the page down again.
    v.layout.scroll_to(0.0);
    v.set_content(long, &Fixed);
    assert_eq!(v.scroll(), 0.0);
}

#[test]
fn the_question_goes_when_the_content_drops_it_and_comes_when_it_brings_it() {
    let mut v = view(form("broker", false));
    assert!(!v.dialog_open());
    v.set_content(with_question(form("broker", false)), &Fixed);
    assert!(v.dialog_open());
    v.set_content(form("broker", false), &Fixed);
    assert!(!v.dialog_open());
}

#[test]
fn a_dialog_grows_until_its_button_labels_fit() {
    let mut content = with_question(form("broker", false));
    if let Some(q) = content.question.as_mut() {
        q.primary = "A very long primary button label for a long language".into();
    }
    let v = view(content);
    let (d, _, _, buttons) = v.dialog_geometry(&Fixed).unwrap();
    for (_, label, _, rect) in &buttons {
        assert!(
            rect.width >= Fixed.width(label, crate::tokens::typography::BODY),
            "{label} fits in {}",
            rect.width
        );
    }
    assert!(d.rect.width > crate::tokens::settings::DIALOG.0, "wider than its usual width");
    assert!(d.rect.right() <= v.size().width, "still inside the window");
}

#[test]
fn a_long_notice_title_wraps_above_its_message() {
    let notice = |title: &str| {
        content(
            GENERAL,
            vec![Block::Notice(Notice {
                key: NOTICE,
                severity: Severity::Caution,
                title: title.into(),
                message: "The option was turned off to be safe".into(),
                action: None,
            })],
        )
    };
    let (min_width, min_height) = t::MIN_SIZE;
    let size = Size::new(min_width, min_height);
    let short = SettingsView::new(notice("Shown again"), size, &Fixed, 1.0);
    let long = SettingsView::new(notice(&"Very long title ".repeat(6)), size, &Fixed, 1.0);
    let (title, lines) = long.layout.notices[0].title;
    assert!(lines > 1, "wrapped, not cut");
    let body = crate::tokens::typography::BODY.line_height;
    assert!(title.height >= lines as f32 * body - 0.01);
    let (message, _) = long.layout.notices[0].message;
    assert!(message.y >= title.bottom() - 0.01, "the message under the whole title");
    assert!(long.layout.notices[0].rect.bottom() >= message.bottom());
    assert_eq!(short.layout.notices[0].title.1, 1);
}

const PICTURES: SettingKey = SettingKey(20);
const SIZE: SettingKey = SettingKey(21);
const PICTURE: SettingKey = SettingKey(22);
const LOOK: SettingKey = SettingKey(23);

fn picture_card(selected: [usize; 3]) -> Block {
    use crate::controls::tray::{TrayIcon, TrayLook};
    let icon = TrayIcon { level: None, badge: None, large: false, look: TrayLook::Windows };
    let group = |key, label: &str, names: [&str; 2], selected| PictureGroup {
        key,
        label: label.into(),
        tiles: names.iter().map(|n| PictureTile { label: (*n).into(), picture: icon }).collect(),
        selected,
    };
    Block::Pictures(PictureCard {
        key: PICTURES,
        icon: Some(Icon::Taskbar),
        title: "Tray icon".into(),
        description: Some("Choose how it looks".into()),
        groups: vec![
            group(SIZE, "Size", ["Normal", "Large"], selected[0]),
            group(PICTURE, "Picture", ["Speaker", "Device"], selected[1]),
            group(LOOK, "Look", ["Windows", "BetterVolume"], selected[2]),
        ],
    })
}

fn with_pictures(selected: [usize; 3]) -> SettingsContent {
    content(
        GENERAL,
        vec![
            card(AUTOSTART, "Start with Windows", toggle(true)),
            picture_card(selected),
            card(LANGUAGE, "Language", choice(0, 3)),
        ],
    )
}

fn tile(v: &SettingsView, key: SettingKey, i: usize) -> Rect {
    v.layout.on_screen(v.layout.tile(key, i).unwrap())
}

#[test]
fn picture_groups_stand_side_by_side_and_wrap_in_a_narrow_window() {
    // At the window's default size.
    let (w, h) = t::DEFAULT_SIZE;
    let v = SettingsView::new(with_pictures([0, 0, 0]), Size::new(w, h), &Fixed, 1.0);
    let card = &v.layout.pictures[0];
    let groups = &card.groups;
    assert_eq!(groups.len(), 3);
    // Side by side, under the card's text, inside the card.
    let text_bottom = card.description.unwrap().0.bottom();
    for g in groups {
        assert!(g.label.y >= text_bottom);
        assert_eq!(g.label.y, groups[0].label.y);
        for r in &g.tiles {
            assert!(r.y >= g.label.bottom());
            assert!(r.x >= card.rect.x + t::CARD_PADDING.0);
            assert!(r.right() <= card.rect.right() - t::CARD_PADDING.0);
            assert!(r.bottom() <= card.rect.bottom() - t::CARD_PADDING.1 + 0.01);
            assert!(r.width >= t::PICTURE_TILE.0);
        }
        assert!(g.tiles[1].x >= g.tiles[0].right() + t::PICTURE_GAPS.0 - 0.01);
    }
    for pair in groups.windows(2) {
        assert!(pair[1].label.x >= pair[0].tiles[1].right() + t::PICTURE_GAPS.1 - 0.01);
    }
    // The cards around keep their place in the flow.
    assert!(v.layout.cards[1].rect.y >= card.rect.bottom() + t::CARD_GAP - 0.01);
    // A narrow window puts the groups that do not fit on the next rows.
    let narrow = SettingsView::new(with_pictures([0, 0, 0]), Size::new(700.0, 600.0), &Fixed, 1.0);
    let n = &narrow.layout.pictures[0];
    assert!(n.groups[2].label.y > n.groups[0].tiles[0].bottom());
    for g in &n.groups {
        assert!(g.tiles[1].right() <= n.rect.right() - t::CARD_PADDING.0 + 0.01);
        assert!(g.tiles[1].bottom() <= n.rect.bottom());
    }
}

#[test]
fn a_click_on_a_tile_chooses_it() {
    let mut v = view(with_pictures([0, 1, 0]));
    let p = center(tile(&v, LOOK, 1));
    let r = click(&mut v, p);
    assert_eq!(r.action, Some(SettingsAction::Choose(LOOK, 1)));
    assert_eq!(v.focus, Some(Target::Tile(LOOK, 1)));
    // The chosen tile is already chosen.
    let p = center(tile(&v, PICTURE, 1));
    let r = click(&mut v, p);
    assert_eq!(r.action, None);
    // Between tiles: nothing.
    let gap = Point::new(tile(&v, SIZE, 0).right() + 2.0, center(tile(&v, SIZE, 0)).y);
    assert_eq!(click(&mut v, gap).action, None);
}

#[test]
fn each_group_of_tiles_is_one_tab_stop_and_the_arrows_choose() {
    let mut v = view(with_pictures([1, 0, 0]));
    v.key(SettingsKey::Tab { back: false }, &Fixed); // navigation
    v.key(SettingsKey::Tab { back: false }, &Fixed); // autostart
    v.key(SettingsKey::Tab { back: false }, &Fixed);
    assert_eq!(v.focus, Some(Target::Tile(SIZE, 1)), "the chosen tile of the first group");
    v.key(SettingsKey::Tab { back: false }, &Fixed);
    assert_eq!(v.focus, Some(Target::Tile(PICTURE, 0)));
    let r = v.key(SettingsKey::Right { select: false }, &Fixed);
    assert_eq!(r.action, Some(SettingsAction::Choose(PICTURE, 1)));
    assert!(r.focus_moved);
    assert_eq!(v.focus, Some(Target::Tile(PICTURE, 1)));
    v.set_content(with_pictures([1, 1, 0]), &Fixed);
    // The last tile stays the last: no wrapping into the next group.
    let r = v.key(SettingsKey::Down, &Fixed);
    assert_eq!(r.action, None);
    assert_eq!(v.focus, Some(Target::Tile(PICTURE, 1)));
    let r = v.key(SettingsKey::Home { select: false }, &Fixed);
    assert_eq!(r.action, Some(SettingsAction::Choose(PICTURE, 0)));
    v.set_content(with_pictures([1, 0, 0]), &Fixed);
    let r = v.key(SettingsKey::End { select: false }, &Fixed);
    assert_eq!(r.action, Some(SettingsAction::Choose(PICTURE, 1)));
    v.set_content(with_pictures([1, 1, 0]), &Fixed);
    v.key(SettingsKey::Tab { back: false }, &Fixed);
    assert_eq!(v.focus, Some(Target::Tile(LOOK, 0)));
    v.key(SettingsKey::Tab { back: true }, &Fixed);
    assert_eq!(v.focus, Some(Target::Tile(PICTURE, 1)));
    v.key(SettingsKey::Tab { back: false }, &Fixed);
    v.key(SettingsKey::Tab { back: false }, &Fixed);
    assert_eq!(v.focus, Some(Target::Control(LANGUAGE)));
}

#[test]
fn screen_readers_see_a_list_per_group_and_can_choose_a_tile() {
    let mut v = view(with_pictures([0, 1, 0]));
    let tree = v.access_tree(&Fixed);
    let card = tree.find(ids::CARD | (PICTURES.0 << 4)).unwrap();
    assert_eq!(card.name, "Tray icon");
    assert_eq!(card.help.as_deref(), Some("Choose how it looks"));
    assert_eq!(card.children.len(), 3);
    let picture = &card.children[1];
    assert_eq!(picture.role, AccessRole::List);
    assert_eq!(picture.name, "Picture");
    let names: Vec<_> = picture.children.iter().map(|c| (c.name.as_str(), c.selected)).collect();
    assert_eq!(names, [("Speaker", Some(false)), ("Device", Some(true))]);
    assert!(picture.children.iter().all(|c| c.role == AccessRole::ListItem && c.focusable));
    let large = ids::TILE | (SIZE.0 << 8) | 1;
    let r = v.access_action(large, AccessAction::Select, &Fixed);
    assert_eq!(r.action, Some(SettingsAction::Choose(SIZE, 1)));
    v.access_action(large, AccessAction::Focus, &Fixed);
    assert_eq!(v.focused_access_id(), Some(large));
    // A tile that does not exist is ignored.
    let r = v.access_action(ids::TILE | (SIZE.0 << 8) | 5, AccessAction::Select, &Fixed);
    assert_eq!(r.action, None);
}
