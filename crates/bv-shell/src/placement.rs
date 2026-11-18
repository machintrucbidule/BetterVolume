//! Where popups go (FR-020): pure geometry in physical pixels, tested without a screen.

/// A rectangle in physical screen pixels (right and bottom excluded).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ScreenRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl ScreenRect {
    pub fn new(left: i32, top: i32, right: i32, bottom: i32) -> ScreenRect {
        ScreenRect { left, top, right, bottom }
    }

    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }

    fn intersects(&self, other: &ScreenRect) -> bool {
        self.left < other.right
            && other.left < self.right
            && self.top < other.bottom
            && other.top < self.bottom
    }
}

/// The screen edge the taskbar is docked to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Edge {
    Left,
    Top,
    Right,
    #[default]
    Bottom,
}

/// The edge a taskbar rectangle is docked to on its monitor: the side it touches along
/// its long dimension. Anything unclear counts as the Windows 11 default, the bottom.
pub(crate) fn taskbar_edge(taskbar: &ScreenRect, monitor: &ScreenRect) -> Edge {
    let horizontal = taskbar.width() >= taskbar.height();
    if horizontal {
        if taskbar.top <= monitor.top && taskbar.bottom < monitor.bottom {
            Edge::Top
        } else {
            Edge::Bottom
        }
    } else if taskbar.left <= monitor.left {
        Edge::Left
    } else {
        Edge::Right
    }
}

/// The area popups may use on a monitor: the work area, minus the taskbar when it hides
/// automatically (the work area then covers it, but popups must sit beside it).
pub(crate) fn usable_area(
    work: &ScreenRect,
    taskbar: Option<&ScreenRect>,
    edge: Edge,
) -> ScreenRect {
    let mut area = *work;
    if let Some(bar) = taskbar.filter(|bar| bar.intersects(work)) {
        match edge {
            Edge::Bottom => area.bottom = area.bottom.min(bar.top),
            Edge::Top => area.top = area.top.max(bar.bottom),
            Edge::Left => area.left = area.left.max(bar.right),
            Edge::Right => area.right = area.right.min(bar.left),
        }
    }
    area
}

/// Top-left corner of the flyout: in the corner of the usable area next to the tray (the
/// end of the taskbar holding the clock), `margin` pixels from both edges. `rtl`: the
/// tray is at the left end of a horizontal taskbar (right-to-left Windows).
pub(crate) fn flyout_origin(
    size: (i32, i32),
    area: &ScreenRect,
    edge: Edge,
    margin: i32,
    rtl: bool,
) -> (i32, i32) {
    let (w, h) = size;
    let right_x = area.right - margin - w;
    let left_x = area.left + margin;
    let bottom_y = area.bottom - margin - h;
    let top_y = area.top + margin;
    let x = match edge {
        Edge::Left => left_x,
        Edge::Right => right_x,
        Edge::Top | Edge::Bottom if rtl => left_x,
        Edge::Top | Edge::Bottom => right_x,
    };
    // Vertical taskbars hold the tray at their bottom end.
    let y = if edge == Edge::Top { top_y } else { bottom_y };
    (x.max(area.left), y.max(area.top))
}

/// Top-left corner of the context menu opened at `anchor` (the pointer or the icon),
/// always kept inside the usable area. Along the taskbar it ends at the anchor when it
/// fits on that side, as Windows tray menus do (otherwise it starts there). Across the
/// taskbar: opened from the taskbar itself, it stands `gap` pixels away from it; opened
/// from inside the area (the hidden-icons panel above the taskbar), it ends at the anchor.
pub(crate) fn menu_origin(
    anchor: (i32, i32),
    size: (i32, i32),
    area: &ScreenRect,
    edge: Edge,
    gap: i32,
) -> (i32, i32) {
    let (w, h) = size;
    let (ax, ay) = anchor;
    let along = |a: i32, extent: i32, low: i32| if a - extent >= low { a - extent } else { a };
    let (x, y) = match edge {
        Edge::Bottom => (along(ax, w, area.left), (area.bottom - gap).min(ay) - h),
        Edge::Top => (along(ax, w, area.left), (area.top + gap).max(ay)),
        Edge::Left => ((area.left + gap).max(ax), along(ay, h, area.top)),
        Edge::Right => ((area.right - gap).min(ax) - w, along(ay, h, area.top)),
    };
    (clamp(x, area.left, area.right - w), clamp(y, area.top, area.bottom - h))
}

/// Top-left corner of the bubble: centred along the usable area, `gap` pixels
/// from a horizontal taskbar, where the Windows 11 volume indicator appears. With a
/// vertical taskbar it stands at the bottom centre of the area, the same gap from the
/// monitor's bottom edge.
pub(crate) fn bubble_origin(
    size: (i32, i32),
    area: &ScreenRect,
    edge: Edge,
    gap: i32,
) -> (i32, i32) {
    let (w, h) = size;
    let x = area.left + (area.width() - w) / 2;
    let y = match edge {
        Edge::Top => area.top + gap,
        Edge::Bottom | Edge::Left | Edge::Right => area.bottom - gap - h,
    };
    (x.max(area.left), y.max(area.top))
}

/// Whether a popup at `origin` stands next to the taskbar (within `gap` of its inner edge).
/// Only then can it rise from behind it; a menu opened higher (from the hidden-icons
/// panel) would otherwise cross half the screen.
pub(crate) fn touches_taskbar(
    origin: (i32, i32),
    size: (i32, i32),
    area: &ScreenRect,
    edge: Edge,
    gap: i32,
) -> bool {
    match edge {
        Edge::Bottom => origin.1 + size.1 >= area.bottom - gap,
        Edge::Top => origin.1 <= area.top + gap,
        Edge::Left => origin.0 <= area.left + gap,
        Edge::Right => origin.0 + size.0 >= area.right - gap,
    }
}

/// Where a popup starts its entrance: moved towards the taskbar until it is entirely past
/// the monitor's edge on the taskbar side. Every frame of the move is cut to the monitor
/// ([`visible_part`]), so the popup rises from behind the taskbar and never shows on a
/// neighbouring monitor.
pub(crate) fn hidden_origin(
    origin: (i32, i32),
    size: (i32, i32),
    monitor: &ScreenRect,
    edge: Edge,
) -> (i32, i32) {
    let (x, y) = origin;
    let (w, h) = size;
    match edge {
        Edge::Bottom => (x, monitor.bottom.max(y)),
        Edge::Top => (x, monitor.top.min(y + h) - h),
        Edge::Left => (monitor.left.min(x + w) - w, y),
        Edge::Right => (monitor.right.max(x), y),
    }
}

/// The part of a popup at `origin` (of `size`) that lies on its monitor: the window
/// rectangle to use, and the offset of the content inside it (zero or negative, when the
/// part cut off is at the top or the left). An empty part has a zero size.
pub(crate) fn visible_part(
    origin: (i32, i32),
    size: (i32, i32),
    monitor: &ScreenRect,
) -> (ScreenRect, (i32, i32)) {
    let left = origin.0.max(monitor.left);
    let top = origin.1.max(monitor.top);
    let right = (origin.0 + size.0).min(monitor.right).max(left);
    let bottom = (origin.1 + size.1).min(monitor.bottom).max(top);
    (ScreenRect::new(left, top, right, bottom), (origin.0 - left, origin.1 - top))
}

/// The position at `progress` (0 = from, 1 = to) of a move, rounded to whole pixels.
pub(crate) fn lerp(from: (i32, i32), to: (i32, i32), progress: f32) -> (i32, i32) {
    let p = progress.clamp(0.0, 1.0);
    let mix = |a: i32, b: i32| (a as f32 + (b - a) as f32 * p).round() as i32;
    (mix(from.0, to.0), mix(from.1, to.1))
}

/// `value` brought within `low..=high`; `low` wins when the range is empty (a popup larger
/// than the area stays aligned on its top-left).
fn clamp(value: i32, low: i32, high: i32) -> i32 {
    value.min(high).max(low)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1920 × 1080 monitor, 48 px taskbar at the bottom.
    fn monitor() -> ScreenRect {
        ScreenRect::new(0, 0, 1920, 1080)
    }

    fn bottom_bar() -> ScreenRect {
        ScreenRect::new(0, 1032, 1920, 1080)
    }

    fn work_above_bar() -> ScreenRect {
        ScreenRect::new(0, 0, 1920, 1032)
    }

    #[test]
    fn taskbar_edges_are_recognised() {
        let m = monitor();
        assert_eq!(taskbar_edge(&bottom_bar(), &m), Edge::Bottom);
        assert_eq!(taskbar_edge(&ScreenRect::new(0, 0, 1920, 48), &m), Edge::Top);
        assert_eq!(taskbar_edge(&ScreenRect::new(0, 0, 62, 1080), &m), Edge::Left);
        assert_eq!(taskbar_edge(&ScreenRect::new(1858, 0, 1920, 1080), &m), Edge::Right);
        // A monitor left of the primary one has negative coordinates.
        let left_monitor = ScreenRect::new(-1920, 0, 0, 1080);
        let bar = ScreenRect::new(-1920, 1032, 0, 1080);
        assert_eq!(taskbar_edge(&bar, &left_monitor), Edge::Bottom);
    }

    #[test]
    fn auto_hidden_taskbar_is_cut_out_of_the_area() {
        // Auto-hide: the work area is the whole monitor.
        let area = usable_area(&monitor(), Some(&bottom_bar()), Edge::Bottom);
        assert_eq!(area, work_above_bar());
        let top_bar = ScreenRect::new(0, 0, 1920, 48);
        assert_eq!(usable_area(&monitor(), Some(&top_bar), Edge::Top).top, 48);
        // Normal taskbar: already outside the work area, nothing changes.
        assert_eq!(
            usable_area(&work_above_bar(), Some(&bottom_bar()), Edge::Bottom),
            work_above_bar()
        );
        // Taskbar on another monitor: nothing changes.
        let other = ScreenRect::new(1920, 1032, 3840, 1080);
        assert_eq!(usable_area(&monitor(), Some(&other), Edge::Bottom), monitor());
    }

    #[test]
    fn bubble_sits_bottom_centre_above_the_taskbar() {
        // Measured on the Windows indicator at 225 %: 432 x 108, 31 px above the taskbar,
        // centred on a 3840-wide monitor.
        let area = ScreenRect::new(0, 0, 3840, 2052);
        assert_eq!(bubble_origin((432, 108), &area, Edge::Bottom, 31), (1704, 1913));
        let top = ScreenRect::new(0, 48, 1920, 1080);
        assert_eq!(bubble_origin((192, 48), &top, Edge::Top, 14), (864, 62));
        let left = ScreenRect::new(62, 0, 1920, 1080);
        assert_eq!(bubble_origin((192, 48), &left, Edge::Left, 14), (895, 1018));
        // A monitor left of the primary one; a bubble wider than the area stays inside.
        let negative = ScreenRect::new(-1920, 0, 0, 1032);
        assert_eq!(bubble_origin((192, 48), &negative, Edge::Bottom, 14), (-1056, 970));
        let narrow = ScreenRect::new(0, 0, 100, 1032);
        assert_eq!(bubble_origin((192, 48), &narrow, Edge::Bottom, 14).0, 0);
    }

    #[test]
    fn flyout_sits_in_the_tray_corner() {
        let area = work_above_bar();
        assert_eq!(flyout_origin((360, 200), &area, Edge::Bottom, 12, false), (1548, 820));
        assert_eq!(flyout_origin((360, 200), &area, Edge::Bottom, 12, true), (12, 820));
        let top = ScreenRect::new(0, 48, 1920, 1080);
        assert_eq!(flyout_origin((360, 200), &top, Edge::Top, 12, false), (1548, 60));
        let left = ScreenRect::new(62, 0, 1920, 1080);
        assert_eq!(flyout_origin((360, 200), &left, Edge::Left, 12, false), (74, 868));
        let right = ScreenRect::new(0, 0, 1858, 1080);
        assert_eq!(flyout_origin((360, 200), &right, Edge::Right, 12, false), (1486, 868));
    }

    #[test]
    fn flyout_never_leaves_a_tiny_screen() {
        let area = ScreenRect::new(0, 0, 300, 150);
        assert_eq!(flyout_origin((360, 200), &area, Edge::Bottom, 12, false), (0, 0));
    }

    #[test]
    fn menu_opens_above_a_bottom_taskbar_ending_at_the_pointer() {
        let area = work_above_bar();
        assert_eq!(menu_origin((1700, 1050), (260, 230), &area, Edge::Bottom, 8), (1440, 794));
        // Too close to the left edge: starts at the pointer instead.
        assert_eq!(menu_origin((100, 1050), (260, 230), &area, Edge::Bottom, 8), (100, 794));
        // Never past the right edge.
        assert_eq!(menu_origin((1919, 1050), (260, 230), &area, Edge::Bottom, 8), (1659, 794));
    }

    #[test]
    fn menu_follows_other_taskbar_positions() {
        let top = ScreenRect::new(0, 48, 1920, 1080);
        assert_eq!(menu_origin((1700, 20), (260, 230), &top, Edge::Top, 8), (1440, 56));
        let left = ScreenRect::new(62, 0, 1920, 1080);
        assert_eq!(menu_origin((30, 1000), (260, 230), &left, Edge::Left, 8), (70, 770));
        let right = ScreenRect::new(0, 0, 1858, 1080);
        assert_eq!(menu_origin((1890, 100), (260, 230), &right, Edge::Right, 8), (1590, 100));
    }

    #[test]
    fn only_popups_next_to_the_taskbar_rise_from_it() {
        let area = work_above_bar();
        let from_taskbar = menu_origin((1700, 1050), (260, 230), &area, Edge::Bottom, 8);
        assert!(touches_taskbar(from_taskbar, (260, 230), &area, Edge::Bottom, 8));
        let from_panel = menu_origin((1600, 900), (260, 230), &area, Edge::Bottom, 8);
        assert!(!touches_taskbar(from_panel, (260, 230), &area, Edge::Bottom, 8));
        let top = ScreenRect::new(0, 48, 1920, 1080);
        assert!(touches_taskbar((1440, 56), (260, 230), &top, Edge::Top, 8));
        let left = ScreenRect::new(62, 0, 1920, 1080);
        assert!(touches_taskbar((70, 770), (260, 230), &left, Edge::Left, 8));
        assert!(!touches_taskbar((400, 770), (260, 230), &left, Edge::Left, 8));
    }

    #[test]
    fn menu_from_the_hidden_icons_panel_ends_at_the_pointer() {
        // The pointer is inside the work area (the panel sits above the taskbar).
        let area = work_above_bar();
        assert_eq!(menu_origin((1600, 900), (260, 230), &area, Edge::Bottom, 8), (1340, 670));
        // Too close to the top: kept inside the area.
        assert_eq!(menu_origin((1600, 100), (260, 230), &area, Edge::Bottom, 8), (1340, 0));
    }

    #[test]
    fn entrance_starts_past_the_monitor_edge() {
        let m = monitor();
        assert_eq!(hidden_origin((1548, 820), (360, 200), &m, Edge::Bottom), (1548, 1080));
        assert_eq!(hidden_origin((1548, 60), (360, 200), &m, Edge::Top), (1548, -200));
        assert_eq!(hidden_origin((74, 868), (360, 200), &m, Edge::Left), (-360, 868));
        assert_eq!(hidden_origin((1486, 868), (360, 200), &m, Edge::Right), (1920, 868));
    }

    #[test]
    fn frames_are_cut_to_the_monitor() {
        let m = monitor();
        // Fully on the monitor: unchanged, no content offset.
        let (r, offset) = visible_part((1548, 820), (360, 200), &m);
        assert_eq!((r, offset), (ScreenRect::new(1548, 820, 1908, 1020), (0, 0)));
        // Rising from the bottom edge: the top part shows, the content stays in place.
        let (r, offset) = visible_part((1548, 1000), (360, 200), &m);
        assert_eq!((r, offset), (ScreenRect::new(1548, 1000, 1908, 1080), (0, 0)));
        // Coming down from the top edge: the bottom part shows, content shifted up.
        let (r, offset) = visible_part((1548, -150), (360, 200), &m);
        assert_eq!((r, offset), (ScreenRect::new(1548, 0, 1908, 50), (0, -150)));
        // From the left edge.
        let (r, offset) = visible_part((-300, 868), (360, 200), &m);
        assert_eq!((r, offset), (ScreenRect::new(0, 868, 60, 1068), (-300, 0)));
        // At the start: nothing on the monitor, an empty window.
        let (r, _) = visible_part((1548, 1080), (360, 200), &m);
        assert_eq!((r.width(), r.height()), (360, 0));
        // A monitor below the primary one is never reached.
        let (r, _) = visible_part((1548, 1050), (360, 200), &m);
        assert!(r.bottom <= m.bottom);
    }

    #[test]
    fn lerp_rounds_and_clamps() {
        assert_eq!(lerp((0, 100), (0, 0), 0.0), (0, 100));
        assert_eq!(lerp((0, 100), (0, 0), 0.5), (0, 50));
        assert_eq!(lerp((0, 100), (0, 0), 1.0), (0, 0));
        assert_eq!(lerp((0, 100), (0, 0), 2.0), (0, 0));
        assert_eq!(lerp((10, 0), (13, 0), 0.5), (12, 0));
    }
}
