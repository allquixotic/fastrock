package ui

import (
	"fmt"
	"image"
	"slices"
	"strings"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/rect"
	"github.com/allquixotic/fastrock/internal/rally"
	"golang.org/x/mobile/event/mouse"
)

type detailTab struct{ Key, Label, Field string }

func detailCollectionField(kind, tab string) string {
	switch tab {
	case "Discussions":
		return "Discussion"
	case "Revisions":
		return "RevisionHistory"
	case "Children":
		if kind == "PortfolioItem/Feature" {
			return "UserStories"
		}
		return "Children"
	case "Test Cases":
		return "TestCases"
	}
	return tab
}

func detailTabs(d *detailView) []detailTab {
	tabs := []detailTab{{Key: "Details", Label: "Details"}}
	if !d.New {
		for _, tab := range []detailTab{
			{Key: "Tasks", Label: "Tasks"}, {Key: "Test Cases", Label: "Test Cases"},
			{Key: "Defects", Label: "Defects"}, {Key: "Children", Label: "Children"},
			{Key: "Discussions", Label: "Discussions"}, {Key: "Revisions", Label: "Revision History"},
			{Key: "Attachments", Label: "Attachments"},
		} {
			if d.Kind == "Task" && (tab.Key == "Tasks" || tab.Key == "Children") {
				continue
			}
			tab.Field = detailCollectionField(d.Kind, tab.Key)
			supported := d.Original[tab.Field] != nil
			for _, field := range d.Fields {
				if field.Name == tab.Field && (field.AttributeType == "COLLECTION" || tab.Key == "Revisions") {
					supported = true
				}
			}
			// Discussion is queried by artifact, rather than through a collection
			// reference. Keep it available for supported artifact types.
			if tab.Key == "Discussions" && slices.Contains(rally.ArtifactKinds, d.Kind) {
				supported = true
			}
			if supported {
				tabs = append(tabs, tab)
			}
		}
	}
	return append(tabs, detailTab{Key: "More fields", Label: "More fields"})
}

func (d *detailView) tabLabel(tab detailTab) string {
	if tab.Field == "" {
		return tab.Label
	}
	if n, ok := d.collectionCounts[tab.Key]; ok {
		return fmt.Sprintf("%s (%d)", tab.Label, n)
	}
	if value, ok := d.Original[tab.Field].(map[string]any); ok && value["Count"] != nil {
		return fmt.Sprintf("%s (%d)", tab.Label, d.Original.Count(tab.Field))
	}
	if value, ok := d.Original[tab.Field].([]any); ok {
		return fmt.Sprintf("%s (%d)", tab.Label, len(value))
	}
	return tab.Label + " (?)"
}

func (a *App) drawDetailTabs(w *nucular.Window, d *detailView) {
	tabs := detailTabs(d)
	if !slices.ContainsFunc(tabs, func(tab detailTab) bool { return tab.Key == d.Tab }) {
		d.Tab = "Details"
	}
	scale := w.Master().Style().Scaling
	available := max(1, w.LayoutAvailableWidth())
	for first := 0; first < len(tabs); {
		var widths []int
		used := 0
		for i := first; i < len(tabs); i++ {
			width := min(available, nucular.FontWidth(w.Master().Style().Font, d.tabLabel(tabs[i]))+int(40*scale))
			if len(widths) > 0 && used+width+int(6*scale) > available {
				break
			}
			widths = append(widths, width)
			used += width + int(6*scale)
		}
		w.RowScaled(int(31 * scale)).StaticScaled(widths...)
		for i := range widths {
			tab := tabs[first+i]
			if detailTabButton(w, d.tabLabel(tab), tab.Key, d.Tab == tab.Key, a.p) {
				d.Tab = tab.Key
				if tab.Field != "" {
					a.loadCollection(d)
				}
			}
		}
		first += len(widths)
	}
}

func detailTabButton(w *nucular.Window, text, key string, active bool, p palette) bool {
	b, out := w.Custom(w.CustomState())
	if out == nil {
		return false
	}
	in := w.Input()
	if active {
		out.FillRect(b, 3, p.Selected)
	} else if in.Mouse.HoveringRect(b) {
		out.FillRect(b, 3, p.Hover)
	}
	scale := w.Master().Style().Scaling
	x, y, size := b.X+int(8*scale), b.Y+b.H/2, max(4, int(10*scale))
	// Small drawn marks keep collection identities visible without a symbol font.
	icon := rect.Rect{X: x, Y: y - size/2, W: size, H: size}
	switch key {
	case "Discussions":
		out.FillRect(icon, 2, p.Accent)
		out.StrokeLine(image.Pt(x+2, y+size/2), image.Pt(x, y+size/2+3), 1, p.Accent)
	case "Revisions":
		out.FillCircle(icon, p.Accent)
		out.FillCircle(inset(icon, 2, 2), p.Surface)
		out.StrokeLine(image.Pt(x+size/2, y), image.Pt(x+size/2, y-size/3), 1, p.Accent)
	case "Defects":
		out.FillCircle(icon, p.Danger)
	default:
		out.FillRect(icon, 1, p.Accent)
		for j := range 2 {
			out.StrokeLine(image.Pt(x+2, y-2+j*4), image.Pt(x+size-2, y-2+j*4), 1, p.Surface)
		}
	}
	left := x + size + int(7*scale)
	labelAt(out, rect.Rect{X: left, Y: b.Y, W: max(0, b.X+b.W-left-int(4*scale)), H: b.H}, text, w.Master().Style().Font, p.Text)
	if in.Mouse.HoveringRect(b) {
		w.Tooltip(strings.TrimSpace(text))
	}
	return in.Mouse.Clicked(mouse.ButtonLeft, b)
}
