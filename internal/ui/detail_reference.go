package ui

import (
	"context"
	"fmt"
	"slices"
	"strconv"
	"strings"
	"time"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/rally"
	"golang.org/x/mobile/event/key"
)

func (d *detailView) referenceField(name string) bool {
	f, _ := d.schemaField(name)
	if f.AttributeType == "OBJECT" {
		return true
	}
	return isReferenceField(name) && (name != "State" || strings.HasPrefix(strings.ToLower(d.Kind), "portfolioitem"))
}

func (d *detailView) originalValue(name string) string {
	if f, ok := d.schemaField(name); ok && f.AttributeType == "BOOLEAN" {
		return strconv.FormatBool(d.Original.Bool(name))
	}
	if f, ok := d.schemaField(name); ok && editableCollection(f) {
		value, _ := collectionRefsValue(d.Original[name])
		return value
	}
	if d.referenceField(name) && d.Original.Ref(name) != "" {
		return d.Original.Ref(name)
	}
	return d.Original.String(name)
}

func referenceKinds(d *detailView, f rally.Field) []string {
	if k, ok := rally.CanonicalKind(strings.ReplaceAll(f.ReferenceType, " ", "")); ok {
		return []string{k}
	}
	switch f.Name {
	case "Tags":
		return []string{"Tag"}
	case "Milestones":
		return []string{"Milestone"}
	case "Owner", "SubmittedBy":
		return []string{"User"}
	case "Project", "Iteration", "Release":
		return []string{f.Name}
	case "Feature":
		return []string{"PortfolioItem/Feature"}
	case "Requirement":
		return []string{"HierarchicalRequirement"}
	case "WorkProduct":
		return []string{"HierarchicalRequirement", "Defect", "TestSet", "DefectSuite"}
	case "PortfolioItem":
		return []string{"PortfolioItem/Feature", "PortfolioItem/Epic"}
	case "Parent":
		if d.Kind == "HierarchicalRequirement" {
			return []string{"HierarchicalRequirement"}
		}
		if d.Kind == "PortfolioItem/Feature" {
			return []string{"PortfolioItem/Epic"}
		}
	}
	return nil
}

type referencePicker struct {
	hideInlineStatus bool
	current          func() bool
	selected         func(rally.Object)
	detail           *detailView
	field            rally.Field
	editor           *desktop.TextEditor
	editorRevision   uint64
	client           *rally.Client
	workspace        string
	project          string
	parents          bool
	children         bool
	kinds            []string
	kind             int
	search           *desktop.TextEditor
	appliedSearch    string
	items            []rally.Object
	start, total     int
	generation       uint64
	loading, closed  bool
	err              string
	cancel           context.CancelFunc
}

func (p *referencePicker) close() {
	p.closed = true
	p.generation++
	if p.cancel != nil {
		p.cancel()
	}
	if p.detail.referencePicker == p {
		p.detail.referencePicker = nil
	}
	p.items = nil
}

func (a *App) detailReference(w *desktop.Window, d *detailView, f rally.Field) {
	title(w, detailCaption(f), a.p)
	value := text(d.Editors[f.Name])
	label := "None"
	if value != "" {
		label = "Selected item"
		if value == d.Original.Ref(f.Name) {
			switch original := d.Original[f.Name].(type) {
			case map[string]any:
				label = fallback(referenceLabel(rally.Object(original)), label)
			case rally.Object:
				label = fallback(referenceLabel(original), label)
			}
		}
		if selected := d.referenceLabels[f.Name+"\x00"+value]; selected != "" {
			label = selected
		}
	}
	w.Row(30).Ratio(.58, .25, .17)
	w.Label(cut(label, 48), "LC")
	if value != "" && w.Input().Mouse.HoveringRect(w.LastWidgetBounds) {
		w.Tooltip(label)
	}
	if enabledButton(w, "Choose…", a.rallyClient != nil && len(referenceKinds(d, f)) > 0, false, a.p) {
		a.openReferencePicker(d, f)
	}
	if enabledButton(w, "Clear", value != "" && !f.Required, false, a.p) {
		setText(d.Editors[f.Name], "")
	}
}

func (a *App) openReferencePicker(d *detailView, f rally.Field) *referencePicker {
	p := a.newReferencePicker(d, f)
	if p != nil && a.window != nil {
		a.window.PopupOpen("Choose "+detailCaption(f), desktop.WindowTitle|desktop.WindowClosable, a.modalBounds(650, 650), false, func(w *desktop.Window) { a.drawReferencePicker(w, p) })
	}
	return p
}

func (a *App) newReferencePicker(d *detailView, f rally.Field) *referencePicker {
	if a.rallyClient == nil || d.Editors[f.Name] == nil || f.ReadOnly {
		return nil
	}
	kinds := referenceKinds(d, f)
	if len(kinds) == 0 {
		return nil
	}
	if d.referencePicker != nil {
		d.referencePicker.close()
	}
	p := &referencePicker{detail: d, field: f, editor: d.Editors[f.Name], editorRevision: d.Editors[f.Name].TextRevision(), client: a.rallyClient, workspace: a.prefs.RallyWorkspace, project: a.prefs.RallyProject, parents: a.prefs.ProjectParents, children: a.prefs.ProjectChildren, kinds: kinds, search: textEditor("", false)}
	p.search.Placeholder = "Search name or ID"
	d.referencePicker = p
	a.searchReferences(p, 1)
	return p
}

func referenceQuery(p *referencePicker, start int) rally.Query {
	q := rally.Query{Workspace: p.workspace, Project: p.project, Parents: p.parents, Children: p.children, Start: start, PageSize: 50, Order: "Name ASC", Fetch: "ObjectID,FormattedID,Name"}
	fields := []string{"Name", "FormattedID"}
	switch p.kinds[p.kind] {
	case "User":
		q.Project, q.Order, q.Fetch = "", "DisplayName ASC", "ObjectID,DisplayName,UserName"
		fields = []string{"DisplayName", "UserName"}
	case "Project", "Tag":
		q.Project, q.Fetch = "", "ObjectID,Name"
		fields = []string{"Name"}
	case "Iteration", "Release":
		q.Fetch = "ObjectID,Name"
		fields = []string{"Name"}
	}
	if search := strings.TrimSpace(text(p.search)); search != "" {
		for _, field := range fields {
			clause := "(" + field + " contains " + rally.Quote(search) + ")"
			if q.Expression == "" {
				q.Expression = clause
			} else {
				q.Expression = "(" + q.Expression + " OR " + clause + ")"
			}
		}
	}
	return q
}

func (a *App) searchReferences(p *referencePicker, start int) {
	if p.closed {
		return
	}
	if p.cancel != nil {
		p.cancel()
	}
	p.generation++
	generation := p.generation
	p.loading, p.err, p.items = true, "", nil
	ctx, cancel := context.WithTimeout(a.ctx, 30*time.Second)
	p.cancel = cancel
	if start > 1 && text(p.search) != p.appliedSearch {
		start = 1
	}
	p.appliedSearch = text(p.search)
	query, kind, client := referenceQuery(p, start), p.kinds[p.kind], p.client
	a.work(func() {
		defer cancel()
		page, err := client.CachedQuery(ctx, kind, query, false)
		a.post(func() {
			if p.closed || p.generation != generation || p.detail.referencePicker != p {
				return
			}
			p.loading = false
			if p.current != nil && !p.current() {
				p.err = "The filter or scope changed. Close and reopen this picker."
				return
			}
			if a.rallyClient != client {
				p.err = "Rally connection changed. Close and reopen this picker."
				return
			}
			if err != nil {
				p.err = err.Error()
				return
			}
			p.items = slices.DeleteFunc(slices.Clone(page.Results), func(o rally.Object) bool {
				actual, valid := client.ReferenceKind(o.String("_ref"))
				return !valid || actual != kind || o.String("_ref") == p.detail.Original.String("_ref")
			})
			p.start, p.total = query.Start, page.Total
		})
	}, func() {
		cancel()
		if !p.closed && p.generation == generation {
			p.loading, p.err = false, errWorkQueueFull.Error()
		}
	})
}

func (a *App) selectReference(p *referencePicker, o rally.Object) bool {
	if p.closed || a.rallyClient != p.client || p.detail.referencePicker != p || p.detail.Editors[p.field.Name] != p.editor || p.editor.TextRevision() != p.editorRevision || p.current != nil && !p.current() {
		p.err = "This field changed while the picker was open. Close and reopen it."
		return false
	}
	ref := o.String("_ref")
	actual, valid := p.client.ReferenceKind(ref)
	if !valid || actual != p.kinds[p.kind] || !slices.ContainsFunc(p.items, func(item rally.Object) bool { return item.String("_ref") == ref }) {
		return false
	}
	if editableCollection(p.field) {
		refs, err := decodeCollectionRefs(text(p.editor))
		if err != nil {
			p.err = err.Error()
			return false
		}
		if !slices.Contains(refs, ref) {
			setText(p.editor, encodeCollectionRefs(append(refs, ref)))
		}
	} else {
		setText(p.editor, ref)
	}
	if p.detail.referenceLabels == nil {
		p.detail.referenceLabels = map[string]string{}
	}
	for k := range p.detail.referenceLabels {
		if !editableCollection(p.field) && strings.HasPrefix(k, p.field.Name+"\x00") {
			delete(p.detail.referenceLabels, k)
		}
	}
	p.detail.referenceLabels[p.field.Name+"\x00"+ref] = referenceLabel(o)
	p.detail.snapshotRevision++
	if p.selected != nil {
		p.selected(o)
	}
	p.close()
	return true
}

func referenceLabel(o rally.Object) string {
	name := fallback(o.String("DisplayName"), fallback(o.String("Name"), fallback(o.String("_refObjectName"), o.String("UserName"))))
	if id := o.String("FormattedID"); id != "" {
		return id + " · " + name
	}
	return name
}

func (a *App) drawReferencePicker(w *desktop.Window, p *referencePicker) {
	w.OnClose(p.close)
	if p.closed {
		w.Close()
		return
	}
	for event := range w.Input().Keyboard.Events() {
		if event.HandleKey(key.CodeEscape, 0) {
			w.Close()
			return
		}
		if event.HandleKey(key.CodeReturnEnter, 0) {
			a.searchReferences(p, 1)
		}
	}
	if len(p.kinds) > 1 {
		labels := make([]string, len(p.kinds))
		for i, kind := range p.kinds {
			labels[i] = rallyKindLabel(kind)
		}
		w.Row(30).Dynamic(1)
		if next := w.ComboSimple(labels, p.kind, 28); next != p.kind {
			p.kind = next
			a.searchReferences(p, 1)
		}
	}
	w.Row(30).Ratio(.78, .22)
	p.search.Edit(w)
	if w.ButtonText("Search") {
		a.searchReferences(p, 1)
	}
	if p.loading {
		muted(w, "Loading choices…", a.p)
	}
	if p.err != "" {
		w.Row(60).Dynamic(1)
		w.LabelWrap(p.err)
	}
	w.RowScaled(max(120, w.LayoutAvailableHeight()-100)).Dynamic(1)
	if body := w.GroupBegin("reference-choices", desktop.WindowNoHScrollbar); body != nil {
		for _, o := range p.items {
			body.Row(32).Dynamic(1)
			if body.ButtonText(referenceLabel(o)) && a.selectReference(p, o) {
				w.Close()
				break
			}
		}
		if !p.loading && p.err == "" && len(p.items) == 0 {
			muted(body, "No matching items.", a.p)
		}
		body.GroupEnd()
	}
	w.Row(30).Dynamic(3)
	if enabledButton(w, "Previous", !p.loading && p.start > 1, false, a.p) {
		a.searchReferences(p, max(1, p.start-50))
	}
	w.Label(fmt.Sprintf("%d choices", p.total), "CC")
	if enabledButton(w, "Next", !p.loading && p.start+50 <= p.total, false, a.p) {
		a.searchReferences(p, p.start+50)
	}
}
