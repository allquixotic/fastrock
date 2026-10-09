package ui

import (
	"fmt"
	"slices"
	"sort"
	"strconv"
	"strings"

	"github.com/aarzilli/nucular"
	"github.com/allquixotic/fastrock/internal/rally"
)

func detailCaption(f rally.Field) string {
	if f.DisplayName != "" {
		return f.DisplayName
	}
	if f.Name == "DisplayColor" {
		return "Color"
	}
	if name := map[string]string{"Project": "Team", "Release": "FY Quarter", "ScheduleState": "Schedule State", "PlanEstimate": "Plan Estimate", "StepsToReproduce": "Steps to Reproduce", "AcceptanceCriteria": "Acceptance Criteria", "LastUpdateDate": "Updated", "CreationDate": "Created", "SubmittedBy": "Submitted By", "BlockedReason": "Blocked reason", "ToDo": "To Do", "TaskRollup": "Task Roll-up"}[f.Name]; name != "" {
		return name
	}
	return f.Name
}

func (d *detailView) schemaField(name string) (rally.Field, bool) {
	for _, f := range d.Fields {
		if f.Name == name {
			return f, true
		}
	}
	return rally.Field{Name: name}, false
}

func (d *detailView) richFields() []rally.Field {
	var fields []rally.Field
	seen := map[string]bool{}
	add := func(name string) {
		if d.Rich[name] == nil || seen[name] {
			return
		}
		f, _ := d.schemaField(name)
		fields, seen[name] = append(fields, f), true
	}
	for _, name := range []string{"Description", "StepsToReproduce", "AcceptanceCriteria", "Notes"} {
		add(name)
	}
	var custom []string
	for name := range d.Rich {
		if !seen[name] {
			custom = append(custom, name)
		}
	}
	sort.Slice(custom, func(i, j int) bool {
		a, _ := d.schemaField(custom[i])
		b, _ := d.schemaField(custom[j])
		return detailCaption(a) < detailCaption(b)
	})
	for _, name := range custom {
		add(name)
	}
	return fields
}

func (d *detailView) propertyFields() []rally.Field {
	var fields []rally.Field
	seen := map[string]bool{}
	add := func(name string) {
		f, exists := d.schemaField(name)
		if seen[name] || d.Rich[name] != nil || !exists && d.Original[name] == nil && (len(d.Fields) > 0 || d.Editors[name] == nil) {
			return
		}
		fields, seen[name] = append(fields, f), true
	}
	for _, name := range []string{"DisplayColor", "Owner", "Project", rally.StateField(d.Kind), "FlowState", "PlanEstimate", "Estimate", "ToDo", "Actuals", "Priority"} {
		add(name)
	}
	if d.Kind != "Task" && (d.Original["TaskStatus"] != nil || d.Original["Tasks"] != nil) {
		fields = append(fields, rally.Field{Name: "TaskRollup", ReadOnly: true})
	}
	for _, name := range []string{"Feature", "Parent", "WorkProduct", "Requirement", "PortfolioItem", "Severity", "Tags", "Release", "Iteration", "Milestones", "Expedite"} {
		add(name)
	}
	var custom []rally.Field
	for _, f := range d.Fields {
		if strings.HasPrefix(f.Name, "c_") && !seen[f.Name] && d.Rich[f.Name] == nil {
			custom = append(custom, f)
		}
	}
	sort.SliceStable(custom, func(i, j int) bool { return detailCaption(custom[i]) < detailCaption(custom[j]) })
	fields = append(fields, custom...)
	for _, name := range []string{"CreationDate", "LastUpdateDate", "SubmittedBy"} {
		add(name)
		if len(fields) > 0 && fields[len(fields)-1].Name == name {
			fields[len(fields)-1].ReadOnly = true
		}
	}
	return fields
}

func detailPaneWidths(width int, scale float64, gap int) (content, properties int, stacked bool) {
	properties = max(int(300*scale), (width-gap)*28/100)
	content = width - gap - properties
	return content, properties, content < int(360*scale)
}

func (a *App) detailFields(w *nucular.Window, d *detailView) {
	structured := d.Kind == "HierarchicalRequirement" || d.Kind == "Task" || d.Kind == "Defect" || strings.HasPrefix(strings.ToLower(d.Kind), "portfolioitem")
	if d.Tab == "More fields" || !structured {
		if len(d.Fields) == 0 {
			var names []string
			for name := range d.Editors {
				if strings.HasPrefix(name, "c_") {
					names = append(names, name)
				}
			}
			sort.Strings(names)
			for _, name := range names {
				a.field(w, name, d.Editors[name], false)
			}
		}
		for _, f := range d.Fields {
			a.detailProperty(w, d, f)
		}
		return
	}
	scale := w.Master().Style().Scaling
	titleSize := int(float64(a.prefs.FontSize+7) * scale)
	if d.titleFont.Face == nil || d.titleSize != titleSize {
		d.titleSize, d.titleFont = titleSize, typeFace(titleSize, boldFont)
	}
	oldFont := w.Master().Style().Font
	w.Master().Style().Font = d.titleFont
	w.Row(40).Dynamic(1)
	d.Editors["Name"].Edit(w)
	w.Master().Style().Font = oldFont
	content, properties, stacked := detailPaneWidths(w.LayoutAvailableWidth(), scale, w.WindowStyle().Spacing.X)
	if stacked {
		a.detailRichContent(w, d)
		a.detailMetadata(w, d)
		return
	}
	w.RowScaled(max(int(420*scale), w.LayoutAvailableHeight()-int(6*scale))).StaticScaled(content, properties)
	if body := w.GroupBegin("artifact-content", nucular.WindowNoHScrollbar); body != nil {
		a.detailRichContent(body, d)
		body.GroupEnd()
	}
	if sidebar := w.GroupBegin("artifact-properties", nucular.WindowNoHScrollbar); sidebar != nil {
		a.detailMetadata(sidebar, d)
		sidebar.GroupEnd()
	}
}

func (a *App) detailRichContent(w *nucular.Window, d *detailView) {
	for _, f := range d.richFields() {
		height := 240
		if f.Name == "Description" {
			height = 360
		}
		a.richField(w, detailCaption(f), d.Rich[f.Name], height)
	}
}

func (a *App) detailMetadata(w *nucular.Window, d *detailView) {
	statusDrawn := false
	for _, f := range d.propertyFields() {
		a.detailProperty(w, d, f)
		if f.Name == rally.StateField(d.Kind) {
			a.detailStatus(w, d)
			statusDrawn = true
		}
	}
	if !statusDrawn {
		a.detailStatus(w, d)
	}
}

func (a *App) detailStatus(w *nucular.Window, d *detailView) {
	w.Row(28).Dynamic(2)
	for _, choice := range []struct{ key, label string }{{"Blocked", "⬟ Blocked"}, {"Ready", "✔ Ready"}} {
		value := text(d.Editors[choice.key]) == "true"
		tone := a.p.Danger
		if choice.key == "Ready" {
			tone = a.p.Success
		}
		if detailStatusButton(w, choice.label, value, tone, a.p) {
			setText(d.Editors[choice.key], strconv.FormatBool(!value))
		}
	}
	if text(d.Editors["Blocked"]) == "true" {
		f, _ := d.schemaField("BlockedReason")
		a.detailProperty(w, d, f)
	}
}

func (a *App) detailProperty(w *nucular.Window, d *detailView, f rally.Field) {
	caption := detailCaption(f)
	if editableCollection(f) {
		a.detailReferenceCollection(w, d, f)
		return
	}
	if f.Name == "DisplayColor" && !f.ReadOnly {
		a.detailColor(w, d, f)
		return
	}
	if f.ReadOnly || f.AttributeType == "COLLECTION" || f.Name == "TaskRollup" {
		value := d.Original.String(f.Name)
		if f.Name == "TaskRollup" {
			value = fmt.Sprintf("%d tasks · %s · %g h remaining", d.Original.Count("Tasks"), fallback(d.Original.String("TaskStatus"), "No status"), d.Original.Number("TaskRemainingTotal"))
		} else if f.AttributeType == "COLLECTION" {
			value = fmt.Sprintf("%d items", d.Original.Count(f.Name))
		}
		if value != "" {
			title(w, caption, a.p)
			w.Row(40).Dynamic(1)
			w.LabelWrap(value)
		}
		return
	}
	if r := d.Rich[f.Name]; r != nil {
		a.richField(w, caption, r, 240)
		return
	}
	ed := d.Editors[f.Name]
	if ed == nil {
		return
	}
	if f.Name == "State" && strings.HasPrefix(strings.ToLower(d.Kind), "portfolioitem") {
		names, refs := []string{"No Entry"}, []string{""}
		for _, state := range d.States {
			names, refs = append(names, state.String("Name")), append(refs, state.String("_ref"))
		}
		title(w, caption, a.p)
		w.Row(30).Dynamic(1)
		current := index(refs, text(ed))
		if next := w.ComboSimple(names, current, 28); next != current {
			setText(ed, refs[next])
		}
		return
	}
	if d.referenceField(f.Name) && !(f.Name == "State" && f.AttributeType != "OBJECT") {
		a.detailReference(w, d, f)
		return
	}
	if len(f.AllowedValues) > 0 {
		values := append([]string{""}, f.AllowedValues...)
		if current := text(ed); current != "" && !slices.Contains(values, current) {
			values = append(values, current)
		}
		title(w, caption, a.p)
		w.Row(30).Dynamic(1)
		current := index(values, text(ed))
		if next := w.ComboSimple(values, current, 28); next != current {
			setText(ed, values[next])
		}
		return
	}
	if f.AttributeType == "BOOLEAN" {
		value := text(ed) == "true"
		w.Row(28).Dynamic(1)
		if w.CheckboxText(caption, &value) {
			setText(ed, strconv.FormatBool(value))
		}
		return
	}
	a.field(w, caption, ed, f.AttributeType == "TEXT")
}
