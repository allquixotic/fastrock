package ui

import (
	"fmt"
	"slices"
	"strconv"
	"strings"

	"github.com/aarzilli/nucular"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/settings"
)

const maxRallyFilters = 64

var rallyFilterFields = []struct{ key, label, kind string }{
	{"Iteration", "Iteration", "Iteration"},
	{"Release", "FY Quarter", "Release"},
	{"Project", "Team", "Project"},
	{"Tags", "Tags", "Tag"},
	{"Type", "Type", ""},
}

var rallyFilterOperators = []string{"is", "is not", "contains"}

type rallyFilterDraft struct {
	Field, Operator, Label string
	Value                  *nucular.TextEditor
	Picker                 *detailView
	Error                  string
}

type rallyFilterDraftState struct {
	Field, Operator, Value, Label string
	Position                      editorPosition
}

func (d *rallyFilterDraft) snapshot() *rallyFilterDraftState {
	if d == nil {
		return nil
	}
	return &rallyFilterDraftState{d.Field, d.Operator, text(d.Value), d.Label, position(d.Value)}
}

func (d *rallyFilterDraft) matches(s *rallyFilterDraftState) bool {
	if d == nil || s == nil {
		return d == nil && s == nil
	}
	return d.Field == s.Field && d.Operator == s.Operator && text(d.Value) == s.Value && d.Label == s.Label && position(d.Value) == s.Position
}

func (s *rallyFilterDraftState) restore() *rallyFilterDraft {
	if s == nil {
		return nil
	}
	d := newRallyFilterDraft(s.Field, s.Operator)
	d.Label = s.Label
	setText(d.Value, s.Value)
	s.Position.apply(d.Value)
	return d
}

func newRallyFilterDraft(field, operator string) *rallyFilterDraft {
	d := &rallyFilterDraft{Field: field, Operator: operator, Value: textEditor("", false)}
	d.Value.Placeholder = "Name contains…"
	d.Value.Flags |= nucular.EditSigEnter
	return d
}

func (d *rallyFilterDraft) filter() settings.RallyFilter {
	return settings.RallyFilter{Field: d.Field, Operator: d.Operator, Value: text(d.Value), Label: d.Label}
}

func (d *rallyFilterDraft) closePicker() {
	if d != nil && d.Picker != nil {
		if p := d.Picker.referencePicker; p != nil {
			p.close()
		}
		d.Picker = nil
	}
}

func (v *rallyView) resetFilterDraft(field, operator string) {
	v.FilterDraft.closePicker()
	v.FilterDraft = newRallyFilterDraft(field, operator)
}

func rallyFilterField(field string) (label, kind string, ok bool) {
	for _, f := range rallyFilterFields {
		if f.key == field {
			return f.label, f.kind, true
		}
	}
	return "", "", false
}

// Exact reference comparisons use IDs, including collection membership for
// Tags. Name substrings use the documented nested Name attribute. Type is
// constrained by the page endpoint or its ArtifactTypes; never invent _type.
func (a *App) rallyFilterClause(v *rallyView, f settings.RallyFilter) (string, error) {
	label, kind, ok := rallyFilterField(f.Field)
	if !ok || !slices.Contains(rallyFilterOperators, f.Operator) {
		return "", fmt.Errorf("Unsupported filter field or operator. Remove this filter and add it again.")
	}
	if strings.TrimSpace(f.Value) == "" {
		return "", fmt.Errorf("Choose a value for %s.", label)
	}
	if f.Field == "Type" {
		types := v.Spec.ArtifactTypes()
		if len(types) == 0 {
			types = []string{v.Spec.Kind}
		}
		match := false
		for _, kind := range types {
			matched, err := rallyTypeMatches(kind, f)
			if err != nil {
				return "", err
			}
			match = match || matched
		}
		if !match {
			return "(ObjectID = 0)", nil
		}
		return "", nil
	}
	if f.Operator == "contains" {
		return "(" + f.Field + ".Name contains " + rally.Quote(f.Value) + ")", nil
	}
	if a.rallyClient == nil {
		return "", fmt.Errorf("Connect to Rally before applying reference filters.")
	}
	actual, valid := a.rallyClient.ReferenceKind(f.Value)
	if !valid || actual != kind {
		return "", fmt.Errorf("Choose %s from this Rally connection.", label)
	}
	operator := "="
	if f.Operator == "is not" {
		operator = "!="
	}
	if f.Field == "Tags" {
		operator = "contains"
		if f.Operator == "is not" {
			operator = "!contains"
		}
	}
	return "(" + f.Field + " " + operator + " " + rally.Quote(f.Value) + ")", nil
}

func (a *App) structuredFilterExpression(v *rallyView) (string, error) {
	if len(v.StructuredFilters) > maxRallyFilters {
		return "", fmt.Errorf("Remove filters until at most %d remain.", maxRallyFilters)
	}
	expression := ""
	for _, f := range v.StructuredFilters {
		clause, err := a.rallyFilterClause(v, f)
		if err != nil {
			return "", err
		}
		expression = rally.And(expression, clause)
	}
	return expression, nil
}

func (v *rallyView) structuredFilterSignature() string {
	var b strings.Builder
	for _, f := range v.StructuredFilters {
		// Quoting preserves boundaries even for embedded NULs or separators.
		b.WriteString(strconv.Quote(f.Field))
		b.WriteString(strconv.Quote(f.Operator))
		b.WriteString(strconv.Quote(f.Value))
	}
	return b.String()
}

func (a *App) addRallyFilter(v *rallyView) bool {
	d := v.FilterDraft
	if d == nil || v.Closed {
		return false
	}
	f := d.filter()
	if _, err := a.rallyFilterClause(v, f); err != nil {
		d.Error = err.Error()
		return false
	}
	if len(v.StructuredFilters) >= maxRallyFilters {
		d.Error = fmt.Sprintf("At most %d filters can be applied.", maxRallyFilters)
		return false
	}
	if !slices.ContainsFunc(v.StructuredFilters, func(old settings.RallyFilter) bool {
		return old.Field == f.Field && old.Operator == f.Operator && old.Value == f.Value
	}) {
		v.StructuredFilters = append(v.StructuredFilters, f)
	}
	v.filterValid = false
	v.resetFilterDraft(d.Field, d.Operator)
	a.refreshRally(v)
	return true
}

func (v *rallyView) clearRallyFilters() {
	setText(v.Query, "")
	v.QueryApplied = ""
	setText(v.Search, "")
	v.Timebox, v.OwnerFilter, v.StateFilter = "", "", ""
	v.TimeboxName, v.ReleaseTimebox, v.ReleaseName = "", "", ""
	v.CurrentIteration, v.OnlyBlocked, v.OnlyReady = false, false, false
	v.StructuredFilters = nil
	v.resetFilterDraft("Iteration", "is")
	v.filterValid = false
}

func (a *App) chooseRallyFilter(v *rallyView) *referencePicker {
	d := v.FilterDraft
	if d == nil || d.Operator == "contains" || v.Closed {
		return nil
	}
	label, kind, ok := rallyFilterField(d.Field)
	if !ok || kind == "" || a.rallyClient == nil {
		return nil
	}
	d.closePicker()
	// Reuse the paged, typed reference chooser without making it the current
	// work item editor. The value and lifecycle remain owned by this draft.
	d.Picker = &detailView{Editors: map[string]*nucular.TextEditor{d.Field: d.Value}}
	p := a.openReferencePicker(d.Picker, rally.Field{Name: d.Field, DisplayName: label, AttributeType: "OBJECT", ReferenceType: kind})
	if p == nil {
		return nil
	}
	scope := a.inlineScope()
	p.current = func() bool { return !v.Closed && v.FilterDraft == d && a.inlineScope() == scope }
	p.selected = func(o rally.Object) { d.Label, d.Error = referenceLabel(o), "" }
	return p
}

func (a *App) drawRallyFilterBuilder(w *nucular.Window, v *rallyView) {
	if v.FilterDraft == nil {
		v.resetFilterDraft("Iteration", "is")
	}
	d := v.FilterDraft
	muted(w, "Match all filters", a.p)
	var fields, labels []string
	for _, f := range rallyFilterFields {
		fields, labels = append(fields, f.key), append(labels, f.label)
	}
	w.Row(30).Ratio(.55, .45)
	if next := w.ComboSimple(labels, index(fields, d.Field), 28); fields[next] != d.Field {
		v.resetFilterDraft(fields[next], d.Operator)
		d = v.FilterDraft
	}
	if next := w.ComboSimple(rallyFilterOperators, index(rallyFilterOperators, d.Operator), 28); rallyFilterOperators[next] != d.Operator {
		v.resetFilterDraft(d.Field, rallyFilterOperators[next])
		d = v.FilterDraft
	}
	var committed bool
	if d.Operator == "contains" {
		w.Row(30).Ratio(.75, .25)
		committed = d.Value.Edit(w)&nucular.EditCommitted != 0
		if d.Value.Active {
			v.cardFocusActive = false
		}
	} else if d.Field == "Type" {
		kinds := slices.Clone(rally.ArtifactKinds)
		if !slices.Contains(kinds, v.Spec.Kind) {
			kinds = append(kinds, v.Spec.Kind)
		}
		choices := []string{"Choose type…"}
		values := append([]string{""}, kinds...)
		for _, kind := range kinds {
			choices = append(choices, rallyKindLabel(kind))
		}
		w.Row(30).Ratio(.75, .25)
		if next := w.ComboSimple(choices, index(values, text(d.Value)), 28); values[next] != text(d.Value) {
			setText(d.Value, values[next])
			d.Label = choices[next]
		}
	} else {
		w.Row(30).Ratio(.5, .25, .25)
		w.Label(fallback(d.Label, "Choose a value…"), "LC")
		if d.Label != "" && w.Input().Mouse.HoveringRect(w.LastWidgetBounds) {
			w.Tooltip(d.Label)
		}
		if enabledButton(w, "Choose…", a.rallyClient != nil, false, a.p) {
			a.chooseRallyFilter(v)
		}
	}
	add := w.ButtonText("Add filter")
	if add || committed {
		a.addRallyFilter(v)
	}
	if d.Error != "" {
		w.Row(44).Dynamic(1)
		w.LabelWrap(d.Error)
	}
}

func (a *App) drawRallyAdvancedQuery(w *nucular.Window, v *rallyView) {
	w.Row(30).Ratio(.68, .14, .18)
	event := v.Query.Edit(w)
	apply := w.ButtonText("Apply")
	if apply || event&nucular.EditCommitted != 0 {
		v.QueryApplied = text(v.Query)
		a.refreshRally(v)
	}
	if w.ButtonText("Clear all") {
		v.clearRallyFilters()
		a.refreshRally(v)
	}
	muted(w, `WSAPI query, e.g. (Blocked = true) or (Owner.UserName = "name@example.com")`, a.p)
}
