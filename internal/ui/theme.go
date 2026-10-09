package ui

import (
	"image"
	"image/color"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/label"
	"github.com/aarzilli/nucular/style"
)

type palette struct {
	Window, Surface, Alt, Sunken, Hover, Selected, Border, Text, Muted, Faint, Accent, Success, Warning, Danger color.RGBA
	BorderStrong, AccentSoft, SuccessSoft, WarningSoft, DangerSoft                                              color.RGBA
	DiffAddBG, DiffDelBG, DiffAddFG, DiffDelFG, DiffHunkFG                                                      color.RGBA
}

func hex(v uint32) color.RGBA { return color.RGBA{uint8(v >> 16), uint8(v >> 8), uint8(v), 255} }
func colors(light bool) palette {
	if light {
		return palette{hex(0xf6f6f7), hex(0xffffff), hex(0xf0f1f3), hex(0xebecef), hex(0xe6e8ec), hex(0xdde4f3), hex(0xd9dbe0), hex(0x1d1f24), hex(0x5f636d), hex(0x8a8e97), hex(0x2f6fe0), hex(0x1f8a55), hex(0xa86a00), hex(0xc43b31), hex(0xb9bcc4), hex(0xe3ecfc), hex(0xe2f4ea), hex(0xfbf0dc), hex(0xfbe5e3), hex(0xe6f6ec), hex(0xfbe9e8), hex(0x1d7a46), hex(0xb3362c), hex(0x4a64a8)}
	}
	return palette{hex(0x1b1c1f), hex(0x232428), hex(0x2b2d32), hex(0x18191c), hex(0x33363c), hex(0x3a3f4b), hex(0x3a3c42), hex(0xe6e7ea), hex(0xa0a3ab), hex(0x767a83), hex(0x5b8def), hex(0x4cb782), hex(0xe0a84a), hex(0xe5675f), hex(0x50535a), hex(0x26324a), hex(0x1f3a2d), hex(0x3d3220), hex(0x45252a), hex(0x1f3a2a), hex(0x44262a), hex(0x7fd4a3), hex(0xf19a94), hex(0x8ea7d9)}
}
func makeStyle(p palette, size int) *style.Style {
	s := style.FromTable(style.ColorTable{
		ColorText: p.Text, ColorWindow: p.Window, ColorHeader: p.Sunken, ColorHeaderFocused: p.Surface, ColorBorder: p.Border,
		ColorButton: p.Surface, ColorButtonHover: p.Hover, ColorButtonActive: p.Selected, ColorToggle: p.Alt, ColorToggleHover: p.Hover, ColorToggleCursor: p.Accent,
		ColorSelect: p.Surface, ColorSelectActive: p.Selected, ColorSlider: p.Alt, ColorSliderCursor: p.Accent, ColorSliderCursorHover: p.Accent, ColorSliderCursorActive: p.Accent,
		ColorProperty: p.Sunken, ColorEdit: p.Sunken, ColorEditCursor: p.Text, ColorCombo: p.Surface, ColorChart: p.Surface, ColorChartColor: p.Accent, ColorChartColorHighlight: p.Success,
		ColorScrollbar: p.Sunken, ColorScrollbarCursor: p.Border, ColorScrollbarCursorHover: p.Muted, ColorScrollbarCursorActive: p.Accent, ColorTabHeader: p.Surface,
	}, 1)
	s.Font = typeFace(size, regularFont)
	s.Unscaled().Font = s.Font
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
	s.GroupWindow.Border = 0
	return s
}
func textEditor(value string, multiline bool) *nucular.TextEditor {
	f := nucular.EditField
	if multiline {
		f = nucular.EditBox | nucular.EditSoftWrap | nucular.EditNoHorizontalScroll
	}
	ed := &nucular.TextEditor{Buffer: []rune(value), Flags: f, Maxlen: 2 << 20}
	ed.TrackChanges()
	return ed
}
func text(ed *nucular.TextEditor) string {
	if ed == nil {
		return ""
	}
	return ed.Snapshot()
}
func setText(ed *nucular.TextEditor, value string) {
	ed.SetText(value)
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
		b.Hover = style.MakeItemColor(p.Hover)
		b.Active = style.MakeItemColor(p.Selected)
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
	b.Hover = style.MakeItemColor(shade(p.Accent, 0.88))
	b.Active = style.MakeItemColor(shade(p.Accent, 0.72))
	b.TextNormal, b.TextHover, b.TextActive = hex(0xffffff), hex(0xffffff), hex(0xffffff)
	w.Master().Style().Button = b
	hit := w.ButtonText(s)
	w.Master().Style().Button = old
	return hit
}
func shade(c color.RGBA, f float64) color.RGBA {
	return color.RGBA{uint8(float64(c.R) * f), uint8(float64(c.G) * f), uint8(float64(c.B) * f), c.A}
}
func dangerButton(w *nucular.Window, s string, p palette) bool {
	p.Accent = p.Danger
	return primary(w, s, p)
}
func title(w *nucular.Window, s string, p palette) {
	old := w.Master().Style().Font
	face := typeFace(fontPointSize(old)+7, boldFont)
	height := titleHeight(w)
	w.Master().Style().Font = face
	w.Row(height).Dynamic(1)
	w.LabelColored(s, "LC", p.Text)
	w.Master().Style().Font = old
}
func muted(w *nucular.Window, s string, p palette) {
	old := w.Master().Style().Font
	face := typeFace(fontPointSize(old)-2, regularFont)
	w.Master().Style().Font = face
	w.Row(max(22, nucular.FontHeight(face)+6)).Dynamic(1)
	w.LabelColored(s, label.Align("LC"), p.Muted)
	w.Master().Style().Font = old
}

func titleHeight(w *nucular.Window) int {
	face := typeFace(fontPointSize(w.Master().Style().Font)+7, boldFont)
	return max(28, nucular.FontHeight(face)+8)
}
func codeEditor(w *nucular.Window, ed *nucular.TextEditor) {
	old := w.Master().Style().Font
	w.Master().Style().Font = typeFace(fontPointSize(old)-1, monoFont)
	ed.Edit(w)
	w.Master().Style().Font = old
}
func (a *App) codeField(w *nucular.Window, name string, ed *nucular.TextEditor, multiline bool) {
	title(w, name, a.p)
	height := 30
	if multiline {
		height = 110
	}
	w.Row(height).Dynamic(1)
	codeEditor(w, ed)
}
