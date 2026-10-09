package ui

import (
	"fmt"
	"image"
	"math"
	"time"

	"github.com/aarzilli/nucular"
)

func rallyRefreshState(v *rallyView) (busy, enabled bool, updated time.Time) {
	if v == nil || v.Closed || v.Spec.ID == "customviews" {
		return false, false, time.Time{}
	}
	busy = v.Loading || v.Mutating || len(v.PendingCards) > 0
	enabled, updated = !busy, v.Refreshed
	if d := v.Detail; d != nil {
		busy = d.Loading || d.SchemaLoading || d.Saving || d.Pending || d.CollectionLoading
		enabled = enabled && !busy && !d.New && !d.collectionsLoading()
		updated = d.Refreshed
	}
	return
}

func refreshAge(updated, now time.Time) string {
	if updated.IsZero() {
		return "Not yet updated"
	}
	seconds := max(0, int(now.Sub(updated).Seconds()))
	if seconds < 60 {
		return fmt.Sprintf("Updated %d s ago", seconds)
	}
	if seconds < 3600 {
		return fmt.Sprintf("Updated %d min ago", seconds/60)
	}
	if seconds < 86400 {
		return fmt.Sprintf("Updated %d h ago", seconds/3600)
	}
	return fmt.Sprintf("Updated %d d ago", seconds/86400)
}

func (a *App) drawRallyFreshness(w *nucular.Window, v *rallyView) {
	busy, _, updated := rallyRefreshState(v)
	label := refreshAge(updated, time.Now())
	if v.Detail != nil && v.Detail.New {
		label = "New work item"
	}
	if busy {
		label = "Updating… · " + label
		w.Row(22).Ratio(.06, .94)
		b, out := w.Custom(w.CustomState())
		if out != nil {
			phase := int(time.Now().UnixNano() / int64(100*time.Millisecond) % 12)
			x, y := float64(b.X+b.W/2), float64(b.Y+b.H/2)
			radius := float64(min(b.W, b.H)) * .34
			for i := range 12 {
				angle := float64(i) * 2 * math.Pi / 12
				color := a.p.Muted
				if (i-phase+12)%12 < 3 {
					color = a.p.Accent
				}
				point := func(r float64) image.Point { return image.Pt(int(x+r*math.Cos(angle)), int(y+r*math.Sin(angle))) }
				out.StrokeLine(point(radius*.5), point(radius), max(1, b.H/14), color)
			}
		}
	} else {
		w.Row(22).Dynamic(1)
	}
	w.LabelColored(label, "LC", a.p.Muted)
}
