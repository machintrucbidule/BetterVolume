//! The flyout panel (FR-020 to FR-065): a retained view of design-system controls.
//!
//! The application describes what to show ([`FlyoutContent`]); the view lays it out,
//! paints it, handles the pointer, the wheel and the keyboard (FR-024), keeps the
//! interaction state across content updates (hover, focus, a slider being dragged), and
//! reports what the user asked for as [`FlyoutAction`]s. It also describes itself for
//! screen readers ([`FlyoutView::access_tree`]).
//!
//! The shell hosts the view in its window: it forwards input, asks for the height, and
//! paints at the height it is currently showing (the height changes smoothly).

mod content;
mod layout;
mod paint;

use std::hash::{Hash, Hasher};

pub use content::{
    AppsSection, DeviceTile, Devices, FlyoutAction, FlyoutContent, FlyoutTab, Footer, ItemKey,
    PanelHeight, SliderTarget, Tabs, UpdateLine, VolumeControl, VolumeIcon,
};
pub use layout::{tile_rows, TextMeasure};

use crate::access::{AccessAction, AccessNode, AccessRange, AccessRole};
use crate::backend::{Canvas, Graphics};
use crate::tokens::slider::THUMB;
use crate::tokens::typography::TextStyle;
use crate::{Point, Rect, Size, Theme};
use layout::{Block, Layout, NodeKey};

/// Measures text with the backend's fonts.
pub struct GraphicsMeasure<'a> {
    /// The graphics objects.
    pub graphics: &'a Graphics,
    /// Windows text scaling.
    pub text_scale: f32,
}

impl TextMeasure for GraphicsMeasure<'_> {
    fn width(&self, text: &str, style: TextStyle) -> f32 {
        self.graphics.measure_text(text, style, self.text_scale).map_or(0.0, |s| s.width)
    }

    fn lines(&self, text: &str, style: TextStyle, width: f32) -> u32 {
        self.graphics.line_count(text, style, self.text_scale, width).unwrap_or(1)
    }
}

/// A key the flyout reacts to (Escape is the shell's: it closes the panel).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlyoutKey {
    /// Tab (`back`: with Shift).
    Tab {
        /// Shift was held.
        back: bool,
    },
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Page Up.
    PageUp,
    /// Page Down.
    PageDown,
    /// Home.
    Home,
    /// End.
    End,
    /// Enter or Space.
    Activate,
}

/// What the view asks its host to do after an input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Response {
    /// Paint again.
    pub redraw: bool,
    /// The user asked for this.
    pub action: Option<FlyoutAction>,
    /// Start (`Some(true)`) or stop (`Some(false)`) capturing the pointer (slider drag).
    pub capture: Option<bool>,
    /// The natural height may have changed (content applied after a drag).
    pub resized: bool,
    /// The keyboard focus moved (screen readers are told).
    pub focus_moved: bool,
}

impl Response {
    fn redraw() -> Response {
        Response { redraw: true, ..Response::default() }
    }

    fn act(action: FlyoutAction) -> Response {
        Response { redraw: true, action: Some(action), ..Response::default() }
    }
}

/// What a content update changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ContentChange {
    /// The panel height changed.
    pub resized: bool,
    /// Controls appeared or went away (tiles, rows, tabs).
    pub restructured: bool,
}

/// A slider being dragged and its value.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Drag {
    target: SliderTarget,
    value: u8,
}

/// The previous content, painted in the middle block while the panel shrinks (the rows
/// of a collapsing section are cut rather than vanishing at once).
#[derive(Clone, Debug)]
struct Ghost {
    content: FlyoutContent,
    layout: Layout,
}

/// The flyout view.
#[derive(Clone, Debug)]
pub struct FlyoutView {
    content: FlyoutContent,
    layout: Layout,
    /// Content received during a drag that would move the rows (FR-063): applied when the
    /// drag ends.
    pending: Option<FlyoutContent>,
    ghost: Option<Ghost>,
    max_height: f32,
    shown_height: f32,
    scroll: f32,
    hot: Option<NodeKey>,
    /// The control under the pointer, disabled ones included: a disabled control shows no
    /// highlight but still explains itself in its tooltip.
    hover: Option<NodeKey>,
    pressed: Option<NodeKey>,
    focus: Option<NodeKey>,
    focus_visible: bool,
    drag: Option<Drag>,
    /// The last tile that had the focus, to come back to it.
    last_tile: Option<ItemKey>,
}

impl FlyoutView {
    /// Lays out `content`. `max_height` is the tallest the panel may be (the work area):
    /// above it, the application list scrolls (FR-023).
    pub fn new(
        content: FlyoutContent,
        measure: &dyn TextMeasure,
        text_scale: f32,
        max_height: f32,
    ) -> FlyoutView {
        let layout = layout::layout(&content, measure, text_scale);
        let mut view = FlyoutView {
            content,
            layout,
            pending: None,
            ghost: None,
            max_height,
            shown_height: 0.0,
            scroll: 0.0,
            hot: None,
            hover: None,
            pressed: None,
            focus: None,
            focus_visible: false,
            drag: None,
            last_tile: None,
        };
        view.shown_height = view.height();
        view
    }

    /// The content shown.
    pub fn content(&self) -> &FlyoutContent {
        &self.content
    }

    /// Width of the panel.
    pub fn width(&self) -> f32 {
        self.layout.width
    }

    /// Height the panel should have: the whole height it is given, or, fitting
    /// the content, everything or the tallest allowed.
    pub fn height(&self) -> f32 {
        match self.content.height {
            PanelHeight::Full => self.max_height.max(1.0),
            PanelHeight::Content => {
                self.layout.natural_height().min(self.max_height).max(self.min_height())
            }
        }
    }

    fn min_height(&self) -> f32 {
        (self.layout.top_height + self.layout.bottom_height).min(self.max_height)
    }

    /// Size the panel should have.
    pub fn size(&self) -> Size {
        Size::new(self.width(), self.height())
    }

    /// Replaces the content, keeping the interaction state of the controls that remain.
    /// Says whether the height changed (the host then animates to
    /// [`height`](Self::height)) and whether controls appeared or went away.
    pub fn set_content(
        &mut self,
        content: FlyoutContent,
        measure: &dyn TextMeasure,
        text_scale: f32,
        max_height: f32,
    ) -> ContentChange {
        let before = self.height();
        self.max_height = max_height;
        if let Some(drag) = self.drag {
            if !self.content.same_structure(&content) {
                // FR-063: nothing moves under a dragged slider. Values follow at once; the
                // new rows wait for the end of the drag.
                self.update_values(&content, drag.target);
                self.pending = Some(content);
                return ContentChange::default();
            }
        }
        // Newer content replaces anything that waited for the end of a drag.
        self.pending = None;
        let restructured = !self.content.same_structure(&content);
        let old = std::mem::replace(&mut self.content, content);
        let old_layout =
            std::mem::replace(&mut self.layout, layout::layout(&self.content, measure, text_scale));
        // Only a panel fitting its content shrinks; a full-height one keeps its size.
        let shrinks = self.content.height == PanelHeight::Content
            && old.height == PanelHeight::Content
            && self.layout.middle_height < old_layout.middle_height;
        if shrinks {
            self.ghost = Some(Ghost { content: old, layout: old_layout });
        } else {
            self.ghost = None;
        }
        self.forget_missing();
        self.clamp_scroll();
        ContentChange { resized: (self.height() - before).abs() > 0.01, restructured }
    }

    /// Copies the values of `content` into the current rows (all but the dragged one).
    fn update_values(&mut self, content: &FlyoutContent, dragged: SliderTarget) {
        let targets: Vec<SliderTarget> = std::iter::once(SliderTarget::Main)
            .chain(self.content.visible_rows().iter().map(|r| r.target))
            .collect();
        for target in targets {
            let Some(new) = content.control(target).cloned() else { continue };
            if let Some(row) = self.content.control_mut(target) {
                let volume = if target == dragged { row.volume } else { new.volume };
                *row = VolumeControl { volume, ..new };
            }
        }
    }

    /// Drops interaction state pointing at controls that no longer exist.
    fn forget_missing(&mut self) {
        let exists =
            |layout: &Layout, key: Option<NodeKey>| key.filter(|k| layout.node(*k).is_some());
        self.hot = exists(&self.layout, self.hot);
        self.hover = exists(&self.layout, self.hover);
        self.pressed = exists(&self.layout, self.pressed);
        if self.focus.is_some() && exists(&self.layout, self.focus).is_none() {
            // The focused control went away (row removed): the next stop takes it.
            self.focus = self.stops().first().copied();
        }
        if self.focus == Some(NodeKey::Settings) && !self.content.footer.settings_enabled {
            self.focus = None;
        }
    }

    /// The host shows the panel this high (during a height change).
    pub fn set_shown_height(&mut self, height: f32) {
        self.shown_height = height;
        self.clamp_scroll();
    }

    /// The height change is over: the previous content is no longer painted.
    pub fn end_transition(&mut self) {
        self.ghost = None;
    }

    /// Whether the view shows nothing but its content: no pointer highlight, no focus
    /// indicator, no drag, no scrolling, no height change. Two views at rest with equal
    /// content paint the same pixels (the host may then keep the frame it has).
    pub fn at_rest(&self) -> bool {
        self.hot.is_none()
            && self.pressed.is_none()
            && !self.focus_visible
            && self.drag.is_none()
            && self.ghost.is_none()
            && self.scroll == 0.0
            && (self.shown_height - self.height()).abs() < 0.01
    }

    /// Whether a slider is being dragged.
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    // ----- Geometry -----

    /// Room between the top and bottom blocks.
    fn middle_room(&self) -> f32 {
        (self.shown_height - self.layout.top_height - self.layout.bottom_height).max(0.0)
    }

    /// Height of the visible part of the middle block. Full height: all of it when it fits
    /// (the free space then lies above it), else the room. Fitting the content: the
    /// room (the panel is as tall as the list, or cuts it while it changes height).
    fn middle_visible(&self) -> f32 {
        match self.content.height {
            PanelHeight::Full => self.layout.middle_height.min(self.middle_room()),
            PanelHeight::Content => self.middle_room(),
        }
    }

    /// Top of the visible part of the middle block: right above the bottom block (full
    /// height), or right below the top block (fitting the content).
    fn middle_top(&self) -> f32 {
        match self.content.height {
            PanelHeight::Full => {
                self.shown_height - self.layout.bottom_height - self.middle_visible()
            }
            PanelHeight::Content => self.layout.top_height,
        }
    }

    fn scroll_range(&self) -> f32 {
        (self.layout.middle_height - self.middle_visible()).max(0.0)
    }

    /// Whether the application list scrolls: it does not fit in the room left (full
    /// height), or the panel does not fit on the screen (fitting the content, FR-023).
    pub fn scrolls(&self) -> bool {
        let too_long = match self.content.height {
            PanelHeight::Full => self.layout.middle_height > self.middle_room() + 0.5,
            PanelHeight::Content => self.layout.natural_height() > self.max_height + 0.5,
        };
        too_long && self.ghost.is_none()
    }

    fn clamp_scroll(&mut self) {
        if !self.scrolls() {
            self.scroll = 0.0;
        }
        self.scroll = self.scroll.clamp(0.0, self.scroll_range());
    }

    /// Where a block starts on the panel.
    fn block_origin(&self, block: Block) -> f32 {
        match block {
            Block::Top => 0.0,
            Block::Middle => self.middle_top() - self.scroll,
            Block::Bottom => self.shown_height - self.layout.bottom_height,
        }
    }

    /// The visible part of the middle block, on the panel.
    fn middle_clip(&self) -> Rect {
        Rect::new(0.0, self.middle_top(), self.layout.width, self.middle_visible())
    }

    /// A node's rectangle on the panel.
    fn placed(&self, block: Block, rect: Rect) -> Rect {
        Rect::new(rect.x, rect.y + self.block_origin(block), rect.width, rect.height)
    }

    fn node_rect(&self, key: NodeKey) -> Option<Rect> {
        self.layout.node(key).map(|n| self.placed(n.block, n.rect))
    }

    /// The control at a point of the panel, disabled or not.
    fn hit_any(&self, p: Point) -> Option<NodeKey> {
        let clip = self.middle_clip();
        self.layout
            .nodes
            .iter()
            .filter(|n| n.block != Block::Middle || (clip.contains(p) && self.ghost.is_none()))
            .find(|n| self.placed(n.block, n.rect).contains(p))
            .map(|n| n.key)
    }

    /// The usable control at a point of the panel.
    fn hit(&self, p: Point) -> Option<NodeKey> {
        self.hit_any(p).filter(|k| self.enabled(*k))
    }

    fn enabled(&self, key: NodeKey) -> bool {
        match key {
            NodeKey::Settings => self.content.footer.settings_enabled,
            NodeKey::Mute(t) | NodeKey::Slider(t) => {
                self.content.control(t).is_some_and(|c| c.enabled)
            }
            _ => true,
        }
    }

    // ----- Pointer -----

    /// The pointer moved to `p` (panel coordinates, DIPs).
    pub fn pointer_move(&mut self, p: Point) -> Response {
        if let Some(drag) = self.drag {
            return self.drag_to(drag.target, p.x);
        }
        self.hover = self.hit_any(p);
        let hot = self.hover.filter(|k| self.enabled(*k));
        if hot == self.hot {
            return Response::default();
        }
        self.hot = hot;
        Response::redraw()
    }

    /// The pointer left the panel.
    pub fn pointer_leave(&mut self) -> Response {
        if self.drag.is_some() {
            return Response::default();
        }
        self.hover = None;
        if self.hot.is_none() {
            return Response::default();
        }
        self.hot = None;
        Response::redraw()
    }

    /// The main button was pressed at `p`.
    pub fn press(&mut self, p: Point) -> Response {
        let Some(key) = self.hit(p) else { return Response::default() };
        self.pressed = Some(key);
        self.hot = Some(key);
        self.set_focus(key, false);
        if let NodeKey::Slider(target) = key {
            // A click anywhere on the track jumps there (FR-051), then drags.
            let value = self.content.control(target).map_or(0, |c| c.volume);
            self.drag = Some(Drag { target, value });
            let mut response = self.drag_to(target, p.x);
            response.capture = Some(true);
            response.redraw = true;
            return response;
        }
        Response::redraw()
    }

    /// The main button was released at `p`.
    pub fn release(&mut self, p: Point) -> Response {
        let pressed = self.pressed.take();
        if let Some(drag) = self.drag.take() {
            if let Some(row) = self.content.control_mut(drag.target) {
                row.volume = drag.value;
            }
            self.hover = self.hit_any(p);
            self.hot = self.hit(p);
            let mut response =
                Response { redraw: true, capture: Some(false), ..Response::default() };
            if let Some(pending) = self.pending.take() {
                response.resized = true;
                self.pending = Some(pending);
            }
            return response;
        }
        match (pressed, self.hit(p)) {
            (Some(a), Some(b)) if a == b => self.activate(a),
            _ => Response::redraw(),
        }
    }

    /// Content waiting for the end of a drag (FR-063): the host applies it with
    /// [`set_content`] when [`Response::resized`] says so.
    pub fn take_pending(&mut self) -> Option<FlyoutContent> {
        self.pending.take()
    }

    /// The pointer was lost (another window captured it): a drag ends where it is.
    pub fn cancel_pointer(&mut self) -> Response {
        self.pressed = None;
        if let Some(drag) = self.drag.take() {
            if let Some(row) = self.content.control_mut(drag.target) {
                row.volume = drag.value;
            }
            return Response {
                redraw: true,
                resized: self.pending.is_some(),
                ..Response::default()
            };
        }
        Response::redraw()
    }

    fn drag_to(&mut self, target: SliderTarget, x: f32) -> Response {
        let Some(rect) = self.node_rect(NodeKey::Slider(target)) else {
            return Response::default();
        };
        let value = value_at(rect, x);
        match &mut self.drag {
            Some(drag) if drag.value == value => Response::default(),
            Some(drag) => {
                drag.value = value;
                Response::act(FlyoutAction::SetVolume(target, value))
            }
            None => Response::default(),
        }
    }

    /// The wheel turned by `notches` (positive away from the user) over `p`.
    pub fn wheel(&mut self, p: Point, notches: i32) -> Response {
        if notches == 0 {
            return Response::default();
        }
        if let Some(NodeKey::Slider(target)) = self.hit(p) {
            // The wheel acts on the slider under the pointer.
            let step = i32::from(self.content.wheel_step.max(1));
            return self.step_volume(target, notches * step);
        }
        if self.scrolls() && self.middle_clip().contains(p) {
            let line = crate::tokens::typography::CAPTION.line_height * 3.0;
            let before = self.scroll;
            self.scroll = (self.scroll - notches as f32 * line).clamp(0.0, self.scroll_range());
            if (self.scroll - before).abs() > 0.01 {
                self.hover = self.hit_any(p);
                self.hot = self.hit(p);
                return Response::redraw();
            }
        }
        Response::default()
    }

    fn step_volume(&mut self, target: SliderTarget, delta: i32) -> Response {
        let Some(row) = self.content.control_mut(target) else { return Response::default() };
        if !row.enabled {
            return Response::default();
        }
        let value = (i32::from(row.volume) + delta).clamp(0, 100) as u8;
        self.set_volume(target, value)
    }

    fn set_volume(&mut self, target: SliderTarget, value: u8) -> Response {
        let Some(row) = self.content.control_mut(target) else { return Response::default() };
        let muted = row.muted;
        if row.volume == value && !muted {
            return Response::default();
        }
        // Shown at once; the application confirms with its next content.
        row.volume = value;
        Response::act(FlyoutAction::SetVolume(target, value))
    }

    /// The action of a control (click, Enter, Space, screen reader).
    fn activate(&mut self, key: NodeKey) -> Response {
        if !self.enabled(key) {
            return Response::default();
        }
        match key {
            NodeKey::Tab(tab) => {
                if self.content.tabs.as_ref().is_some_and(|t| t.selected == tab) {
                    Response::redraw()
                } else {
                    Response::act(FlyoutAction::SelectTab(tab))
                }
            }
            NodeKey::Tile(device) => Response::act(FlyoutAction::SelectDevice(device)),
            NodeKey::Header => {
                let expanded = self.content.apps.as_ref().is_some_and(|a| a.expanded);
                Response::act(FlyoutAction::SetAppsExpanded(!expanded))
            }
            NodeKey::Mute(target) => Response::act(FlyoutAction::ToggleMute(target)),
            NodeKey::Slider(_) => Response::default(),
            NodeKey::SoundSettings => Response::act(FlyoutAction::OpenSoundSettings),
            NodeKey::Settings => Response::act(FlyoutAction::OpenSettings),
            NodeKey::UpdateAction => Response::act(FlyoutAction::InstallUpdate),
            NodeKey::UpdateClose => Response::act(FlyoutAction::DismissUpdate),
        }
    }

    // ----- Keyboard (FR-024) -----

    /// The keyboard stops, in Tab order. The tabs and the tiles are one stop each (the
    /// arrows move inside them), entered at the selected tab or the current tile.
    fn stops(&self) -> Vec<NodeKey> {
        let mut stops = Vec::new();
        if let Some(tabs) = &self.content.tabs {
            stops.push(NodeKey::Tab(tabs.selected));
        }
        if let Some(tile) = self.entry_tile() {
            stops.push(NodeKey::Tile(tile));
        }
        for node in &self.layout.nodes {
            match node.key {
                NodeKey::Tab(_) | NodeKey::Tile(_) => {}
                key if self.enabled(key) => stops.push(key),
                _ => {}
            }
        }
        stops
    }

    fn entry_tile(&self) -> Option<ItemKey> {
        let tiles = &self.content.devices.tiles;
        let remembered = self.last_tile.filter(|k| tiles.iter().any(|t| t.key == *k));
        remembered
            .or_else(|| tiles.iter().find(|t| t.current).map(|t| t.key))
            .or_else(|| tiles.first().map(|t| t.key))
    }

    /// The stop a control belongs to.
    fn stop_of(&self, key: NodeKey) -> NodeKey {
        match key {
            NodeKey::Tab(_) => self.content.tabs.as_ref().map_or(key, |t| NodeKey::Tab(t.selected)),
            NodeKey::Tile(_) => self.entry_tile().map_or(key, NodeKey::Tile),
            other => other,
        }
    }

    fn set_focus(&mut self, key: NodeKey, visible: bool) -> bool {
        if let NodeKey::Tile(tile) = key {
            self.last_tile = Some(tile);
        }
        let moved = self.focus != Some(key);
        self.focus = Some(key);
        self.focus_visible = visible;
        if visible {
            self.scroll_into_view(key);
        }
        moved
    }

    fn scroll_into_view(&mut self, key: NodeKey) {
        let Some(node) = self.layout.node(key) else { return };
        if node.block != Block::Middle || !self.scrolls() {
            return;
        }
        let visible = self.middle_visible();
        let (top, bottom) = (node.rect.y - 8.0, node.rect.bottom() + 8.0);
        if top < self.scroll {
            self.scroll = top.max(0.0);
        } else if bottom > self.scroll + visible {
            self.scroll = (bottom - visible).min(self.scroll_range());
        }
    }

    /// A key was pressed.
    pub fn key(&mut self, key: FlyoutKey) -> Response {
        if let FlyoutKey::Tab { back } = key {
            return self.tab(back);
        }
        let Some(focus) = self.focus.filter(|f| self.layout.node(*f).is_some()) else {
            // Nothing focused yet: any navigation key enters at the first stop.
            return match key {
                FlyoutKey::Activate => Response::default(),
                _ => self.tab(false),
            };
        };
        let was_visible = self.focus_visible;
        self.focus_visible = true;
        let mut response = match (focus, key) {
            (_, FlyoutKey::Activate) => self.activate(focus),
            (NodeKey::Slider(target), _) => self.slider_key(target, key),
            (NodeKey::Tab(_), FlyoutKey::Left | FlyoutKey::Right) => self.switch_tab(),
            (NodeKey::Tile(tile), _) => self.tile_key(tile, key),
            (_, FlyoutKey::Left | FlyoutKey::Up) => self.move_stop(true),
            (_, FlyoutKey::Right | FlyoutKey::Down) => self.move_stop(false),
            _ => Response::default(),
        };
        if !was_visible {
            response.redraw = true;
        }
        response
    }

    fn tab(&mut self, back: bool) -> Response {
        let stops = self.stops();
        if stops.is_empty() {
            return Response::default();
        }
        let current = self.focus.map(|f| self.stop_of(f));
        let position = current.and_then(|c| stops.iter().position(|s| *s == c));
        let next = match (position, back) {
            (None, false) => 0,
            (None, true) => stops.len() - 1,
            (Some(p), false) => (p + 1) % stops.len(),
            (Some(p), true) => (p + stops.len() - 1) % stops.len(),
        };
        self.set_focus(stops[next], true);
        Response { redraw: true, focus_moved: true, ..Response::default() }
    }

    fn move_stop(&mut self, back: bool) -> Response {
        let stops = self.stops();
        let Some(current) = self.focus.map(|f| self.stop_of(f)) else {
            return Response::default();
        };
        let Some(position) = stops.iter().position(|s| *s == current) else {
            return Response::default();
        };
        let next = if back { position.checked_sub(1) } else { Some(position + 1) };
        match next.and_then(|n| stops.get(n)) {
            Some(&key) => {
                self.set_focus(key, true);
                Response { redraw: true, focus_moved: true, ..Response::default() }
            }
            None => Response::redraw(),
        }
    }

    fn switch_tab(&mut self) -> Response {
        let Some(tabs) = &self.content.tabs else { return Response::default() };
        let other = match tabs.selected {
            FlyoutTab::Output => FlyoutTab::Input,
            FlyoutTab::Input => FlyoutTab::Output,
        };
        self.set_focus(NodeKey::Tab(other), true);
        let mut response = Response::act(FlyoutAction::SelectTab(other));
        response.focus_moved = true;
        response
    }

    fn slider_key(&mut self, target: SliderTarget, key: FlyoutKey) -> Response {
        let Some(volume) = self.content.control(target).map(|c| i32::from(c.volume)) else {
            return Response::default();
        };
        let value = match key {
            FlyoutKey::Left | FlyoutKey::Down => volume - 1,
            FlyoutKey::Right | FlyoutKey::Up => volume + 1,
            FlyoutKey::PageUp => volume + 10,
            FlyoutKey::PageDown => volume - 10,
            FlyoutKey::Home => 0,
            FlyoutKey::End => 100,
            FlyoutKey::Tab { .. } | FlyoutKey::Activate => return Response::default(),
        };
        let mut response = self.set_volume(target, value.clamp(0, 100) as u8);
        response.redraw = true;
        response
    }

    /// Arrows among the tiles: left and right in reading order, up and down to the
    /// nearest tile of the row above or below.
    fn tile_key(&mut self, tile: ItemKey, key: FlyoutKey) -> Response {
        let tiles: Vec<(ItemKey, Rect)> = self
            .content
            .devices
            .tiles
            .iter()
            .filter_map(|t| self.layout.node(NodeKey::Tile(t.key)).map(|n| (t.key, n.rect)))
            .collect();
        let Some(index) = tiles.iter().position(|(k, _)| *k == tile) else {
            return Response::default();
        };
        let rect = tiles[index].1;
        let centre = rect.x + rect.width / 2.0;
        let target = match key {
            FlyoutKey::Left => index.checked_sub(1),
            FlyoutKey::Right => Some(index + 1).filter(|&i| i < tiles.len()),
            FlyoutKey::Up | FlyoutKey::Down => {
                let up = key == FlyoutKey::Up;
                let row_y = tiles
                    .iter()
                    .map(|(_, r)| r.y)
                    .filter(|&y| if up { y < rect.y - 0.5 } else { y > rect.y + 0.5 })
                    .fold(None, |best: Option<f32>, y| match best {
                        None => Some(y),
                        Some(b) if up => Some(b.max(y)),
                        Some(b) => Some(b.min(y)),
                    });
                row_y.and_then(|y| {
                    tiles
                        .iter()
                        .enumerate()
                        .filter(|(_, (_, r))| (r.y - y).abs() < 0.5)
                        .min_by(|(_, (_, a)), (_, (_, b))| {
                            let da = (a.x + a.width / 2.0 - centre).abs();
                            let db = (b.x + b.width / 2.0 - centre).abs();
                            da.total_cmp(&db)
                        })
                        .map(|(i, _)| i)
                })
            }
            FlyoutKey::Home => Some(0),
            FlyoutKey::End => Some(tiles.len() - 1),
            _ => None,
        };
        match target {
            Some(i) if i != index => {
                self.set_focus(NodeKey::Tile(tiles[i].0), true);
                Response { redraw: true, focus_moved: true, ..Response::default() }
            }
            _ => Response::redraw(),
        }
    }

    // ----- Tooltips -----

    /// The tooltip of the control under the pointer, with that control's rectangle (panel
    /// coordinates): the full name of a cut tile label, the calls marker, icon buttons.
    pub fn tooltip(&self) -> Option<(String, Rect)> {
        if self.drag.is_some() || self.pressed.is_some() {
            return None;
        }
        let key = self.hover?;
        let rect = self.node_rect(key)?;
        let text = match key {
            NodeKey::Tile(tile) => {
                let index = self.content.devices.tiles.iter().position(|t| t.key == tile)?;
                let device = &self.content.devices.tiles[index];
                let truncated = self.layout.tiles.get(index).is_some_and(|g| g.truncated);
                match (truncated, &device.calls) {
                    (true, Some(calls)) => format!("{}\n{calls}", device.name),
                    (true, None) => device.name.clone(),
                    (false, Some(calls)) => calls.clone(),
                    (false, None) => return None,
                }
            }
            NodeKey::Mute(target) => self.content.control(target)?.mute_tooltip.clone(),
            NodeKey::Settings => self.content.footer.settings.clone(),
            NodeKey::UpdateClose => self.content.update.as_ref()?.close.clone()?,
            _ => return None,
        };
        (!text.is_empty()).then_some((text, rect))
    }

    // ----- Painting -----

    /// Paints the panel at its shown height (background included).
    pub fn paint(&self, canvas: &mut Canvas<'_>, theme: &Theme) {
        paint::paint(self, canvas, theme);
    }

    // ----- Accessibility (FR-024) -----

    /// The panel described for screen readers (bounds in DIPs, panel coordinates).
    pub fn access_tree(&self) -> AccessNode {
        let size = Rect::new(0.0, 0.0, self.layout.width, self.shown_height);
        let mut root = AccessNode::new(access_id(None), AccessRole::Pane, &self.content.name, size);
        let rect = |key| self.node_rect(key).unwrap_or_default();
        let focusable = |node: &mut AccessNode, key: NodeKey| {
            node.focusable = self.enabled(key);
            node.enabled = self.enabled(key);
        };
        if let Some(tabs) = &self.content.tabs {
            let bar = Rect::new(
                rect(NodeKey::Tab(FlyoutTab::Output)).x,
                rect(NodeKey::Tab(FlyoutTab::Output)).y,
                rect(NodeKey::Tab(FlyoutTab::Input)).right()
                    - rect(NodeKey::Tab(FlyoutTab::Output)).x,
                rect(NodeKey::Tab(FlyoutTab::Output)).height,
            );
            let mut list = AccessNode::new(id_of("tabs"), AccessRole::TabList, &tabs.name, bar);
            for (tab, label) in [(FlyoutTab::Output, &tabs.output), (FlyoutTab::Input, &tabs.input)]
            {
                let key = NodeKey::Tab(tab);
                let mut node =
                    AccessNode::new(access_id(Some(key)), AccessRole::Tab, label, rect(key));
                node.focusable = true;
                node.selected = Some(tabs.selected == tab);
                list.children.push(node);
            }
            root.children.push(list);
        }
        if self.content.devices.tiles.is_empty() {
            let empty = self.layout.empty.unwrap_or_default();
            root.children.push(AccessNode::new(
                id_of("empty"),
                AccessRole::Text,
                &self.content.devices.empty,
                empty,
            ));
        } else {
            let first = self
                .content
                .devices
                .tiles
                .first()
                .map_or(Rect::default(), |t| rect(NodeKey::Tile(t.key)));
            let last = self
                .content
                .devices
                .tiles
                .last()
                .map_or(Rect::default(), |t| rect(NodeKey::Tile(t.key)));
            let bounds = Rect::new(
                first.x,
                first.y,
                self.layout.width - 2.0 * first.x,
                last.bottom() - first.y,
            );
            let mut list = AccessNode::new(
                id_of("devices"),
                AccessRole::List,
                &self.content.devices.name,
                bounds,
            );
            for device in &self.content.devices.tiles {
                let key = NodeKey::Tile(device.key);
                let mut node = AccessNode::new(
                    access_id(Some(key)),
                    AccessRole::ListItem,
                    &device.name,
                    rect(key),
                );
                node.focusable = true;
                node.selected = Some(device.current);
                node.help = device.calls.clone();
                list.children.push(node);
            }
            root.children.push(list);
        }
        if let Some(apps) = &self.content.apps {
            let key = NodeKey::Header;
            let mut header =
                AccessNode::new(access_id(Some(key)), AccessRole::Expander, &apps.title, rect(key));
            header.focusable = true;
            header.expanded = Some(apps.expanded);
            root.children.push(header);
            for row in self.content.visible_rows() {
                root.children.push(self.access_row(row, &rect, &focusable));
            }
        }
        root.children.push(self.access_row(&self.content.main, &rect, &focusable));
        if let (Some(update), Some(text)) = (&self.content.update, self.layout.update_text) {
            let bounds = self.placed(Block::Bottom, text);
            root.children.push(AccessNode::new(
                id_of("update"),
                AccessRole::Text,
                &update.text,
                bounds,
            ));
            for (key, label) in [
                (NodeKey::UpdateAction, update.action.as_ref()),
                (NodeKey::UpdateClose, update.close.as_ref()),
            ] {
                if let Some(label) = label {
                    let mut button =
                        AccessNode::new(access_id(Some(key)), AccessRole::Button, label, rect(key));
                    button.focusable = true;
                    root.children.push(button);
                }
            }
        }
        let mut link = AccessNode::new(
            access_id(Some(NodeKey::SoundSettings)),
            AccessRole::Button,
            &self.content.footer.sound_settings,
            rect(NodeKey::SoundSettings),
        );
        link.focusable = true;
        root.children.push(link);
        let mut settings = AccessNode::new(
            access_id(Some(NodeKey::Settings)),
            AccessRole::Button,
            &self.content.footer.settings,
            rect(NodeKey::Settings),
        );
        focusable(&mut settings, NodeKey::Settings);
        root.children.push(settings);
        root
    }

    fn access_row(
        &self,
        row: &VolumeControl,
        rect: &dyn Fn(NodeKey) -> Rect,
        focusable: &dyn Fn(&mut AccessNode, NodeKey),
    ) -> AccessNode {
        let mute_key = NodeKey::Mute(row.target);
        let slider_key = NodeKey::Slider(row.target);
        let (m, s) = (rect(mute_key), rect(slider_key));
        let bounds =
            Rect::new(m.x, m.y, crate::tokens::flyout::WIDTH - 2.0 * m.x, m.height.max(s.height));
        let mut group =
            AccessNode::new(id_of_target(row.target), AccessRole::Group, &row.label, bounds);
        group.enabled = row.enabled;
        let mut mute =
            AccessNode::new(access_id(Some(mute_key)), AccessRole::ToggleButton, &row.mute_name, m);
        focusable(&mut mute, mute_key);
        mute.toggled = Some(row.muted);
        mute.help = (!row.mute_tooltip.is_empty()).then(|| row.mute_tooltip.clone());
        let mut slider =
            AccessNode::new(access_id(Some(slider_key)), AccessRole::Slider, &row.slider_name, s);
        focusable(&mut slider, slider_key);
        let value = match self.drag {
            Some(drag) if drag.target == row.target => drag.value,
            _ => row.volume,
        };
        slider.range = Some(AccessRange {
            value: f64::from(value),
            min: 0.0,
            max: 100.0,
            small_change: 1.0,
            large_change: 10.0,
        });
        group.children.push(mute);
        group.children.push(slider);
        group
    }

    /// The screen-reader identifier of the focused control.
    pub fn focused_access_id(&self) -> Option<u64> {
        self.focus.filter(|f| self.layout.node(*f).is_some()).map(|f| access_id(Some(f)))
    }

    /// The screen-reader identifier of the control whose state an action changes (to
    /// tell screen readers once the application applied it).
    pub fn action_access_id(action: FlyoutAction) -> Option<u64> {
        let key = match action {
            FlyoutAction::SelectTab(tab) => NodeKey::Tab(tab),
            FlyoutAction::SelectDevice(device) => NodeKey::Tile(device),
            FlyoutAction::SetVolume(target, _) => NodeKey::Slider(target),
            FlyoutAction::ToggleMute(target) => NodeKey::Mute(target),
            FlyoutAction::SetAppsExpanded(_) => NodeKey::Header,
            FlyoutAction::OpenSoundSettings
            | FlyoutAction::OpenSettings
            | FlyoutAction::InstallUpdate
            | FlyoutAction::DismissUpdate => return None,
        };
        Some(access_id(Some(key)))
    }

    /// The screen-reader identifier of a volume slider (to report its value changes).
    pub fn slider_access_id(target: SliderTarget) -> u64 {
        access_id(Some(NodeKey::Slider(target)))
    }

    /// A request from an assistive tool.
    pub fn access_action(&mut self, id: u64, action: AccessAction) -> Response {
        let Some(key) = self.layout.nodes.iter().map(|n| n.key).find(|k| access_id(Some(*k)) == id)
        else {
            return Response::default();
        };
        if !self.enabled(key) {
            return Response::default();
        }
        match (key, action) {
            (_, AccessAction::Focus) => {
                self.set_focus(key, true);
                Response { redraw: true, focus_moved: true, ..Response::default() }
            }
            (NodeKey::Slider(target), AccessAction::SetValue(value)) => {
                self.set_volume(target, value.round().clamp(0.0, 100.0) as u8)
            }
            (NodeKey::Header, AccessAction::Expand | AccessAction::Collapse) => {
                let expand = action == AccessAction::Expand;
                let expanded = self.content.apps.as_ref().is_some_and(|a| a.expanded);
                if expand == expanded {
                    Response::default()
                } else {
                    Response::act(FlyoutAction::SetAppsExpanded(expand))
                }
            }
            (NodeKey::Slider(_), _) => Response::default(),
            (_, AccessAction::Invoke | AccessAction::Toggle | AccessAction::Select) => {
                self.activate(key)
            }
            _ => Response::default(),
        }
    }
}

/// The value (percent) at horizontal position `x` of a slider whose hit area is `rect`.
pub(crate) fn value_at(rect: Rect, x: f32) -> u8 {
    let travel = (rect.width - THUMB).max(1.0);
    let fraction = ((x - rect.x - THUMB / 2.0) / travel).clamp(0.0, 1.0);
    (fraction * 100.0).round() as u8
}

/// Centre of the thumb of a slider at `value`.
pub(crate) fn thumb_x(rect: Rect, value: u8) -> f32 {
    rect.x + THUMB / 2.0 + (rect.width - THUMB).max(0.0) * f32::from(value.min(100)) / 100.0
}

fn hash_of(value: impl Hash) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    // Never 0: UI Automation identifiers of the root are derived from the window.
    hasher.finish() | 1
}

fn access_id(key: Option<NodeKey>) -> u64 {
    match key {
        None => 1,
        Some(key) => hash_of(("node", key)),
    }
}

fn id_of(name: &str) -> u64 {
    hash_of(("group", name))
}

fn id_of_target(target: SliderTarget) -> u64 {
    hash_of(("row", target))
}

#[cfg(test)]
mod tests;
