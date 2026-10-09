package ui

import (
	"github.com/allquixotic/fastrock/internal/desktop"
)

// An absent catalog value remains an explicit choice until the user changes it.
func picker(w *desktop.Window, current *string, values, labels []string, empty string, locked bool) {
	values = append([]string{""}, values...)
	labels = append([]string{empty}, labels...)
	selected := 0
	for i, value := range values {
		if value == *current {
			selected = i
			break
		}
	}
	if *current != "" && selected == 0 {
		selected = len(values)
		values = append(values, *current)
		labels = append(labels, *current+" (unavailable)")
	}
	next := w.ComboSimple(labels, selected, 28)
	if next != selected && !locked && next >= 0 && next < len(values) {
		*current = values[next]
	}
}
func (a *App) modelPickers(w *desktop.Window, model, effort, tier *string, locked bool) {
	var values, labels []string
	for _, m := range a.catalog.Models {
		if m.Hidden && m.Model != *model {
			continue
		}
		values = append(values, m.Model)
		labels = append(labels, m.Name)
	}
	picker(w, model, values, labels, "Configured model", locked)
	m, _ := a.catalog.Find(fallback(*model, a.catalog.DefaultModel()))
	values, labels = nil, nil
	for _, e := range m.Efforts {
		values = append(values, e.ID)
		labels = append(labels, e.ID)
	}
	picker(w, effort, values, labels, "Default effort", locked)
	values, labels = nil, nil
	for _, t := range a.catalog.Speeds(m) {
		values = append(values, t.ID)
		labels = append(labels, t.Name)
	}
	picker(w, tier, values, labels, "Configured speed", locked)
}
func setModelParams(params map[string]any, model, effort, tier string) {
	if model != "" {
		params["model"] = model
	}
	if effort != "" {
		params["effort"] = effort
	}
	if tier != "" {
		params["serviceTier"] = tier
	}
}
