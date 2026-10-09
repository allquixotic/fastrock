package ui

import (
	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/font"
	"github.com/aarzilli/nucular/label"
	"github.com/aarzilli/nucular/style"
	"golang.org/x/image/font/gofont/goregular"
	"golang.org/x/mobile/event/mouse"
	"image"
	"image/color"
)

type palette struct{ Window, Surface, Alt, Sunken, Hover, Selected, Border, Text, Muted, Faint, Accent, Success, Warning, Danger color.RGBA }

func hex(v uint32) color.RGBA { return color.RGBA{uint8(v >> 16), uint8(v >> 8), uint8(v), 255} }
func colors(light bool) palette {
	if light {
		return palette{hex(0xf6f6f7), hex(0xffffff), hex(0xf0f1f3), hex(0xebecef), hex(0xe6e8ec), hex(0xdde4f3), hex(0xd9dbe0), hex(0x1d1f24), hex(0x5f636d), hex(0x8a8e97), hex(0x2f6fe0), hex(0x1f8a55), hex(0xa86a00), hex(0xc43b31)}
	}
	return palette{hex(0x1b1c1f), hex(0x232428), hex(0x2b2d32), hex(0x18191c), hex(0x33363c), hex(0x3a3f4b), hex(0x3a3c42), hex(0xe6e7ea), hex(0xa0a3ab), hex(0x767a83), hex(0x5b8def), hex(0x4cb782), hex(0xe0a84a), hex(0xe5675f)}
}
func makeStyle(p palette, size int) *style.Style {
	s := style.FromTable(style.ColorTable{
		ColorText: p.Text, ColorWindow: p.Window, ColorHeader: p.Sunken, ColorHeaderFocused: p.Surface, ColorBorder: p.Border,
		ColorButton: p.Surface, ColorButtonHover: p.Hover, ColorButtonActive: p.Selected, ColorToggle: p.Alt, ColorToggleHover: p.Hover, ColorToggleCursor: p.Accent,
		ColorSelect: p.Surface, ColorSelectActive: p.Selected, ColorSlider: p.Alt, ColorSliderCursor: p.Accent, ColorSliderCursorHover: p.Accent, ColorSliderCursorActive: p.Accent,
		ColorProperty: p.Sunken, ColorEdit: p.Sunken, ColorEditCursor: p.Text, ColorCombo: p.Surface, ColorChart: p.Surface, ColorChartColor: p.Accent, ColorChartColorHighlight: p.Success,
		ColorScrollbar: p.Sunken, ColorScrollbarCursor: p.Border, ColorScrollbarCursorHover: p.Muted, ColorScrollbarCursorActive: p.Accent, ColorTabHeader: p.Surface,
	}, 1)
	s.Font, _ = font.NewFace(goregular.TTF, size)
	s.Button.Rounding = 4
	s.Button.Border = 1
	s.Button.Padding = image.Pt(10, 4)
	s.Edit.Rounding = 4
	s.Edit.Border = 1
	s.Edit.Padding = image.Pt(8, 5)
	s.NormalWindow.Padding = image.Pt(0, 0)
	s.NormalWindow.Spacing = image.Pt(0, 0)
	s.GroupWindow.Padding = image.Pt(10, 8)
	s.GroupWindow.Spacing = image.Pt(6, 6)
	s.GroupWindow.Border = 1
	return s
}
func textEditor(value string, multiline bool) *nucular.TextEditor {
	f := nucular.EditField
	if multiline {
		f = nucular.EditBox
	}
	return &nucular.TextEditor{Buffer: []rune(value), Flags: f, Maxlen: 2 << 20}
}
func text(ed *nucular.TextEditor) string {
	if ed == nil {
		return ""
	}
	return string(ed.Buffer)
}
func setText(ed *nucular.TextEditor, value string) {
	ed.Buffer = []rune(value)
	ed.Cursor = len(ed.Buffer)
	ed.SelectStart = ed.Cursor
	ed.SelectEnd = ed.Cursor
	ed.Redraw = true
}
func button(w *nucular.Window, s string, active bool, p palette) bool {
	old := w.Master().Style().Button
	if active {
		b := old
		b.Normal = style.MakeItemColor(p.Selected)
		b.BorderColor = p.Accent
		w.Master().Style().Button = b
	}
	clicked := w.ButtonText(s)
	w.Master().Style().Button = old
	return clicked
}
func primary(w *nucular.Window, s string, p palette) bool {
	old := w.Master().Style().Button
	b := old
	b.Normal = style.MakeItemColor(p.Accent)
	b.TextNormal = hex(0xffffff)
	w.Master().Style().Button = b
	hit := w.ButtonText(s)
	w.Master().Style().Button = old
	return hit
}
func title(w *nucular.Window, s string, p palette) {
	w.Row(28).Dynamic(1)
	w.LabelColored(s, "LC", p.Text)
}
func muted(w *nucular.Window, s string, p palette) {
	w.Row(22).Dynamic(1)
	w.LabelColored(s, label.Align("LC"), p.Muted)
}

// A multiline card title with the same hit target as a button.
func wrapButton(w *nucular.Window, value string, p palette) bool {
	state := w.CustomState()
	bounds, out := w.Custom(state)
	if out == nil {
		return false
	}
	fill := p.Alt
	in := w.Input()
	if in.Mouse.HoveringRect(bounds) {
		fill = p.Hover
	}
	out.FillRect(bounds, 4, fill)
	inner := bounds
	inner.X += 10
	inner.Y += 8
	inner.W -= 20
	inner.H -= 16
	face := w.Master().Style().Font
	lineHeight := nucular.FontHeight(face)
	for _, line := range nucular.WrapText(face, value, inner.W) {
		if inner.H < lineHeight {
			break
		}
		r := inner
		r.H = lineHeight
		out.DrawText(r, line, face, p.Text)
		inner.Y += lineHeight
		inner.H -= lineHeight
	}
	return in.Mouse.Clicked(mouse.ButtonLeft, bounds)
}
