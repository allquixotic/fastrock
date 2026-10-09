package settings

import "slices"

// BoardDisplay is local presentation only; it never changes the Rally query or
// enforces a workflow rule. A pointer in older persisted records distinguishes
// an absent setting from an explicit zero (unlimited WIP / disabled age).
type BoardDisplay struct {
	Density  string `json:"density"`
	ColorBy  string `json:"colorBy"`
	WIPLimit int    `json:"wipLimit"`
	AgeDays  int    `json:"ageDays"`
}

func DefaultBoardDisplay() BoardDisplay {
	return BoardDisplay{Density: "Comfortable", ColorBy: "Work Item", AgeDays: 3}
}

func DisplayOrDefault(d *BoardDisplay) BoardDisplay {
	if d == nil {
		return DefaultBoardDisplay()
	}
	return d.Normalized()
}

func (d BoardDisplay) Normalized() BoardDisplay {
	if d.Density != "Compact" {
		d.Density = "Comfortable"
	}
	if d.ColorBy != "Owner" && d.ColorBy != "Priority" {
		d.ColorBy = "Work Item"
	}
	d.WIPLimit = max(0, min(d.WIPLimit, 100000))
	d.AgeDays = max(0, min(d.AgeDays, 36500))
	return d
}

func (d BoardDisplay) Copy() *BoardDisplay { v := d.Normalized(); return &v }

func (v SavedView) Clone() SavedView {
	v.Filters = slices.Clone(v.Filters)
	v.CardFields = slices.Clone(v.CardFields)
	v.Columns = slices.Clone(v.Columns)
	if v.Display != nil {
		v.Display = v.Display.Copy()
	}
	return v
}
