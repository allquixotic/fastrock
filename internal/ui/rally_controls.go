package ui

import (
	"fmt"
	"image"
	"slices"
	"strconv"
	"strings"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/rect"
	"golang.org/x/mobile/event/mouse"
)

type rallyModeChoice struct{ Key, Label string }

func (a *App) drawRallyViewLabel(w *desktop.Window, v *rallyView) {
	b, out := w.Custom(w.CustomState())
	if out == nil {
		return
	}
	labelAt(out, inset(b, 4, 0), "Saved view", w.Master().Style().Font, a.p.Muted)
	if v.AIView {
		scale := w.Master().Style().Scaling
		badge := rect.Rect{X: b.X + int(105*scale), Y: b.Y + 2, W: int(80 * scale), H: b.H - 4}
		out.FillRect(badge, 4, a.p.AccentSoft)
		labelAt(out, inset(badge, 8, 0), "AI view", w.Master().Style().Font, a.p.Accent)
	}
}

func rallyModeChoices(v *rallyView) []rallyModeChoice {
	switch {
	case v.Spec.ID == "portfoliokanban":
		return []rallyModeChoice{{"board", "Board"}, {"charts", "Charts"}}
	case v.Spec.Mode == "board":
		return []rallyModeChoice{{"list", "List"}, {"board", "Board"}, {"charts", "Charts"}}
	case v.Spec.ID == "timeboxes":
		return []rallyModeChoice{{"list", "List"}, {"charts", "Charts"}}
	case v.Spec.Mode == "planning":
		return []rallyModeChoice{{"list", "List"}, {"planning", "Planning"}, {"charts", "Charts"}}
	case v.Spec.Mode == "timeline":
		return []rallyModeChoice{{"list", "List"}, {"timeline", "Timeline"}, {"charts", "Charts"}}
	}
	return nil
}

func (a *App) drawRallyModes(w *desktop.Window, v *rallyView) {
	choices := rallyModeChoices(v)
	if len(choices) > 0 {
		a.rallyRow(w, v, "modes", func(w *desktop.Window) {
			var captions []string
			for _, choice := range choices {
				captions = append(captions, choice.Label)
			}
			compactRallyButtons(w, 26, captions, 44, func(i int) {
				choice := choices[i]
				if rallyModeButton(w, choice, v.Mode == choice.Key, a.p) {
					v.Mode = choice.Key
				}
			})
		})
	}
	if v.Mode == "board" || v.Mode == "list" {
		a.rallyRow(w, v, "density", func(w *desktop.Window) { a.drawBoardDisplayControls(w, v) })
	}
}

func rallyModeButton(w *desktop.Window, choice rallyModeChoice, active bool, p palette) bool {
	b, out := w.Custom(w.CustomState())
	if out == nil {
		return false
	}
	in := w.Input()
	fg := p.Text
	if active {
		out.FillRect(b, 3, hex(0x3272d9))
		fg = hex(0xffffff)
	} else if in.Mouse.HoveringRect(b) {
		out.FillRect(b, 3, p.Hover)
	}
	scale := float64(w.Master().Style().Scaling)
	x, y := b.X+int(10*scale), b.Y+b.H/2-int(7*scale)
	unit := max(1, int(3*scale))
	// Draw icons directly; appearance does not depend on Unicode glyph coverage.
	switch choice.Key {
	case "board":
		for i := range 3 {
			out.FillRect(rect.Rect{X: x + i*2*unit, Y: y, W: unit, H: 4 * unit}, 0, fg)
		}
	case "charts":
		for i := range 3 {
			out.FillRect(rect.Rect{X: x + i*2*unit, Y: y + (2-i)*unit, W: unit, H: (i + 2) * unit}, 0, fg)
		}
	case "planning", "timeline":
		for i := range 3 {
			out.FillRect(rect.Rect{X: x + i*unit, Y: y + i*2*unit, W: 3 * unit, H: unit}, 0, fg)
		}
	default:
		for i := range 3 {
			out.StrokeLine(image.Pt(x, y+i*2*unit), image.Pt(x+5*unit, y+i*2*unit), 1, fg)
		}
	}
	left := x + 7*unit
	labelAt(out, rect.Rect{X: left, Y: b.Y, W: max(0, b.X+b.W-left-int(6*scale)), H: b.H}, choice.Label, w.Master().Style().Font, fg)
	if in.Mouse.HoveringRect(b) {
		w.Tooltip(choice.Label)
	}
	return in.Mouse.Clicked(mouse.ButtonLeft, b)
}

func rallyGroupChoices(v *rallyView) (keys, labels []string) {
	keys = []string{"None", "Owner", "Feature", "Iteration", "Project", "Release"}
	if v.Group != "" && !contains(keys, v.Group) {
		keys = append(keys, v.Group)
	}
	for _, key := range keys {
		labels = append(labels, rallyFieldLabel(v, key))
	}
	return
}

func (a *App) drawRallyGrouping(w *desktop.Window, v *rallyView) {
	label := "Group By"
	if v.Mode == "board" {
		label = "Swimlanes"
	}
	w.Row(28).StaticScaled(rallyButtonWidth(w, label, 16), min(int(320*w.Master().Style().Scaling), w.LayoutAvailableWidth()-rallyButtonWidth(w, label, 16)-int(6*w.Master().Style().Scaling)))
	w.Label(label, "LC")
	keys, labels := rallyGroupChoices(v)
	old := index(keys, fallback(v.Group, "None"))
	if next := w.ComboSimple(labels, old, 28); next != old {
		v.Group = keys[next]
		a.refreshRally(v)
	}
}

type rallyActiveFilter struct{ Key, Label string }

func rallyChipValue(value string) string {
	return strings.Join(strings.Fields(cut(value, 180)), " ")
}

func (a *App) rallyFilterLabel(key, value string) string {
	if p := a.scopeChoices[key]; p != nil {
		if i, ok := p.indices[value]; ok {
			return p.Names[i]
		}
	}
	return value
}

func (a *App) activeRallyFilters(v *rallyView) []rallyActiveFilter {
	var filters []rallyActiveFilter
	if value := strings.TrimSpace(text(v.Search)); value != "" {
		filters = append(filters, rallyActiveFilter{"search", "Search: " + rallyChipValue(value)})
	}
	if v.CurrentIteration {
		filters = append(filters, rallyActiveFilter{"timebox", "Timebox: Current iteration"})
	} else if v.Timebox != "" || v.TimeboxName != "" {
		filters = append(filters, rallyActiveFilter{"timebox", "Timebox: " + rallyChipValue(fallback(v.TimeboxName, a.rallyFilterLabel("Iteration", v.Timebox)))})
	}
	if v.ReleaseTimebox != "" || v.ReleaseName != "" {
		filters = append(filters, rallyActiveFilter{"release", "Release: " + rallyChipValue(fallback(v.ReleaseName, a.rallyFilterLabel("Release", v.ReleaseTimebox)))})
	}
	if v.OwnerFilter != "" {
		filters = append(filters, rallyActiveFilter{"owner", rallyFieldLabel(v, "Owner") + " is " + rallyChipValue(a.rallyFilterLabel("OwnerFilter", v.OwnerFilter))})
	}
	if v.StateFilter != "" {
		filters = append(filters, rallyActiveFilter{"state", rallyFieldLabel(v, v.stateField()) + " is " + rallyChipValue(v.StateFilter)})
	}
	if v.OnlyBlocked {
		filters = append(filters, rallyActiveFilter{"blocked", "Blocked is true"})
	}
	if v.OnlyReady {
		filters = append(filters, rallyActiveFilter{"ready", "Ready is true"})
	}
	if v.QueryApplied != "" {
		filters = append(filters, rallyActiveFilter{"query", "Query: " + rallyChipValue(v.QueryApplied)})
	}
	for i, f := range v.StructuredFilters {
		label, _, valid := rallyFilterField(f.Field)
		if !valid {
			label = f.Field
		}
		filters = append(filters, rallyActiveFilter{"structured:" + strconv.Itoa(i), label + " " + f.Operator + " " + rallyChipValue(fallback(f.Label, f.Value))})
	}
	return filters
}

func (v *rallyView) removeFilter(key string) bool {
	if raw, ok := strings.CutPrefix(key, "structured:"); ok {
		i, err := strconv.Atoi(raw)
		if err != nil || i < 0 || i >= len(v.StructuredFilters) {
			return false
		}
		v.StructuredFilters = slices.Delete(v.StructuredFilters, i, i+1)
		v.filterValid = false
		return true
	}
	switch key {
	case "search":
		setText(v.Search, "")
	case "timebox":
		v.Timebox, v.TimeboxName = "", ""
		v.CurrentIteration = false
	case "release":
		v.ReleaseTimebox, v.ReleaseName = "", ""
	case "owner":
		v.OwnerFilter = ""
	case "state":
		v.StateFilter = ""
	case "blocked":
		v.OnlyBlocked = false
	case "ready":
		v.OnlyReady = false
	case "query":
		v.QueryApplied = ""
		setText(v.Query, "")
	default:
		return false
	}
	return true
}

func rallyFilterCaption(open bool, count int) string {
	action := "Show"
	if open {
		action = "Hide"
	}
	noun := "Filters"
	if count == 1 {
		noun = "Filter"
	}
	return fmt.Sprintf("%s %d %s", action, count, noun)
}

func (a *App) drawRallyFilterChips(w *desktop.Window, v *rallyView, filters []rallyActiveFilter) {
	style := w.Master().Style()
	scale := float64(style.Scaling)
	width := max(1, w.LayoutAvailableWidth())
	spacing := max(1, int(8*scale))
	for start := 0; start < len(filters); {
		var widths []int
		used := 0
		for i := start; i < len(filters); i++ {
			n := min(width, desktop.FontWidth(style.Font, filters[i].Label)+int(40*scale))
			if len(widths) > 0 && used+spacing+n > width {
				break
			}
			widths = append(widths, n)
			used += n + spacing
		}
		w.RowScaled(int(28 * scale)).StaticScaled(widths...)
		for i := range widths {
			filter := filters[start+i]
			if rallyFilterChip(w, filter.Label, a.p) && v.removeFilter(filter.Key) {
				a.refreshRally(v)
			}
		}
		start += len(widths)
	}
}

func rallyFilterChip(w *desktop.Window, label string, p palette) bool {
	b, out := w.Custom(w.CustomState())
	if out == nil {
		return false
	}
	in := w.Input()
	fill := p.Selected
	if in.Mouse.HoveringRect(b) {
		fill = p.Hover
		w.Tooltip("Remove filter: " + label)
	}
	out.FillRect(b, 4, fill)
	scale := float64(w.Master().Style().Scaling)
	pad, closeSize := int(8*scale), int(20*scale)
	labelAt(out, rect.Rect{X: b.X + pad, Y: b.Y, W: max(0, b.W-closeSize-2*pad), H: b.H}, label, w.Master().Style().Font, p.Text)
	closeGlyph(out, rect.Rect{X: b.X + b.W - closeSize, Y: b.Y, W: closeSize, H: b.H}, p.Text)
	return in.Mouse.Clicked(mouse.ButtonLeft, b)
}
