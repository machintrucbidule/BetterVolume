//! Layout of the flyout (FR-023): three blocks stacked vertically.
//!
//! - top: tab bar and device tiles, anchored to the top edge;
//! - middle: the application list, below the top block; the only part that scrolls when
//!   the panel cannot fit on the screen, and the part that is cut or revealed while the
//!   height changes;
//! - bottom: master volume and footer, anchored to the bottom edge.
//!
//! Every rectangle is in DIPs, relative to the top of its block.

use super::content::{FlyoutContent, FlyoutTab, ItemKey, SliderTarget};
use crate::tokens::flyout::{
    EMPTY_HEIGHT, FOOTER_HEIGHT, FOOTER_INSET, PADDING_BOTTOM, PADDING_TOP, PADDING_X, SECTION_GAP,
    TABS_GAP, WIDTH,
};
use crate::tokens::typography::{TextStyle, BODY, BODY_STRONG, CAPTION, CAPTION_STRONG};
use crate::tokens::{footer, section, tabs, tile, volume_row};
use crate::Rect;

/// Measures text with the real font, at the Windows text scaling.
pub trait TextMeasure {
    /// Width of one line of text.
    fn width(&self, text: &str, style: TextStyle) -> f32;
    /// Number of lines the text takes when wrapped in `width`.
    fn lines(&self, text: &str, style: TextStyle, width: f32) -> u32;
}

/// Which block a node belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Block {
    Top,
    Middle,
    Bottom,
}

/// What a node is; also its identity across content updates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum NodeKey {
    Tab(FlyoutTab),
    Tile(ItemKey),
    Header,
    Mute(SliderTarget),
    Slider(SliderTarget),
    SoundSettings,
    Settings,
    UpdateAction,
    UpdateClose,
}

/// A control placed in a block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Node {
    pub key: NodeKey,
    pub rect: Rect,
    pub block: Block,
}

/// Extra geometry of a tile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TileGeometry {
    /// The label does not fit on its lines (the tooltip then shows the full name).
    pub truncated: bool,
}

/// The laid-out flyout.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Layout {
    pub width: f32,
    pub top_height: f32,
    pub middle_height: f32,
    pub bottom_height: f32,
    pub nodes: Vec<Node>,
    /// Per tile, in content order.
    pub tiles: Vec<TileGeometry>,
    /// Labels above volume rows: (target, rectangle, block).
    pub labels: Vec<(SliderTarget, Rect, Block)>,
    /// Rectangle of the "no device" text, when there is no tile.
    pub empty: Option<Rect>,
    /// Vertical position of the line at the top of the middle block, if any.
    pub middle_divider: Option<f32>,
    /// Vertical position of the line at the top of the bottom block.
    pub bottom_divider: f32,
    /// Footer band, in the bottom block (the update line included, when shown).
    pub footer: Rect,
    /// The text of the update line (its glyph at its left), in the bottom block.
    pub update_text: Option<Rect>,
    /// Width of the two tab labels (output, input).
    pub tab_labels: [f32; 2],
}

impl Layout {
    /// Height needed to show everything.
    pub fn natural_height(&self) -> f32 {
        self.top_height + self.middle_height + self.bottom_height
    }

    pub fn node(&self, key: NodeKey) -> Option<&Node> {
        self.nodes.iter().find(|n| n.key == key)
    }
}

/// How many tiles each row holds: balanced rows of at most three, fuller rows
/// first (4 = 2 + 2, 5 = 3 + 2, 7 = 3 + 2 + 2).
pub fn tile_rows(count: usize) -> Vec<usize> {
    if count == 0 {
        return Vec::new();
    }
    let rows = count.div_ceil(tile::PER_ROW);
    let (base, extra) = (count / rows, count % rows);
    (0..rows).map(|row| base + usize::from(row < extra)).collect()
}

/// Height of a tile at a text scale.
pub(crate) fn tile_height(text_scale: f32) -> f32 {
    (tile::PADDING_TOP
        + tile::ICON_SIZE
        + tile::ICON_GAP
        + CAPTION.line_height * text_scale * tile::LABEL_LINES as f32
        + tile::PADDING_BOTTOM)
        .round()
}

/// The style of a tile label: stronger on the current device.
pub(crate) fn tile_style(current: bool) -> TextStyle {
    if current {
        CAPTION_STRONG
    } else {
        CAPTION
    }
}

/// The style of a tab label.
pub(crate) fn tab_style(selected: bool) -> TextStyle {
    if selected {
        BODY_STRONG
    } else {
        BODY
    }
}

/// Lays the content out.
pub(crate) fn layout(
    content: &FlyoutContent,
    measure: &dyn TextMeasure,
    text_scale: f32,
) -> Layout {
    let inner = WIDTH - 2.0 * PADDING_X;
    let mut nodes = Vec::new();
    let mut tiles = Vec::new();
    let mut labels = Vec::new();

    // Top block: tabs and tiles.
    let mut tab_labels = [0.0; 2];
    let mut y = PADDING_TOP;
    if let Some(bar) = &content.tabs {
        let bar_h = (tabs::HEIGHT - BODY.line_height + BODY.line_height * text_scale).round();
        let bar_rect = Rect::new(PADDING_X, y, inner, bar_h);
        let tab_width = (inner - 3.0 * tabs::PADDING) / 2.0;
        for (index, tab) in [FlyoutTab::Output, FlyoutTab::Input].into_iter().enumerate() {
            let x = bar_rect.x + tabs::PADDING + index as f32 * (tab_width + tabs::PADDING);
            let rect = Rect::new(x, y + tabs::PADDING, tab_width, bar_h - 2.0 * tabs::PADDING);
            nodes.push(Node { key: NodeKey::Tab(tab), rect, block: Block::Top });
        }
        tab_labels = [
            measure.width(&bar.output, tab_style(bar.selected == FlyoutTab::Output)),
            measure.width(&bar.input, tab_style(bar.selected == FlyoutTab::Input)),
        ];
        y += bar_h + TABS_GAP;
    }
    let tile_h = tile_height(text_scale);
    let mut empty = None;
    if content.devices.tiles.is_empty() {
        empty = Some(Rect::new(PADDING_X, y, inner, EMPTY_HEIGHT));
        y += EMPTY_HEIGHT;
    } else {
        let mut index = 0;
        for (row, count) in tile_rows(content.devices.tiles.len()).into_iter().enumerate() {
            if row > 0 {
                y += tile::GAP;
            }
            let width = (inner - (count - 1) as f32 * tile::GAP) / count as f32;
            for column in 0..count {
                let device = &content.devices.tiles[index];
                let x = PADDING_X + column as f32 * (width + tile::GAP);
                let rect = Rect::new(x, y, width, tile_h);
                nodes.push(Node { key: NodeKey::Tile(device.key), rect, block: Block::Top });
                let label_width = width - 2.0 * tile::PADDING_X;
                let lines = measure.lines(&device.name, tile_style(device.current), label_width);
                tiles.push(TileGeometry { truncated: lines > tile::LABEL_LINES });
                index += 1;
            }
            y += tile_h;
        }
    }
    let top_height = y;

    // Middle block: the application list.
    let label_h = (CAPTION.line_height * text_scale).ceil();
    let row_h = label_h + volume_row::LABEL_GAP + volume_row::HEIGHT;
    let mut middle_height = 0.0;
    let mut middle_divider = None;
    if let Some(apps) = &content.apps {
        let mut y = SECTION_GAP;
        middle_divider = Some(y);
        y += SECTION_GAP;
        let header_h = section::HEIGHT.max((CAPTION_STRONG.line_height * text_scale).ceil() + 12.0);
        let header = Rect::new(PADDING_X - 4.0, y, inner + 8.0, header_h);
        nodes.push(Node { key: NodeKey::Header, rect: header, block: Block::Middle });
        y += header_h;
        if apps.expanded {
            y += section::GAP;
            for (index, row) in apps.rows.iter().enumerate() {
                if index > 0 {
                    y += volume_row::ROW_SPACING;
                }
                volume_nodes(&mut nodes, &mut labels, row.target, y, label_h, false, Block::Middle);
                y += row_h;
            }
        }
        middle_height = y;
    }

    // Bottom block: main volume row and footer.
    let mut y = SECTION_GAP;
    let bottom_divider = y;
    y += SECTION_GAP;
    volume_nodes(&mut nodes, &mut labels, SliderTarget::Main, y, label_h, true, Block::Bottom);
    y += row_h + PADDING_BOTTOM;
    let footer_top = y;
    let mut update_text = None;
    if let Some(update) = &content.update {
        // Glyph and text at the left, the button and the close button at the right.
        // The line and its buttons grow with the Windows text size.
        let line_h = (CAPTION_STRONG.line_height * text_scale).ceil();
        let row_h = footer::UPDATE_HEIGHT.max(line_h + 2.0 * footer::UPDATE_GAP);
        let button_h = footer::UPDATE_BUTTON.max(line_h + footer::UPDATE_GAP);
        let button_y = y + (row_h - button_h) / 2.0;
        let mut right = WIDTH - FOOTER_INSET;
        if update.close.is_some() {
            // As wide as the settings button below it: the two glyphs line up.
            right -= footer::BUTTON;
            let close = Rect::new(right, button_y, footer::BUTTON, button_h);
            nodes.push(Node { key: NodeKey::UpdateClose, rect: close, block: Block::Bottom });
            right -= footer::UPDATE_GAP / 2.0;
        }
        if let Some(label) = &update.action {
            let w = (measure.width(label, CAPTION_STRONG) + 2.0 * footer::PADDING_X).ceil();
            right -= w;
            let action = Rect::new(right, button_y, w, button_h);
            nodes.push(Node { key: NodeKey::UpdateAction, rect: action, block: Block::Bottom });
        }
        let x = FOOTER_INSET + footer::PADDING_X + footer::GLYPH + footer::UPDATE_GAP;
        let width = (right - footer::UPDATE_GAP - x).max(0.0);
        update_text = Some(Rect::new(x, y, width, row_h));
        y += row_h;
    }
    let footer_rect = Rect::new(0.0, footer_top, WIDTH, y - footer_top + FOOTER_HEIGHT);
    let button_y = y + (FOOTER_HEIGHT - footer::BUTTON) / 2.0;
    let link_w = measure.width(&content.footer.sound_settings, CAPTION) + 2.0 * footer::PADDING_X;
    let link = Rect::new(FOOTER_INSET, button_y, link_w.ceil(), footer::BUTTON);
    nodes.push(Node { key: NodeKey::SoundSettings, rect: link, block: Block::Bottom });
    let settings =
        Rect::new(WIDTH - FOOTER_INSET - footer::BUTTON, button_y, footer::BUTTON, footer::BUTTON);
    nodes.push(Node { key: NodeKey::Settings, rect: settings, block: Block::Bottom });
    y += FOOTER_HEIGHT;

    Layout {
        width: WIDTH,
        top_height,
        middle_height,
        bottom_height: y,
        nodes,
        tiles,
        labels,
        empty,
        middle_divider,
        bottom_divider,
        footer: footer_rect,
        update_text,
        tab_labels,
    }
}

/// The nodes of one volume row whose label line starts at `y`.
fn volume_nodes(
    nodes: &mut Vec<Node>,
    labels: &mut Vec<(SliderTarget, Rect, Block)>,
    target: SliderTarget,
    y: f32,
    label_h: f32,
    main: bool,
    block: Block,
) {
    let x = PADDING_X;
    let right = WIDTH - PADDING_X;
    let slider_x = x + volume_row::BUTTON + volume_row::SLIDER_GAP;
    let label_x = if main { x + volume_row::MASTER_LABEL_INDENT } else { slider_x };
    labels.push((target, Rect::new(label_x, y, right - label_x, label_h), block));
    let row_y = y + label_h + volume_row::LABEL_GAP;
    let button_y = row_y + (volume_row::HEIGHT - volume_row::BUTTON) / 2.0;
    let mute = Rect::new(x, button_y, volume_row::BUTTON, volume_row::BUTTON);
    nodes.push(Node { key: NodeKey::Mute(target), rect: mute, block });
    let slider_right = right - volume_row::VALUE_WIDTH - volume_row::VALUE_GAP;
    let slider_y = row_y + (volume_row::HEIGHT - crate::tokens::slider::HEIGHT) / 2.0;
    let slider =
        Rect::new(slider_x, slider_y, slider_right - slider_x, crate::tokens::slider::HEIGHT);
    nodes.push(Node { key: NodeKey::Slider(target), rect: slider, block });
}

/// Rectangle of the numeric value next to a slider.
pub(crate) fn value_rect(slider: Rect) -> Rect {
    Rect::new(
        slider.right() + volume_row::VALUE_GAP,
        slider.y,
        volume_row::VALUE_WIDTH,
        slider.height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_rows_are_balanced_fuller_first() {
        let cases: [(usize, &[usize]); 11] = [
            (0, &[]),
            (1, &[1]),
            (2, &[2]),
            (3, &[3]),
            (4, &[2, 2]),
            (5, &[3, 2]),
            (6, &[3, 3]),
            (7, &[3, 2, 2]),
            (8, &[3, 3, 2]),
            (9, &[3, 3, 3]),
            (10, &[3, 3, 2, 2]),
        ];
        for (count, rows) in cases {
            assert_eq!(tile_rows(count), rows, "{count} tiles");
        }
        for count in 1..40 {
            let rows = tile_rows(count);
            assert_eq!(rows.iter().sum::<usize>(), count);
            assert!(rows.iter().all(|&r| (1..=3).contains(&r)));
            assert!(rows.windows(2).all(|w| w[0] >= w[1] && w[0] - w[1] <= 1));
        }
    }

    #[test]
    fn tiles_grow_with_the_text() {
        assert!(tile_height(1.5) > tile_height(1.0));
    }
}
