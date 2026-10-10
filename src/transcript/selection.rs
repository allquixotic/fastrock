//! Select rendered glyphs without changing markdown layout or retaining history.
use crate::app::AppController;
use crate::ui::{Theme, TranscriptState};
use i_slint_core::item_tree::ItemRc;
use i_slint_core::item_tree::ParentItemTraversalMode;
use i_slint_core::items::{StyledTextItem, TextInput};
use i_slint_core::lengths::{LogicalPoint, LogicalRect, LogicalSize, ScaleFactor};
use i_slint_core::window::WindowInner;
use slint::ComponentHandle;
use std::ops::ControlFlow;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone)]
struct TextNode {
    item: ItemRc,
    text: String,
    rect: LogicalRect,
    visible_rect: LogicalRect,
}
impl TextNode {
    fn current_text(&self) -> String {
        if let Some(text) = self.item.downcast::<StyledTextItem>() {
            i_slint_core::styled_text::get_raw_text(&text.as_pin_ref().text()).into_owned()
        } else if let Some(input) = self.item.downcast::<TextInput>() {
            input.as_pin_ref().text().to_string()
        } else {
            String::new()
        }
    }
    fn cursor(
        &self,
        window: &slint::Window,
        point: LogicalPoint,
        offset: Option<usize>,
    ) -> (usize, LogicalRect) {
        let local = point - self.rect.origin.to_vector();
        if let Some(text) = self.item.downcast::<StyledTextItem>() {
            let (index, rect) = i_slint_core::textlayout::sharedparley::rich_text_cursor(
                ScaleFactor::new(window.scale_factor()),
                text.as_pin_ref(),
                &self.item,
                local,
                offset,
                window,
            );
            return (
                index
                    .min(self.text.len())
                    .min(self.text.floor_char_boundary(index.min(self.text.len()))),
                rect.translate(self.rect.origin.to_vector()),
            );
        }
        if let Some(input) = self.item.downcast::<TextInput>() {
            let adapter = WindowInner::from_pub(window).window_adapter();
            let renderer = adapter.renderer();
            let index = offset.unwrap_or_else(|| {
                i_slint_core::textlayout::sharedparley::text_input_byte_offset_for_position(
                    renderer,
                    input.as_pin_ref(),
                    &self.item,
                    local,
                    None,
                )
                .0
            });
            let rect =
                i_slint_core::textlayout::sharedparley::text_input_cursor_rect_for_byte_offset(
                    renderer,
                    input.as_pin_ref(),
                    &self.item,
                    index,
                    i_slint_core::items::TextCursorAffinity::NextCharacter,
                    None,
                );
            return (
                index.min(self.text.len()),
                rect.translate(self.rect.origin.to_vector()),
            );
        }
        (0, self.rect)
    }
    fn select(
        &self,
        anchor: usize,
        cursor: usize,
        background: slint::Color,
        foreground: slint::Color,
        caret: bool,
    ) {
        let a = i32::try_from(anchor).unwrap_or(i32::MAX);
        let b = i32::try_from(cursor).unwrap_or(i32::MAX);
        if let Some(text) = self.item.downcast::<StyledTextItem>() {
            let text = text.as_pin_ref();
            text.selection_anchor.set(a);
            text.selection_cursor.set(b);
            text.selection_background.set(background);
            text.selection_foreground.set(foreground);
            text.selection_caret_visible.set(caret);
        } else if let Some(input) = self.item.downcast::<TextInput>() {
            input.as_pin_ref().anchor_position_byte_offset.set(a);
            input.as_pin_ref().cursor_position_byte_offset.set(b);
        }
    }
}

fn nodes(window: &slint::Window, viewport: LogicalRect) -> Vec<TextNode> {
    let tracker = std::pin::pin!(i_slint_core::properties::PropertyTracker::<false>::default());
    tracker.as_ref().evaluate_as_dependency_root(|| {
        let inner = WindowInner::from_pub(window);
        let Some(component) = inner.try_component() else {
            return Vec::new();
        };
        let root = ItemRc::new_root(component);
        let mut nodes = Vec::new();
        root.visit_descendants::<()>(|item| {
            let geometry = item.geometry();
            let rect = LogicalRect::new(item.map_to_window(geometry.origin), geometry.size);
            if !item.is_visible() || !rect.intersects(&viewport) {
                return ControlFlow::Continue(());
            }
            let mut visible_rect = rect.intersection(&viewport);
            let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
            while let Some(ancestor) = parent {
                if ancestor.borrow().as_ref().clips_children() {
                    let geometry = ancestor.geometry();
                    let clip =
                        LogicalRect::new(ancestor.map_to_window(geometry.origin), geometry.size);
                    visible_rect = visible_rect.and_then(|rect| rect.intersection(&clip));
                }
                parent = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
            }
            let Some(visible_rect) = visible_rect else {
                return ControlFlow::Continue(());
            };
            let text = if let Some(text) = item.downcast::<StyledTextItem>() {
                i_slint_core::styled_text::get_raw_text(&text.as_pin_ref().text()).into_owned()
            } else if let Some(input) = item.downcast::<TextInput>()
                && input.as_pin_ref().read_only()
            {
                input.as_pin_ref().text().to_string()
            } else {
                return ControlFlow::Continue(());
            };
            if !text.is_empty() {
                nodes.push(TextNode {
                    item: item.clone(),
                    text,
                    rect,
                    visible_rect,
                });
            }
            ControlFlow::Continue(())
        });
        nodes.sort_by(|a, b| {
            a.rect
                .min_y()
                .total_cmp(&b.rect.min_y())
                .then_with(|| a.rect.min_x().total_cmp(&b.rect.min_x()))
        });
        nodes
    })
}

#[derive(Default)]
pub(crate) struct SelectionController {
    anchor: Option<(TextNode, usize)>,
    cursor: Option<(TextNode, usize)>,
    highlighted: Vec<TextNode>,
    text: String,
}
impl SelectionController {
    fn clear(&mut self) {
        for node in &self.highlighted {
            node.select(
                0,
                0,
                slint::Color::default(),
                slint::Color::default(),
                false,
            );
        }
        *self = Self::default();
    }
    fn apply(
        &mut self,
        visible: Vec<TextNode>,
        background: slint::Color,
        foreground: slint::Color,
    ) {
        for node in self.highlighted.drain(..) {
            node.select(0, 0, background, foreground, false);
        }
        self.text.clear();
        let (Some((anchor, a)), Some((cursor, b))) = (&self.anchor, &self.cursor) else {
            return;
        };
        // Virtualized rows can reuse an item for different text after scrolling.
        if anchor.current_text() != anchor.text || cursor.current_text() != cursor.text {
            self.clear();
            return;
        }
        let mut scene = visible;
        for endpoint in [anchor, cursor] {
            if !scene.iter().any(|node| node.item == endpoint.item) {
                scene.push(endpoint.clone());
            }
        }
        scene.sort_by(|a, b| {
            a.rect
                .min_y()
                .total_cmp(&b.rect.min_y())
                .then_with(|| a.rect.min_x().total_cmp(&b.rect.min_x()))
        });
        let Some(ai) = scene.iter().position(|node| node.item == anchor.item) else {
            return;
        };
        let Some(bi) = scene.iter().position(|node| node.item == cursor.item) else {
            return;
        };
        let (from, start, to, end) = if (ai, a) <= (bi, b) {
            (ai, *a, bi, *b)
        } else {
            (bi, *b, ai, *a)
        };
        for (index, node) in scene
            .into_iter()
            .enumerate()
            .filter(|(index, _)| *index >= from && *index <= to)
        {
            let start = if index == from { start } else { 0 };
            let end = if index == to { end } else { node.text.len() };
            let start = node.text.floor_char_boundary(start.min(node.text.len()));
            let end = node.text.floor_char_boundary(end.min(node.text.len()));
            node.select(
                start,
                end,
                background,
                foreground,
                from == to && start == end,
            );
            if index > from {
                self.text.push('\n');
            }
            self.text.push_str(&node.text[start..end]);
            self.highlighted.push(node);
        }
    }
}

fn horizontal(text: &str, offset: usize, right: bool, word: bool) -> usize {
    let offset = text.floor_char_boundary(offset.min(text.len()));
    let boundaries = if word {
        text.unicode_word_indices()
            .map(|(index, _)| index)
            .collect::<Vec<_>>()
    } else {
        text.grapheme_indices(true)
            .map(|(index, _)| index)
            .collect()
    };
    if right {
        boundaries
            .into_iter()
            .find(|index| *index > offset)
            .unwrap_or(text.len())
    } else {
        boundaries
            .into_iter()
            .rfind(|index| *index < offset)
            .unwrap_or(0)
    }
}

impl AppController {
    /// Test-only script path: dispatch actual pointer events through Slint.
    pub(crate) fn selection_automation(&mut self, args: &[String]) {
        let argument = |index: usize| args.get(index).map(String::as_str).unwrap_or("");
        match argument(0) {
            "capture" => {
                let text = self.clipboard_text().unwrap_or_default();
                if let Err(error) = std::fs::write(argument(1), text) {
                    eprintln!("codex-gui automation: clipboard capture failed: {error}");
                }
            }
            "drag" | "click" | "menu" => {
                let scene = nodes(self.window.window(), self.selection_viewport());
                let Some(node) = scene
                    .into_iter()
                    .find(|node| node.text.contains(argument(1)))
                else {
                    eprintln!(
                        "codex-gui automation: selection text not found: {:?}",
                        argument(1)
                    );
                    return;
                };
                let offset = |index| {
                    argument(index)
                        .parse::<usize>()
                        .unwrap_or(0)
                        .min(node.text.len())
                };
                let point = |offset| {
                    let (_, rect) =
                        node.cursor(self.window.window(), node.rect.origin, Some(offset));
                    slint::LogicalPosition::new(rect.min_x(), rect.min_y() + rect.height() / 2.0)
                };
                let start = point(offset(2));
                if argument(0) == "menu" {
                    use slint::platform::{PointerEventButton, WindowEvent};
                    self.automation_dispatch(vec![
                        WindowEvent::PointerPressed {
                            position: start,
                            button: PointerEventButton::Right,
                        },
                        WindowEvent::PointerReleased {
                            position: start,
                            button: PointerEventButton::Right,
                        },
                    ]);
                    return;
                }
                let end = point(if argument(0) == "click" {
                    offset(2)
                } else {
                    offset(3)
                });
                eprintln!("codex-gui automation: selection points {start:?} -> {end:?}");
                use slint::platform::{PointerEventButton, WindowEvent};
                self.automation_dispatch(vec![
                    WindowEvent::PointerPressed {
                        position: start,
                        button: PointerEventButton::Left,
                    },
                    WindowEvent::PointerMoved { position: end },
                    WindowEvent::PointerReleased {
                        position: end,
                        button: PointerEventButton::Left,
                    },
                ]);
            }
            _ => eprintln!(
                "codex-gui automation: unknown selection action {:?}",
                argument(0)
            ),
        }
    }
    fn selection_viewport(&self) -> LogicalRect {
        let view = self.window.global::<TranscriptState>().get_viewport();
        LogicalRect::new(
            LogicalPoint::new(view.x, view.y),
            LogicalSize::new(view.width, view.height),
        )
    }
    pub(crate) fn selection_clear(&mut self) {
        self.transcript_selection.clear();
        self.window
            .global::<TranscriptState>()
            .set_selection_active(false);
    }
    pub(crate) fn selection_blur(&mut self) {
        for node in &self.transcript_selection.highlighted {
            if let Some(text) = node.item.downcast::<StyledTextItem>() {
                text.as_pin_ref().selection_caret_visible.set(false);
            }
        }
    }
    pub(crate) fn selection_pointer(&mut self, x: f32, y: f32, start: bool, extend: bool) {
        let point = LogicalPoint::new(x, y);
        let scene = nodes(self.window.window(), self.selection_viewport());
        let node = scene
            .iter()
            .find(|node| node.visible_rect.contains(point))
            .or_else(|| {
                scene.iter().min_by(|a, b| {
                    let distance = |node: &TextNode| {
                        (point.y
                            - point
                                .y
                                .clamp(node.visible_rect.min_y(), node.visible_rect.max_y()))
                        .abs()
                            + (point.x
                                - point
                                    .x
                                    .clamp(node.visible_rect.min_x(), node.visible_rect.max_x()))
                            .abs()
                    };
                    distance(a).total_cmp(&distance(b))
                })
            })
            .cloned();
        let Some(node) = node else {
            return;
        };
        let offset = node.cursor(self.window.window(), point, None).0;
        if std::env::var_os("CODEX_GUI_AUTOMATION").is_some() {
            eprintln!(
                "codex-gui automation: selection pointer ({x}, {y}) start={start} offset={offset}"
            );
        }
        if start && (!extend || self.transcript_selection.anchor.is_none()) {
            self.transcript_selection.clear();
            self.transcript_selection.anchor = Some((node.clone(), offset));
        }
        self.transcript_selection.cursor = Some((node, offset));
        self.selection_apply(scene);
    }
    fn selection_apply(&mut self, scene: Vec<TextNode>) {
        let theme = self.window.global::<Theme>();
        self.transcript_selection
            .apply(scene, theme.get_selected(), theme.get_text());
        self.window
            .global::<TranscriptState>()
            .set_selection_active(!self.transcript_selection.text.is_empty());
    }
    pub(crate) fn selection_copy(&mut self) {
        if self
            .transcript_selection
            .highlighted
            .iter()
            .any(|node| node.current_text() != node.text)
        {
            self.selection_clear();
            return;
        }
        let text = self.transcript_selection.text.clone();
        if !text.is_empty() {
            self.copy_to_clipboard(&text);
        }
    }
    pub(crate) fn selection_key(&mut self, key: &str, shift: bool, primary: bool) -> bool {
        let Some((node, offset)) = self.transcript_selection.cursor.clone() else {
            return false;
        };
        if primary && key.eq_ignore_ascii_case("c") {
            self.selection_copy();
            return true;
        }
        if key == "escape" {
            self.selection_clear();
            return true;
        }
        if primary && key.eq_ignore_ascii_case("a") {
            self.transcript_selection.anchor = Some((node.clone(), 0));
            self.transcript_selection.cursor = Some((node.clone(), node.text.len()));
            self.selection_apply(nodes(self.window.window(), self.selection_viewport()));
            return true;
        }
        let scene = nodes(self.window.window(), self.selection_viewport());
        if matches!(key, "left" | "right") {
            // Standard arrow behavior collapses an existing range first.
            if !shift
                && !self.transcript_selection.text.is_empty()
                && let Some(anchor) = self.transcript_selection.anchor.clone()
            {
                let anchor_first = (anchor.0.rect.min_y(), anchor.0.rect.min_x(), anchor.1)
                    <= (node.rect.min_y(), node.rect.min_x(), offset);
                let endpoint = if (key == "left") == anchor_first {
                    anchor
                } else {
                    (node, offset)
                };
                self.transcript_selection.anchor = Some(endpoint.clone());
                self.transcript_selection.cursor = Some(endpoint);
                self.selection_apply(scene);
                return true;
            }
            // A paragraph boundary is part of the conversation's selected text.
            if (key == "left" && offset == 0) || (key == "right" && offset == node.text.len()) {
                let neighbor = scene
                    .iter()
                    .position(|item| item.item == node.item)
                    .and_then(|index| {
                        if key == "left" {
                            index.checked_sub(1)
                        } else {
                            index.checked_add(1)
                        }
                    })
                    .and_then(|index| scene.get(index))
                    .cloned();
                if let Some(neighbor) = neighbor {
                    let target = if key == "left" {
                        neighbor.text.len()
                    } else {
                        0
                    };
                    if !shift {
                        self.transcript_selection.anchor = Some((neighbor.clone(), target));
                    }
                    self.transcript_selection.cursor = Some((neighbor, target));
                    self.selection_apply(scene);
                    return true;
                }
            }
        }
        let target = match key {
            "left" => horizontal(&node.text, offset, false, primary),
            "right" => horizontal(&node.text, offset, true, primary),
            "home" | "end" => {
                if primary {
                    if key == "home" { 0 } else { node.text.len() }
                } else {
                    let (_, rect) =
                        node.cursor(self.window.window(), node.rect.origin, Some(offset));
                    let x = if key == "home" {
                        node.rect.min_x()
                    } else {
                        node.rect.max_x()
                    };
                    node.cursor(
                        self.window.window(),
                        LogicalPoint::new(x, rect.min_y() + rect.height() / 2.0),
                        None,
                    )
                    .0
                }
            }
            "up" | "down" => {
                let (_, rect) = node.cursor(self.window.window(), node.rect.origin, Some(offset));
                let y = if key == "up" {
                    rect.min_y() - rect.height() / 2.0
                } else {
                    rect.max_y() + rect.height() / 2.0
                };
                node.cursor(
                    self.window.window(),
                    LogicalPoint::new(rect.min_x(), y),
                    None,
                )
                .0
            }
            _ => return false,
        };
        if !shift {
            self.transcript_selection.anchor = Some((node.clone(), target));
        }
        self.transcript_selection.cursor = Some((node, target));
        self.selection_apply(scene);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v9_keyboard_offsets_respect_graphemes_and_utf8() {
        let text = "a👩‍💻é z";
        let first = horizontal(text, 1, true, false);
        assert_eq!(&text[1..first], "👩‍💻");
        assert_eq!(horizontal(text, first, false, false), 1);
        assert_eq!(horizontal(text, 0, false, false), 0);
        assert_eq!(horizontal(text, text.len(), true, false), text.len());
        assert!(text.is_char_boundary(horizontal(text, 1, true, true)));
        assert_eq!(horizontal("fix auth tests", 0, true, true), 4);
        assert_eq!(horizontal("fix auth tests", 7, false, true), 4);
    }
}
