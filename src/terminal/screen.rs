//! Bounded VT state projected into native Slint cells.
use std::{num::NonZeroU16, sync::Arc};
pub const HISTORY: usize = 5000;
pub const MAX_ROWS: u16 = 80;
pub const MAX_COLS: u16 = 200;
#[derive(Clone, Debug, PartialEq, Default)]
pub(super) struct Cell {
    pub text: String,
    pub fg: u32,
    pub bg: u32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub width: u8,
}
#[derive(Clone, Default)]
pub(super) struct Frame {
    pub cells: Arc<Vec<Cell>>,
    pub rows: u16,
    pub cols: u16,
    pub cursor: Option<(u16, u16)>,
    pub application_cursor: bool,
    pub scrollback: usize,
    pub history: usize,
    pub text: String,
    pub status: String,
    pub cwd: String,
    pub virtual_copy: bool,
    pub pid: u32,
    pub finished: bool,
}
pub(super) fn size(rows: u16, cols: u16) -> (NonZeroU16, NonZeroU16) {
    (
        NonZeroU16::new(rows.clamp(2, MAX_ROWS)).unwrap(),
        NonZeroU16::new(cols.clamp(10, MAX_COLS)).unwrap(),
    )
}
fn color(color: vt100::Color, default: u32) -> u32 {
    match color {
        vt100::Color::Default => default,
        vt100::Color::Rgb(r, g, b) => (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b),
        vt100::Color::Idx(i) => {
            const ANSI: [u32; 16] = [
                0x1b1d23, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xabb2bf,
                0x5c6370, 0xff7a85, 0xb2e08d, 0xffd68a, 0x80c7ff, 0xe69cff, 0x78dce8, 0xffffff,
            ];
            if i < 16 {
                ANSI[i as usize]
            } else if i >= 232 {
                let v = 8 + (u32::from(i) - 232) * 10;
                v * 0x010101
            } else {
                let n = u32::from(i) - 16;
                let level = |v| if v == 0 { 0 } else { 55 + 40 * v };
                (level(n / 36) << 16) | (level(n / 6 % 6) << 8) | level(n % 6)
            }
        }
    }
}
pub(super) fn snapshot(screen: &vt100::Screen, previous: &Frame) -> Frame {
    let (rows, cols) = screen.size();
    let mut cells = Vec::with_capacity(usize::from(rows.get()) * usize::from(cols.get()));
    for row in 0..rows.get() {
        for col in 0..cols.get() {
            let cell = screen.cell(row, col).unwrap();
            let mut fg = color(cell.fgcolor(), 0xd7dae0);
            let mut bg = color(cell.bgcolor(), 0x16181d);
            if cell.inverse() {
                std::mem::swap(&mut fg, &mut bg);
            }
            if cell.dim() {
                fg = ((fg & 0xfefefe) >> 1) + ((bg & 0xfefefe) >> 1);
            }
            cells.push(Cell {
                text: cell.contents().into(),
                fg,
                bg,
                bold: cell.bold(),
                italic: cell.italic(),
                underline: cell.underline(),
                width: if cell.is_wide_continuation() {
                    0
                } else if cell.is_wide() {
                    2
                } else {
                    1
                },
            });
        }
    }
    Frame {
        cells: Arc::new(cells),
        rows: rows.get(),
        cols: cols.get(),
        cursor: (!screen.hide_cursor() && screen.scrollback() == 0)
            .then(|| screen.cursor_position()),
        application_cursor: screen.application_cursor(),
        scrollback: screen.scrollback(),
        history: screen.scrollback_len(),
        text: screen.contents(),
        ..previous.clone()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v61_vt_preserves_colors_wide_cells_alt_screen_and_bounded_history() {
        let (rows, cols) = size(4, 20);
        let mut parser = vt100::Parser::new(rows, cols, HISTORY);
        parser.process(b"\x1b[31mred\x1b[0m ");
        parser.process("界".as_bytes());
        let frame = snapshot(parser.screen(), &Frame::default());
        assert_eq!(frame.cells[0].fg, 0xe06c75);
        assert_eq!(frame.cells[4].width, 2);
        assert_eq!(frame.cells[5].width, 0);
        parser.process(b"\x1b[?1049hother\x1b[?1049l");
        assert!(parser.screen().contents().contains("red"));
        for _ in 0..HISTORY + 100 {
            parser.process(b"line\r\n");
        }
        assert_eq!(parser.screen().scrollback_len(), HISTORY);
        parser.screen_mut().set_scrollback(usize::MAX);
        assert_eq!(parser.screen().scrollback(), HISTORY);
        let (r, c) = size(0, u16::MAX);
        parser.set_size(r, c);
        assert_eq!(parser.screen().size(), size(2, MAX_COLS));
    }
}
