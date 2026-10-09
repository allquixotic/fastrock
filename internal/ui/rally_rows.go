package ui

import (
	"image"
	"slices"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/rect"
)

type rallyControlRow struct{ ID, Label string }

func (a *App) rallyRows(v *rallyView) []rallyControlRow {
	rows := []rallyControlRow{{"sections", "Sections"}, {"pages", "Pages"}, {"project", "Team"}, {"page", "Page"}, {"freshness", "Updated"}}
	if v.Detail != nil || v.Spec.ID == "customviews" {
		return rows
	}
	rows = append(rows, rallyControlRow{"saved-view", "Saved view"})
	if rallyTimeboxesSupported(v) {
		rows = append(rows, rallyControlRow{"timeboxes", "Iteration"})
	}
	rows = append(rows, rallyControlRow{"view-actions", "Actions"})
	if len(rallyModeChoices(v)) > 0 {
		rows = append(rows, rallyControlRow{"modes", "View"})
	}
	if v.Mode == "board" || v.Mode == "list" {
		rows = append(rows, rallyControlRow{"density", "Density"})
	}
	if a.findSavedView(v.Spec.ID, v.ViewName) != nil {
		rows = append(rows, rallyControlRow{"saved-changes", "View changes"})
	}
	rows = append(rows, rallyControlRow{"search", "Search"}, rallyControlRow{"swimlanes", "Swimlanes"})
	if len(a.activeRallyFilters(v)) > 0 {
		rows = append(rows, rallyControlRow{"filter-chips", "Active filters"})
	}
	if v.Filters {
		rows = append(rows, rallyControlRow{"filters", "Filters"})
	}
	if v.ShowFields {
		rows = append(rows, rallyControlRow{"fields", "Fields"})
	}
	if v.Mode == "board" {
		rows = append(rows, rallyControlRow{"widgets", "Widgets"})
	}
	return append(rows, rallyControlRow{"summary", "Totals"})
}

func (a *App) rallyRowHidden(id string) bool {
	return slices.Contains(a.prefs.RallyHiddenRows, id) || a.prefs.RallyNavHidden && (id == "sections" || id == "pages")
}

func (a *App) setRallyRowHidden(id string, hidden bool) {
	if a.prefs.RallyNavHidden {
		for _, old := range []string{"sections", "pages"} {
			if !slices.Contains(a.prefs.RallyHiddenRows, old) {
				a.prefs.RallyHiddenRows = append(a.prefs.RallyHiddenRows, old)
			}
		}
		a.prefs.RallyNavHidden = false
	}
	if hidden && !slices.Contains(a.prefs.RallyHiddenRows, id) {
		a.prefs.RallyHiddenRows = append(a.prefs.RallyHiddenRows, id)
	} else if !hidden {
		a.prefs.RallyHiddenRows = slices.DeleteFunc(a.prefs.RallyHiddenRows, func(row string) bool { return row == id })
	}
	a.savePrefs()
}

// Hidden rows share a wrapping restore strip, so each hidden row costs no
// additional height. Hiding controls never changes their selected values.
func (a *App) drawRallyRowRestore(w *desktop.Window, v *rallyView) {
	var rows []rallyControlRow
	var captions []string
	for _, row := range a.rallyRows(v) {
		if a.rallyRowHidden(row.ID) {
			rows = append(rows, row)
			captions = append(captions, "+ "+row.Label)
		}
	}
	if len(rows) == 0 {
		return
	}
	if len(rows) > 1 {
		captions = append(captions, "+ All")
	}
	compactRallyButtons(w, 22, captions, 24, func(i int) {
		if w.ButtonText(captions[i]) {
			if i == len(rows) {
				a.prefs.RallyHiddenRows, a.prefs.RallyNavHidden = nil, false
				a.savePrefs()
			} else {
				a.setRallyRowHidden(rows[i].ID, false)
			}
		}
	})
}

// The small gutter button belongs to the whole row, including wrapping or
// expanded forms. Measure the child instead of assuming a fixed row height.
func (a *App) rallyRow(w *desktop.Window, v *rallyView, id string, draw func(*desktop.Window)) {
	if a.rallyRowHidden(id) {
		return
	}
	scale := w.Master().Style().Scaling
	key := v.Spec.ID + "/" + id
	if a.rallyRowHeights == nil {
		a.rallyRowHeights = map[string]int{}
	}
	height := a.rallyRowHeights[key]
	if height == 0 {
		height = int(28 * scale)
	}
	bounds := w.RowScaled(height).SpaceBegin(2)
	w.LayoutSpacePushScaled(rect.Rect{W: int(18 * scale), H: int(22 * scale)})
	oldButton := w.Master().Style().Button
	w.Master().Style().Button.Padding = image.Point{}
	if w.ButtonText("−") {
		a.setRallyRowHidden(id, true)
	}
	w.Master().Style().Button = oldButton
	if w.Input().Mouse.HoveringRect(w.LastWidgetBounds) {
		label := id
		for _, row := range a.rallyRows(v) {
			if row.ID == id {
				label = row.Label
				break
			}
		}
		w.Tooltip("Hide " + label + " row")
	}
	gutter := int(23 * scale)
	w.LayoutSpacePushScaled(rect.Rect{X: gutter, W: max(1, bounds.W-gutter), H: height})
	oldGroup := w.Master().Style().GroupWindow
	w.Master().Style().GroupWindow.Padding = image.Point{}
	w.Master().Style().GroupWindow.Spacing = image.Pt(int(5*scale), int(3*scale))
	if row := w.GroupBegin("rally-row-"+key, desktop.WindowNoScrollbar); row != nil {
		draw(row)
		measured := max(int(22*scale), row.LayoutNextRowY()-row.Bounds.Y-int(3*scale))
		row.GroupEnd()
		if measured != a.rallyRowHeights[key] {
			a.rallyRowHeights[key] = measured
			w.Master().Changed()
		}
	}
	w.Master().Style().GroupWindow = oldGroup
}

func rallyButtonWidth(w *desktop.Window, caption string, padding int) int {
	return desktop.FontWidth(w.Master().Style().Font, caption) + int(float64(padding)*w.Master().Style().Scaling)
}

func compactRallyButtons(w *desktop.Window, height int, captions []string, padding int, draw func(int)) {
	width, gap := w.LayoutAvailableWidth(), w.WindowStyle().Spacing.X
	for start := 0; start < len(captions); {
		var widths []int
		used, end := 0, start
		for end < len(captions) {
			n := min(width, rallyButtonWidth(w, captions[end], padding))
			if end > start && used+gap+n > width {
				break
			}
			widths = append(widths, n)
			used += n + gap
			end++
		}
		w.Row(height).StaticScaled(widths...)
		for i := start; i < end; i++ {
			draw(i)
		}
		start = end
	}
}
