package ui

import (
	"image/color"
	"slices"
	"strconv"
	"strings"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/rect"
	"github.com/allquixotic/fastrock/internal/rally"
	"golang.org/x/mobile/event/mouse"
)

// The WSAPI palette is documented in Broadcom article 11610. Prefer schema
// values if a workspace advertises a different set; preserve unknown values.
var rallyColors = []struct{ Value, Name string }{
	{"#105cab", "Dark Blue"}, {"#21a2e0", "Blue"}, {"#107c1e", "Green"},
	{"#4a1d7e", "Purple"}, {"#df1a7b", "Pink"}, {"#ee6c19", "Burnt Orange"},
	{"#f9a814", "Orange"}, {"#fce205", "Yellow"}, {"#848689", "Grey"},
}

func (a *App) detailColor(w *nucular.Window, d *detailView, f rally.Field) {
	ed := d.Editors[f.Name]
	if ed == nil {
		return
	}
	title(w, detailCaption(f), a.p)
	values := slices.Clone(f.AllowedValues)
	if len(values) == 0 {
		for _, c := range rallyColors {
			values = append(values, c.Value)
		}
	}
	current := text(ed)
	currentName := current
	for _, known := range rallyColors {
		if strings.EqualFold(known.Value, current) {
			currentName = known.Name
		}
	}
	muted(w, "Current: "+fallback(currentName, "Default"), a.p)
	w.Row(28).Dynamic(1)
	if enabledButton(w, "Default color", !d.Saving && !f.Required, current == "", a.p) {
		setText(ed, "")
	}
	for start := 0; start < len(values); start += 5 {
		row := values[start:min(start+5, len(values))]
		w.Row(30).Dynamic(len(row))
		for _, value := range row {
			label := value
			for _, known := range rallyColors {
				if strings.EqualFold(known.Value, value) {
					label = known.Name
					break
				}
			}
			b, out := w.Custom(w.CustomState())
			if out == nil {
				continue
			}
			fill := a.p.Surface
			if n, err := strconv.ParseUint(strings.TrimPrefix(value, "#"), 16, 24); err == nil && len(value) == 7 {
				fill = color.RGBA{R: byte(n >> 16), G: byte(n >> 8), B: byte(n), A: 255}
			}
			if strings.EqualFold(current, value) {
				out.FillRect(b, 3, a.p.Text)
			}
			out.FillRect(inset(b, 3, 3), 3, fill)
			if w.Input().Mouse.HoveringRect(b) {
				w.Tooltip(label)
				if !d.Saving && w.Input().Mouse.Clicked(mouse.ButtonLeft, b) {
					setText(ed, value)
				}
			}
			// A short label keeps named non-hex schema values visible too.
			if len(value) != 7 || !strings.HasPrefix(value, "#") {
				labelAt(out, rect.Rect{X: b.X, Y: b.Y, W: b.W, H: b.H}, cut(label, 7), w.Master().Style().Font, a.p.Text)
			}
		}
	}
}
