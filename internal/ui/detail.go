package ui

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/font"
	"github.com/allquixotic/fastrock/internal/assistant"
	"github.com/allquixotic/fastrock/internal/rally"
)

type detailView struct {
	Original        rally.Object
	Kind            string
	New             bool
	Tab             string
	Editors         map[string]*nucular.TextEditor
	Rich            map[string]*richEditor
	CommentRich     *richEditor
	ItemRich        map[string]*richEditor
	Fields          []rally.Field
	States          []rally.Object
	Items           []rally.Object
	Loading, Saving bool
	Error           string
	titleFont       font.Face
	titleSize       int
}

func makeDetail(o rally.Object, kind string, isNew bool) *detailView {
	if canonical, ok := rally.CanonicalKind(kind); ok {
		kind = canonical
	}
	d := &detailView{Rich: map[string]*richEditor{}, CommentRich: newRichEditor(""), ItemRich: map[string]*richEditor{}, Original: o.Clone(), Kind: kind, New: isNew, Tab: "Details", Editors: map[string]*nucular.TextEditor{}}
	for _, k := range []string{"Name", "Description", "Notes", "AcceptanceCriteria", "BlockedReason", "PlanEstimate", "Estimate", "ToDo", "Actuals", "Priority", "Severity", "FormattedID", "Owner", "Iteration", "Release", "Project", "Feature", "Parent", "State", "ScheduleState", "Blocked", "Ready", "LastVerdict"} {
		value := o.String(k)
		if k == "State" && o.Ref(k) != "" {
			value = o.Ref(k)
		}
		if k == "Blocked" || k == "Ready" {
			value = strconv.FormatBool(o.Bool(k))
		}
		d.Editors[k] = textEditor(value, k == "Description" || k == "Notes" || k == "AcceptanceCriteria")
	}
	for _, key := range []string{"Description", "Notes", "AcceptanceCriteria"} {
		d.Rich[key] = newRichEditor(o.String(key))
	}
	for k := range o {
		if strings.HasPrefix(k, "c_") {
			d.Editors[k] = textEditor(o.String(k), false)
		}
	}
	return d
}
func (a *App) openArtifact(v *rallyView, o rally.Object) {
	d := makeDetail(o, v.Spec.Kind, false)
	if kind := o.String("_type"); kind != "" {
		d.Kind = kind
	}
	v.Detail = d
	d.Loading = true
	c := a.rallyClient
	a.work(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 30*time.Second)
		defer cancel()
		full, e := c.Get(ctx, o.String("_ref"))
		fields, _ := c.Fields(ctx, d.Kind)
		states, _ := c.All(ctx, "State", rally.Query{})
		var nd *detailView
		if e == nil {
			nd = makeDetail(full, d.Kind, false)
			mergeSchemaEditors(nd, fields)
			nd.States = states
		}
		a.post(func() {
			d.Loading = false
			if e != nil {
				d.Error = e.Error()
				return
			}
			if v.Detail == d {
				v.Detail = nd
			}
		})
	})
}
func (a *App) newArtifact(v *rallyView) {
	o := rally.Object{"Project": map[string]any{"_ref": a.prefs.RallyProject}, "Workspace": map[string]any{"_ref": a.prefs.RallyWorkspace}}
	v.Detail = makeDetail(o, v.Spec.Kind, true)
	d := v.Detail
	c := a.rallyClient
	a.work(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 30*time.Second)
		defer cancel()
		fields, _ := c.Fields(ctx, d.Kind)
		a.post(func() {
			mergeSchemaEditors(d, fields)
			for _, f := range fields {
				if strings.HasPrefix(f.Name, "c_") && d.Editors[f.Name] == nil {
					d.Editors[f.Name] = textEditor("", false)
				}
			}
		})
	})
}
func (a *App) drawDetail(w *nucular.Window, v *rallyView) {
	d := v.Detail
	w.Row(34).Static(100, max(150, w.LayoutAvailableWidth()-410), 100, 100, 100)
	if w.ButtonText("← Back") {
		if d.dirty() {
			a.confirm("Discard changes?", "The open work item contains unsaved edits.", func() { v.Detail = nil })
		} else {
			v.Detail = nil
		}
		return
	}
	w.Label(fallback(d.Original.ID(), "New "+d.Kind), "LC")
	if w.ButtonText("Open web") {
		if ref := d.Original.String("_ref"); ref != "" {
			a.openURL(a.prefs.RallyEndpoint + "/#/detail/" + strings.ToLower(d.Kind) + "/" + d.Original.String("ObjectID"))
		}
	}
	if !d.Saving {
		if primary(w, "Save", a.p) {
			a.saveDetail(v)
		}
	} else {
		w.Label("Saving…", "LC")
	}
	if w.ButtonText("Delete…") && !d.New {
		a.confirm("Delete "+d.Original.ID()+"?", "This deletes the work item from Rally.", func() { a.deleteArtifact(v, d.Original) })
	}
	w.Row(31).Dynamic(7)
	for _, tab := range []string{"Details", "Tasks", "Discussions", "Attachments", "Revisions", "Children", "More fields"} {
		if sectionTab(w, tab, d.Tab == tab, a.p) {
			d.Tab = tab
			if tab != "Details" && tab != "More fields" {
				a.loadCollection(d)
			}
		}
	}
	if d.Error != "" {
		w.Row(45).Dynamic(1)
		w.LabelWrap(d.Error)
	}
	if d.Loading {
		muted(w, "Loading work item…", a.p)
	}
	w.Row(max(150, w.LayoutAvailableHeight()-8)).Dynamic(1)
	if body := w.GroupBegin("detail-body", nucular.WindowNoHScrollbar); body != nil {
		if d.Tab == "Details" || d.Tab == "More fields" {
			a.detailFields(body, d)
		} else {
			a.detailCollection(body, v, d)
		}
		body.GroupEnd()
	}
}
func (a *App) detailFields(w *nucular.Window, d *detailView) {
	structured := d.Kind == "HierarchicalRequirement" || d.Kind == "Task" || d.Kind == "Defect" || strings.HasPrefix(strings.ToLower(d.Kind), "portfolioitem")
	if d.Tab == "More fields" || !structured {
		fields := d.Fields
		if len(fields) == 0 {
			keys := []string{}
			for k := range d.Editors {
				if strings.HasPrefix(k, "c_") {
					keys = append(keys, k)
				}
			}
			sort.Strings(keys)
			for _, k := range keys {
				a.field(w, k, d.Editors[k], false)
			}
		}
		for _, f := range fields {
			if f.AttributeType == "COLLECTION" {
				continue
			}
			if f.ReadOnly {
				if val := d.Original.String(f.Name); val != "" {
					title(w, f.DisplayName, a.p)
					muted(w, val, a.p)
				}
				continue
			}
			ed := d.Editors[f.Name]
			if ed == nil {
				continue
			}
			caption := fallback(f.DisplayName, f.Name)
			if len(f.AllowedValues) > 0 {
				title(w, caption, a.p)
				w.Row(30).Dynamic(1)
				values := append([]string{""}, f.AllowedValues...)
				i := index(values, text(ed))
				next := w.ComboSimple(values, i, 28)
				if next != i {
					setText(ed, values[next])
				}
			} else if f.AttributeType == "BOOLEAN" {
				value := text(ed) == "true"
				w.Row(28).Dynamic(1)
				if w.CheckboxText(caption, &value) {
					setText(ed, strconv.FormatBool(value))
				}
			} else {
				if r := d.Rich[f.Name]; r != nil {
					a.richField(w, caption, r, 220)
				} else {
					a.field(w, caption, ed, f.AttributeType == "TEXT")
				}
			}
		}
		return
	}
	if d.titleSize != a.prefs.FontSize {
		d.titleSize = a.prefs.FontSize
		d.titleFont, _ = font.NewFace(uiRegular, d.titleSize+5)
	}
	oldFont := w.Master().Style().Font
	w.Master().Style().Font = d.titleFont
	w.Row(40).Dynamic(1)
	d.Editors["Name"].Edit(w)
	w.Master().Style().Font = oldFont
	width := w.LayoutAvailableWidth()
	if width < 720 {
		a.detailMetadata(w, d)
		for _, field := range []string{"Description", "AcceptanceCriteria", "Notes"} {
			a.richField(w, field, d.Rich[field], 240)
		}
		return
	}
	w.Row(max(420, w.LayoutAvailableHeight()-6)).Static(width-290, 280)
	if content := w.GroupBegin("artifact-content", nucular.WindowNoHScrollbar); content != nil {
		for _, field := range []string{"Description", "AcceptanceCriteria", "Notes"} {
			a.richField(content, field, d.Rich[field], 260)
		}
		content.GroupEnd()
	}
	if properties := w.GroupBegin("artifact-properties", nucular.WindowNoHScrollbar); properties != nil {
		a.detailMetadata(properties, d)
		properties.GroupEnd()
	}
}
func (a *App) detailMetadata(w *nucular.Window, d *detailView) {
	w.Row(28).Dynamic(1)
	w.Label("State", "LC")
	w.Row(30).Dynamic(1)
	field := rally.StateField(d.Kind)
	stateEditor := d.Editors[field]
	if stateEditor == nil {
		stateEditor = textEditor("", false)
		d.Editors[field] = stateEditor
	}
	if strings.HasPrefix(strings.ToLower(d.Kind), "portfolioitem") {
		names := []string{"No Entry"}
		refs := []string{""}
		for _, s := range d.States {
			names = append(names, s.String("Name"))
			refs = append(refs, s.String("_ref"))
		}
		sel := index(refs, text(stateEditor))
		n := w.ComboSimple(names, sel, 28)
		if n != sel {
			setText(stateEditor, refs[n])
		}
	} else {
		values := rally.States(d.Kind)
		for _, f := range d.Fields {
			if f.Name == field && len(f.AllowedValues) > 0 {
				values = f.AllowedValues
			}
		}
		if current := text(stateEditor); current != "" && !contains(values, current) {
			values = append(values, current)
		}
		i := index(values, text(stateEditor))
		next := w.ComboSimple(values, i, 28)
		if next != i || text(stateEditor) == "" {
			setText(stateEditor, values[next])
		}
	}
	w.Row(28).Dynamic(2)
	blocked := text(d.Editors["Blocked"]) == "true"
	if w.CheckboxText("Blocked", &blocked) {
		setText(d.Editors["Blocked"], strconv.FormatBool(blocked))
	}
	ready := text(d.Editors["Ready"]) == "true"
	if w.CheckboxText("Ready", &ready) {
		setText(d.Editors["Ready"], strconv.FormatBool(ready))
	}
	for _, field := range []struct {
		Name    string
		Objects []rally.Object
	}{{"Owner", a.users}, {"Iteration", a.iterations}, {"Release", a.releases}, {"Project", a.projects}} {
		w.Row(30).Ratio(.22, .78)
		w.Label(field.Name, "LC")
		names := []string{"None"}
		refs := []string{""}
		for _, o := range field.Objects {
			names = append(names, fallback(o.String("DisplayName"), o.String("Name")))
			refs = append(refs, o.String("_ref"))
		}
		ed := d.Editors[field.Name]
		current := text(ed)
		if current == d.Original.String(field.Name) {
			current = d.Original.Ref(field.Name)
		}
		i := index(refs, current)
		next := w.ComboSimple(names, i, 28)
		if next != i {
			setText(ed, refs[next])
		}
	}
	fields := []string{"PlanEstimate"}
	if d.Kind == "Task" {
		fields = []string{"Estimate", "ToDo", "Actuals"}
	}
	if d.Kind == "Defect" {
		fields = append(fields, "Priority", "Severity")
	}
	for _, field := range fields {
		a.field(w, field, d.Editors[field], false)
	}
	a.field(w, "BlockedReason", d.Editors["BlockedReason"], false)

}
func (a *App) field(w *nucular.Window, name string, ed *nucular.TextEditor, multiline bool) {
	if ed == nil {
		return
	}
	switch name {
	case "PlanEstimate":
		name = "Plan estimate"
	case "BlockedReason":
		name = "Blocked reason"
	case "ToDo":
		name = "To do"
	case "AcceptanceCriteria":
		name = "Acceptance criteria"
	}
	title(w, name, a.p)
	height := 30
	if multiline {
		height = 110
	}
	w.Row(height).Dynamic(1)
	ed.Edit(w)
}
func (d *detailView) syncRich() {
	for key, r := range d.Rich {
		if r != nil {
			value := r.html()
			if value != text(d.Editors[key]) {
				setText(d.Editors[key], value)
			}
		}
	}
}
func (d *detailView) dirty() bool {
	d.syncRich()
	for k, e := range d.Editors {
		original := d.Original.String(k)
		if k == "Blocked" || k == "Ready" {
			original = strconv.FormatBool(d.Original.Bool(k))
		}
		if k == "State" && d.Original.Ref(k) != "" {
			original = d.Original.Ref(k)
		}
		if text(e) != original {
			return true
		}
	}
	return false
}
func (d *detailView) changes() (rally.Object, error) {
	d.syncRich()
	result := rally.Object{}
fields:
	for k, ed := range d.Editors {
		value := text(ed)
		original := d.Original.String(k)
		if k == "State" && d.Original.Ref(k) != "" {
			original = d.Original.Ref(k)
		}
		if k == "Blocked" || k == "Ready" {
			original = strconv.FormatBool(d.Original.Bool(k))
		}
		if value == original {
			continue
		}
		var v any = value
		if k == "PlanEstimate" || k == "Estimate" || k == "ToDo" || k == "Actuals" {
			if strings.TrimSpace(value) == "" {
				v = nil
			} else {
				n, e := strconv.ParseFloat(value, 64)
				if e != nil || n < 0 {
					return nil, fmt.Errorf("%s must be a non-negative number", k)
				}
				v = n
			}
		}
		if k == "Blocked" || k == "Ready" {
			v = value == "true"
		}
		if k == "Owner" || k == "Iteration" || k == "Release" || k == "Project" || k == "Feature" || k == "Parent" {
			if value == "" {
				v = nil
			} else if !strings.Contains(value, "/") {
				return nil, fmt.Errorf("select a valid %s reference", k)
			}
		}
		for _, f := range d.Fields {
			if f.Name == k {
				if f.ReadOnly {
					continue fields
				}
				if len(f.AllowedValues) > 0 && value != "" && !contains(f.AllowedValues, value) {
					return nil, fmt.Errorf("%s must be one of %s", k, strings.Join(f.AllowedValues, ", "))
				}
				switch f.AttributeType {
				case "BOOLEAN":
					v = value == "true"
				case "INTEGER", "DECIMAL", "QUANTITY":
					if value == "" {
						v = nil
					} else {
						n, e := strconv.ParseFloat(value, 64)
						if e != nil {
							return nil, fmt.Errorf("%s requires a number", k)
						}
						v = n
					}
				}
			}
		}
		if k != "FormattedID" {
			result[k] = v
		}
	}
	if strings.TrimSpace(text(d.Editors["Name"])) == "" {
		return nil, fmt.Errorf("Name is required")
	}
	for _, f := range d.Fields {
		if f.Required && !f.ReadOnly {
			if ed := d.Editors[f.Name]; ed != nil && strings.TrimSpace(text(ed)) == "" && d.Original[f.Name] == nil {
				return nil, fmt.Errorf("%s is required", f.DisplayName)
			}
		}
	}
	return result, nil
}
func (a *App) saveDetail(v *rallyView) {
	d := v.Detail
	fields, e := d.changes()
	if e != nil {
		d.Error = e.Error()
		return
	}
	if len(fields) == 0 && !d.New {
		a.toast = "No changes to save"
		return
	}
	if d.New {
		fields["Name"] = text(d.Editors["Name"])
		if a.prefs.RallyProject != "" {
			fields["Project"] = a.prefs.RallyProject
		}
		if a.prefs.RallyWorkspace != "" {
			fields["Workspace"] = a.prefs.RallyWorkspace
		}
	}
	d.Saving = true
	c := a.rallyClient
	a.work(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 30*time.Second)
		defer cancel()
		var saved rally.Object
		var e error
		if d.New {
			saved, e = c.Create(ctx, d.Kind, fields)
		} else {
			current, err := c.Get(ctx, d.Original.String("_ref"))
			if err != nil {
				e = err
			} else if stamp := d.Original.String("LastUpdateDate"); stamp != "" && stamp != current.String("LastUpdateDate") {
				e = fmt.Errorf("This item changed on Rally. Reopen it to review the current version before saving")
			} else {
				saved, e = c.Update(ctx, d.Original.String("_ref"), d.Kind, fields)
			}
		}
		a.post(func() {
			d.Saving = false
			if e != nil {
				d.Error = e.Error()
				return
			}
			v.Detail = makeDetail(saved, d.Kind, false)
			mergeSchemaEditors(v.Detail, d.Fields)
			v.Detail.States = d.States
			a.toast = "Saved " + saved.ID()
			a.refreshRally(v)
		})
	})
}
func (a *App) loadCollection(d *detailView) {
	if d.New {
		return
	}
	c := a.rallyClient
	tab := d.Tab
	d.Loading = true
	d.Items = nil
	a.work(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 30*time.Second)
		defer cancel()
		var items []rally.Object
		var e error
		ref := d.Original.String("_ref")
		switch tab {
		case "Discussions":
			items, e = c.All(ctx, "ConversationPost", rally.Query{Expression: rally.Eq("Artifact", ref), Order: "CreationDate DESC"})
		case "Revisions":
			history, err := c.Get(ctx, d.Original.Ref("RevisionHistory"))
			if err != nil {
				e = err
			} else {
				var p rally.Page
				p, e = c.Collection(ctx, history.Ref("Revisions"), rally.Query{PageSize: 2000})
				items = p.Results
			}
		default:
			field := tab
			if tab == "Children" {
				field = "Children"
			}
			collection := d.Original.Ref(field)
			if collection != "" {
				var p rally.Page
				p, e = c.Collection(ctx, collection, rally.Query{PageSize: 2000})
				items = p.Results
			}
		}
		a.post(func() {
			d.Loading = false
			if d.Tab == tab {
				d.Items = items
				if e != nil {
					d.Error = e.Error()
				}
			}
		})
	})
}
func (a *App) detailCollection(w *nucular.Window, v *rallyView, d *detailView) {
	if d.Tab == "Discussions" {
		a.richField(w, "Add to discussion", d.CommentRich, 140)
		w.Row(28).Static(140)
		if primary(w, "Add comment", a.p) && strings.TrimSpace(string(d.CommentRich.doc.Text)) != "" {
			c := a.rallyClient
			comment := d.CommentRich.html()
			a.work(func() {
				ctx, cancel := context.WithTimeout(a.ctx, 30*time.Second)
				defer cancel()
				_, e := c.Create(ctx, "ConversationPost", rally.Object{"Artifact": d.Original.String("_ref"), "Text": comment})
				a.post(func() {
					if e != nil {
						d.Error = e.Error()
					} else {
						d.CommentRich = newRichEditor("")
						a.loadCollection(d)
					}
				})
			})
		}
	}
	if d.Tab == "Attachments" {
		w.Row(30).Static(180)
		if w.ButtonText("Add attachment…") {
			a.choosePath(false, false, func(path string) {
				c := a.rallyClient
				a.work(func() {
					var err error
					info, e := os.Stat(path)
					if e != nil {
						err = e
					} else if info.Size() > 5<<20 {
						err = fmt.Errorf("Attachment exceeds 5 MiB")
					} else {
						data, e := os.ReadFile(path)
						if e != nil {
							err = e
						} else {
							ctx, cancel := context.WithTimeout(a.ctx, 45*time.Second)
							defer cancel()
							_, err = c.Upload(ctx, d.Original.String("_ref"), filepath.Base(path), "application/octet-stream", data)
						}
					}
					a.post(func() {
						if err != nil {
							d.Error = err.Error()
						} else {
							a.loadCollection(d)
						}
					})
				})
			})
		}
	}
	if d.Tab == "Tasks" {
		w.Row(28).Static(140)
		if w.ButtonText("Add task…") {
			a.inputDialog("New task", "", func(name string) {
				c := a.rallyClient
				a.work(func() {
					ctx, cancel := context.WithTimeout(a.ctx, 30*time.Second)
					defer cancel()
					_, e := c.Create(ctx, "Task", rally.Object{"Name": name, "WorkProduct": d.Original.String("_ref")})
					a.post(func() {
						if e != nil {
							d.Error = e.Error()
						} else {
							a.loadCollection(d)
						}
					})
				})
			})
		}
	}
	for _, o := range d.Items {
		if d.Tab == "Discussions" || d.Tab == "Revisions" {
			title(w, fallback(o.String("User"), o.String("CreationDate")), a.p)
			id := o.String("_ref")
			r := d.ItemRich[id]
			if r == nil {
				r = newRichEditor(fallback(o.String("Text"), o.String("Description")))
				r.mode = "Preview"
				d.ItemRich[id] = r
			}
			a.richField(w, "", r, 120)
		} else {
			w.Row(33).Dynamic(1)
			if w.ButtonText(o.ID() + "  " + o.String("Name")) {
				if d.Tab == "Attachments" {
					a.downloadAttachment(o)
				} else {
					a.openArtifact(v, o)
				}
			}
		}
	}
	if len(d.Items) == 0 && !d.Loading {
		muted(w, "No "+strings.ToLower(d.Tab)+" on this work item.", a.p)
	}
}
func (a *App) bulk(v *rallyView, fields rally.Object) {
	p := assistant.Plan{Summary: "Update selected work items"}
	for _, o := range v.Items {
		if v.Selected[o.String("_ref")] {
			p.Changes = append(p.Changes, assistant.Change{Operation: "update", Kind: v.Spec.Kind, Ref: o.String("_ref"), Fields: fields, Before: o.Clone()})
		}
	}
	a.applyPlan(v, p)
}
func (a *App) deleteSelected(v *rallyView) {
	a.confirm("Delete selected work items?", "Deletion is applied to Rally and cannot be undone here.", func() {
		p := assistant.Plan{Summary: "Delete selected work items"}
		for _, o := range v.Items {
			if v.Selected[o.String("_ref")] {
				p.Changes = append(p.Changes, assistant.Change{Operation: "delete", Kind: v.Spec.Kind, Ref: o.String("_ref"), Before: o.Clone()})
			}
		}
		a.applyPlan(v, p)
	})
}
func (a *App) deleteArtifact(v *rallyView, o rally.Object) {
	a.applyPlan(v, assistant.Plan{Summary: "Delete " + o.ID(), Changes: []assistant.Change{{Operation: "delete", Kind: v.Spec.Kind, Ref: o.String("_ref"), Before: o.Clone()}}})
	v.Detail = nil
}
func (a *App) applyPlan(v *rallyView, p assistant.Plan) {
	c := a.rallyClient
	a.work(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 2*time.Minute)
		defer cancel()
		n, e := assistant.Apply(ctx, c, p)
		a.post(func() {
			if e != nil {
				a.toast = fmt.Sprintf("Applied %d of %d changes: %v. Refresh before retrying.", n, len(p.Changes), e)
			} else {
				a.toast = fmt.Sprintf("Applied %d changes", n)
			}
			a.refreshRally(v)
		})
	})
}

func mergeSchemaEditors(d *detailView, fields []rally.Field) {
	d.Fields = fields
	for _, f := range fields {
		if !f.ReadOnly && f.AttributeType == "TEXT" && d.Rich[f.Name] == nil {
			d.Rich[f.Name] = newRichEditor(d.Original.String(f.Name))
		}
		if f.ReadOnly || f.AttributeType == "COLLECTION" || d.Editors[f.Name] != nil {
			continue
		}
		value := d.Original.String(f.Name)
		if f.AttributeType == "OBJECT" {
			value = d.Original.Ref(f.Name)
		}
		d.Editors[f.Name] = textEditor(value, f.AttributeType == "TEXT")
	}
}
