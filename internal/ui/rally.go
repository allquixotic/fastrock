package ui

import (
	"context"
	"fmt"
	"image/color"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/aarzilli/nucular"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/settings"
)

type rallyView struct {
	Spec                                    rally.PageSpec
	Mode, Group, Timebox, ViewName          string
	Query, Search                           *nucular.TextEditor
	Items                                   []rally.Object
	Loading                                 bool
	Error                                   string
	Generation                              int
	Selected                                map[string]bool
	Filters, Widgets, ExitAgreements, Rules bool
	Sort                                    string
	Descending                              bool
	Page, PageSize                          int
	Detail                                  *detailView
	Columns                                 []string
	ShowFields                              bool
	filterCache                             []rally.Object
	filterSource                            *rally.Object
	filterGeneration                        int
	filterText, filterOwner, filterState    string
	filterBlocked, filterReady              bool
	filterRevision                          uint64
	filterSearch                            []string
	filterSort                              string
	filterDesc                              bool
	OnlyBlocked, OnlyReady                  bool
	OwnerFilter, StateFilter                string
	cards                                   map[string]*boardCard
	cardSource                              *rally.Object
	cardGeneration                          int
	dragCard                                string
	dragCardX, dragCardY                    int
	cardDragging                            bool
	ownerOptions, stateOptions              []string
	boardGroups                             []boardGroup
	boardWidths                             []int
	boardRevision                           uint64
	boardGrouping                           string
	boardPrepared                           bool
}

func newRallyView(s rally.PageSpec) *rallyView {
	v := &rallyView{Spec: s, Mode: s.Mode, Group: "None", Query: textEditor("", false), Search: textEditor("", false), Selected: map[string]bool{}, Page: 1, PageSize: 25, Columns: []string{"FormattedID", "Name", "ScheduleState", "PlanEstimate", "Owner", "Iteration"}, Sort: "Rank"}
	v.Search.Placeholder = "Search ID, title, owner, description"
	v.Query.Placeholder = "Advanced WSAPI query"
	return v
}
func (a *App) rallyQuery(v *rallyView) rally.Query {
	q := rally.Query{Expression: text(v.Query), Workspace: a.prefs.RallyWorkspace, Project: a.prefs.RallyProject, Parents: a.prefs.ProjectParents, Children: a.prefs.ProjectChildren, Order: v.Sort + " ASC"}
	if v.Descending {
		q.Order = v.Sort + " DESC"
	}
	if v.Timebox != "" {
		kind := "Iteration"
		for _, r := range a.releases {
			if r.String("_ref") == v.Timebox {
				kind = "Release"
				break
			}
		}
		q.Expression = rally.And(q.Expression, rally.Eq(kind, v.Timebox))
	}
	if v.Spec.Kind == "Workspace" || v.Spec.Kind == "User" || v.Spec.Kind == "Project" {
		q.Project = ""
		q.Order = "Name ASC"
	}
	return q
}
func (a *App) refreshRally(v *rallyView) {
	c := a.rallyClient
	if c == nil {
		return
	}
	q := a.rallyQuery(v)
	kind := v.Spec.Kind
	v.Loading = true
	v.Error = ""
	v.Generation++
	generation := v.Generation
	a.work(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 60*time.Second)
		defer cancel()
		items, e := c.All(ctx, kind, q)
		a.post(func() {
			if v.Generation != generation {
				return
			}
			v.Loading = false
			if e != nil {
				v.Error = e.Error()
				return
			}
			v.Items = items
			v.Selected = map[string]bool{}
			v.Page = 1
		})
	})
}
func (a *App) drawRally(w *nucular.Window, v *rallyView) {
	if v == nil {
		return
	}
	a.rallyNav(w, v)
	if a.rallyClient == nil {
		title(w, "Connect to Rally", a.p)
		w.Row(65).Dynamic(1)
		w.LabelWrap(a.rallyErr)
		w.Row(30).Static(160)
		if primary(w, "Open Settings", a.p) {
			a.openSettings()
		}
		return
	}
	if v.Detail != nil {
		a.drawDetail(w, v)
		return
	}
	w.Row(32).Static(210, 170, 155, 90, 80, 80, 80)
	w.Label(v.Spec.Title, "LC")
	boxes := []string{"All timeboxes"}
	refs := []string{""}
	for _, o := range append(append([]rally.Object{}, a.iterations...), a.releases...) {
		boxes = append(boxes, o.String("Name"))
		refs = append(refs, o.String("_ref"))
	}
	bi := index(refs, v.Timebox)
	next := w.ComboSimple(boxes, bi, 28)
	if bi != next {
		v.Timebox = refs[next]
		a.refreshRally(v)
	}
	saved := []string{"Standard View"}
	for _, view := range a.prefs.Views {
		if view.Page == v.Spec.ID {
			saved = append(saved, view.Name)
		}
	}
	vi := index(saved, v.ViewName)
	nv := w.ComboSimple(saved, vi, 28)
	if nv != vi {
		v.ViewName = saved[nv]
		if nv == 0 {
			setText(v.Query, "")
			v.Group = "None"
			v.Mode = v.Spec.Mode
		} else {
			for _, sv := range a.prefs.Views {
				if sv.Name == v.ViewName && sv.Page == v.Spec.ID {
					setText(v.Query, sv.Query)
					v.Group = sv.Group
					v.Mode = sv.Mode
					break
				}
			}
		}
		a.refreshRally(v)
	}
	if w.ButtonText("Save view") {
		a.inputDialog("Save a private view", v.ViewName, func(name string) {
			if name == "" {
				return
			}
			sv := settings.SavedView{Name: name, Page: v.Spec.ID, Query: text(v.Query), Group: v.Group, Mode: v.Mode}
			replaced := false
			for i, x := range a.prefs.Views {
				if x.Name == name && x.Page == v.Spec.ID {
					a.prefs.Views[i] = sv
					replaced = true
				}
			}
			if !replaced {
				a.prefs.Views = append(a.prefs.Views, sv)
			}
			v.ViewName = name
			a.savePrefs()
		})
	}
	if button(w, "List", v.Mode == "list", a.p) {
		v.Mode = "list"
	}
	if button(w, "Board", v.Mode == "board", a.p) {
		v.Mode = "board"
	}
	if button(w, "Charts", v.Mode == "charts" || v.Mode == "dashboard", a.p) {
		v.Mode = "charts"
	}
	w.Row(30).Static(240, 95, 100, 150, 110, 100)
	v.Search.Edit(w)
	if primary(w, "+ Add New", a.p) {
		a.newArtifact(v)
	}
	if button(w, "Filters", v.Filters, a.p) {
		v.Filters = !v.Filters
	}
	groups := []string{"None", "Owner", "Iteration", "Release", "ScheduleState", "Feature"}
	v.Group = groups[w.ComboSimple(groups, index(groups, v.Group), 28)]
	if w.ButtonText("Show Fields") {
		v.ShowFields = !v.ShowFields
	}
	if w.ButtonText("Export CSV") {
		a.exportRally(v)
	}
	if v.Filters {
		v.prepareCards()
		w.Row(28).Static(100, 100, 200, 180)
		w.CheckboxText("Blocked", &v.OnlyBlocked)
		w.CheckboxText("Ready", &v.OnlyReady)
		owners := v.ownerOptions
		if n := w.ComboSimple(owners, index(owners, fallback(v.OwnerFilter, "All owners")), 28); n == 0 {
			v.OwnerFilter = ""
		} else {
			v.OwnerFilter = owners[n]
		}
		states := v.stateOptions
		if n := w.ComboSimple(states, index(states, fallback(v.StateFilter, "All states")), 28); n == 0 {
			v.StateFilter = ""
		} else {
			v.StateFilter = states[n]
		}
		w.Row(30).Ratio(.78, .1, .12)
		v.Query.Edit(w)
		if w.ButtonText("Apply") {
			a.refreshRally(v)
		}
		if w.ButtonText("Clear all") {
			setText(v.Query, "")
			setText(v.Search, "")
			v.Timebox = ""
			v.OwnerFilter, v.StateFilter = "", ""
			v.OnlyBlocked, v.OnlyReady = false, false
			a.refreshRally(v)
		}
		muted(w, `WSAPI query, e.g. (Blocked = true) or (Owner.UserName = "name@example.com")`, a.p)
	}
	if v.ShowFields {
		w.Row(28).Dynamic(6)
		for _, name := range []string{"FormattedID", "Name", "ScheduleState", "PlanEstimate", "Owner", "Iteration", "Release", "Blocked", "Tasks", "Discussion", "Feature", "LastUpdateDate"} {
			on := contains(v.Columns, name)
			if w.CheckboxText(name, &on) {
				if on {
					v.Columns = append(v.Columns, name)
				} else {
					v.Columns = remove(v.Columns, name)
				}
			}
		}
	}
	items := v.filtered()
	if v.Loading {
		muted(w, "Loading Rally work items…", a.p)
	}
	if v.Error != "" {
		w.Row(45).Dynamic(1)
		w.LabelWrap(v.Error)
	}
	selected := 0
	for _, on := range v.Selected {
		if on {
			selected++
		}
	}
	if selected > 0 {
		w.Row(30).Static(130, 100, 100, 130, 100)
		w.Label(fmt.Sprintf("%d selected", selected), "LC")
		if w.ButtonText("Block") {
			a.bulk(v, rally.Object{"Blocked": true})
		}
		if w.ButtonText("Unblock") {
			a.bulk(v, rally.Object{"Blocked": false})
		}
		if w.ButtonText("Complete") {
			field := rally.StateField(v.Spec.Kind)
			a.bulk(v, rally.Object{field: "Completed"})
		}
		if w.ButtonText("Delete…") {
			a.deleteSelected(v)
		}
	}
	if v.Mode == "board" {
		w.Row(26).Dynamic(3)
		w.CheckboxText("Show Widgets", &v.Widgets)
		w.CheckboxText("Show Exit Agreements", &v.ExitAgreements)
		w.CheckboxText("Show Work Rules based on State", &v.Rules)
		if v.Widgets {
			a.metrics(w, items, v.Spec.Kind)
		}
	}
	h := max(150, w.LayoutAvailableHeight()-38)
	w.Row(h).Dynamic(1)
	if body := w.GroupBegin("rally-content-"+v.Spec.ID, 0); body != nil {
		switch v.Mode {
		case "board":
			a.board(body, v, items)
		case "charts", "dashboard":
			a.charts(body, v, items)
		case "planning":
			a.planning(body, v, items)
		case "timeline":
			a.timeline(body, v, items)
		default:
			a.table(body, v, items)
		}
		body.GroupEnd()
	}
	w.Row(28).Static(170, 120, 120, 70, 70, 120)
	pages := max(1, (len(items)+v.PageSize-1)/v.PageSize)
	w.Label(fmt.Sprintf("%d work items", len(items)), "LC")
	w.Label(fmt.Sprintf("Page %d of %d", v.Page, pages), "LC")
	sizes := []string{"10", "25", "50", "100"}
	oldSize := v.PageSize
	v.PageSize, _ = strconv.Atoi(sizes[w.ComboSimple(sizes, index(sizes, strconv.Itoa(v.PageSize)), 28)])
	if oldSize != v.PageSize {
		v.Page = 1
	}
	if w.ButtonText("Previous") {
		v.Page = max(1, v.Page-1)
	}
	if w.ButtonText("Next") {
		v.Page = min(pages, v.Page+1)
	}
	if w.ButtonText("Refresh") {
		a.refreshRally(v)
	}
}
func (a *App) rallyNav(w *nucular.Window, v *rallyView) {
	w.Row(34).Static(170, 90, 90, 58, 58, 58, 70, 82, 70)
	projects := []string{"All teams"}
	refs := []string{""}
	for _, o := range a.projects {
		projects = append(projects, o.String("Name"))
		refs = append(refs, o.String("_ref"))
	}
	old := index(refs, a.prefs.RallyProject)
	next := w.ComboSimple(projects, old, 28)
	if next != old {
		a.prefs.RallyProject = refs[next]
		a.savePrefs()
		a.loadScope()
		for _, r := range a.rallyViews {
			a.refreshRally(r)
		}
	}
	parentsChanged := w.CheckboxText("↑ Parents", &a.prefs.ProjectParents)
	childrenChanged := w.CheckboxText("↓ Children", &a.prefs.ProjectChildren)
	if parentsChanged || childrenChanged {
		a.savePrefs()
		a.reloadRally()
	}
	for _, group := range []string{"Home", "Plan", "Track", "Quality", "Portfolio", "Reports"} {
		if menu := w.Menu(labelText(group), 210, nil); menu != nil {
			for _, p := range rally.Pages {
				if p.Group == group {
					menu.Row(27).Dynamic(1)
					if menu.MenuItem(labelText(p.Title)) {
						a.openRally(p.ID)
					}
				}
			}
		}
	}
	w.Row(29).Ratio(.8, .2)
	w.LabelColored("Fastrock  /  "+v.Spec.Title, "LC", a.p.Muted)
	if button(w, "Ask AI", a.assistant != nil, a.p) {
		a.openAssistant(v)
	}
}
func (v *rallyView) filtered() []rally.Object {
	var source *rally.Object
	if len(v.Items) > 0 {
		source = &v.Items[0]
	}
	queryText := text(v.Search)
	changed := source != v.filterSource || v.Generation != v.filterGeneration || len(v.filterSearch) != len(v.Items)
	if !changed && queryText == v.filterText && v.OwnerFilter == v.filterOwner && v.StateFilter == v.filterState && v.OnlyBlocked == v.filterBlocked && v.OnlyReady == v.filterReady && v.Sort == v.filterSort && v.Descending == v.filterDesc {
		return v.filterCache
	}
	if changed {
		v.filterSearch = make([]string, len(v.Items))
		for i, o := range v.Items {
			v.filterSearch[i] = strings.ToLower(o.ID() + " " + o.String("Name") + " " + o.String("Owner") + " " + plainHTML(o.String("Description")))
		}
		v.filterSource = source
		v.filterGeneration = v.Generation
	}
	v.filterText, v.filterOwner, v.filterState = queryText, v.OwnerFilter, v.StateFilter
	v.filterBlocked, v.filterReady = v.OnlyBlocked, v.OnlyReady
	v.filterRevision++
	v.filterSort = v.Sort
	v.filterDesc = v.Descending
	query := strings.ToLower(queryText)
	out := v.filterCache[:0]
	for i, o := range v.Items {
		if query != "" && !strings.Contains(v.filterSearch[i], query) {
			continue
		}
		if v.OnlyBlocked && !o.Bool("Blocked") {
			continue
		}
		if v.OnlyReady && !o.Bool("Ready") {
			continue
		}
		if v.OwnerFilter != "" && fallback(o.String("Owner"), "Unassigned") != v.OwnerFilter {
			continue
		}
		if v.StateFilter != "" && o.String(rally.StateField(v.Spec.Kind)) != v.StateFilter {
			continue
		}
		out = append(out, o)
	}
	if v.Sort != "Rank" {
		sort.SliceStable(out, func(i, j int) bool {
			a, b := out[i].String(v.Sort), out[j].String(v.Sort)
			if v.Descending {
				return a > b
			}
			return a < b
		})
	}
	v.filterCache = out
	return out
}
func (a *App) table(w *nucular.Window, v *rallyView, items []rally.Object) {
	columns := v.Columns
	if v.Spec.Kind == "Task" {
		columns = []string{"FormattedID", "Name", "State", "Estimate", "ToDo", "Owner"}
	} else if v.Spec.Kind == "Project" || v.Spec.Kind == "User" || v.Spec.Kind == "Release" || v.Spec.Kind == "Iteration" {
		columns = []string{"ObjectID", "Name", "State", "StartDate", "EndDate"}
	}
	ratios := []float64{.04}
	for _, k := range columns {
		width := .12
		if k == "Name" {
			width = .35
		}
		ratios = append(ratios, width)
	}
	total := 0.0
	for _, r := range ratios {
		total += r
	}
	for i := range ratios {
		ratios[i] /= total
	}
	w.Row(28).Ratio(ratios...)
	all := len(items) > 0
	for _, o := range items {
		all = all && v.Selected[o.String("_ref")]
	}
	if w.CheckboxText("", &all) {
		for _, o := range items {
			v.Selected[o.String("_ref")] = all
		}
	}
	for _, k := range columns {
		if w.ButtonText(k) {
			if v.Sort == k {
				v.Descending = !v.Descending
			} else {
				v.Sort = k
				v.Descending = false
			}
		}
	}
	start := min((v.Page-1)*v.PageSize, len(items))
	end := min(start+v.PageSize, len(items))
	previous := ""
	for _, o := range items[start:end] {
		if v.Group != "None" && o.String(v.Group) != previous {
			previous = o.String(v.Group)
			title(w, fallback(previous, "Unassigned"), a.p)
		}
		w.Row(33).Ratio(ratios...)
		ref := o.String("_ref")
		on := v.Selected[ref]
		if w.CheckboxText("", &on) {
			v.Selected[ref] = on
		}
		for _, k := range columns {
			value := o.String(k)
			if k == "Tasks" || k == "Discussion" {
				value = strconv.Itoa(o.Count(k))
			}
			if k == "Blocked" && o.Bool(k) {
				value = "Blocked"
			}
			if k == "Name" || k == "FormattedID" || k == "ObjectID" {
				if button(w, cut(fallback(value, "—"), 58), false, a.p) {
					a.openArtifact(v, o)
				}
			} else {
				fg := a.p.Text
				if k == "ScheduleState" || k == "State" {
					fg = stateColor(value, a.p)
				}
				w.LabelColored(fallback(value, "—"), "LC", fg)
			}
		}
	}
	if len(items) == 0 {
		muted(w, "No work items match the current scope and filters.", a.p)
	}
}
func (a *App) board(w *nucular.Window, v *rallyView, items []rally.Object) {
	a.drawTeamBoard(w, v, items)
}
func (a *App) metrics(w *nucular.Window, items []rally.Object, kind string) {
	counts := map[string]int{}
	points := 0.0
	blocked := 0
	for _, o := range items {
		counts[o.String(rally.StateField(kind))]++
		points += o.Number("PlanEstimate")
		if o.Bool("Blocked") {
			blocked++
		}
	}
	w.Row(64).Dynamic(4)
	for _, s := range []string{fmt.Sprintf("%d\nWork items", len(items)), fmt.Sprintf("%g\nPlan estimate", points), fmt.Sprintf("%d\nAccepted", counts["Accepted"]), fmt.Sprintf("%d\nBlocked", blocked)} {
		w.LabelWrap(s)
	}
}
func (a *App) charts(w *nucular.Window, v *rallyView, items []rally.Object) {
	a.metrics(w, items, v.Spec.Kind)
	title(w, "Work by state", a.p)
	counts := map[string]int{}
	for _, o := range items {
		counts[o.String(rally.StateField(v.Spec.Kind))]++
	}
	largest := 1
	for _, n := range counts {
		largest = max(largest, n)
	}
	for _, s := range rally.States(v.Spec.Kind) {
		w.Row(30).Ratio(.25, .65, .10)
		w.Label(s, "LC")
		n := counts[s]
		w.Progress(&n, largest, false)
		w.Label(strconv.Itoa(counts[s]), "RC")
	}
	title(w, "Team workload", a.p)
	owners := map[string]float64{}
	for _, o := range items {
		owners[fallback(o.String("Owner"), "Unassigned")] += o.Number("PlanEstimate")
	}
	for owner, points := range owners {
		w.Row(28).Ratio(.75, .25)
		w.Label(owner, "LC")
		w.Label(fmt.Sprintf("%g points", points), "RC")
	}
	muted(w, "Calculated from the work items in the selected scope and filters.", a.p)
}
func (a *App) planning(w *nucular.Window, v *rallyView, items []rally.Object) {
	a.metrics(w, items, v.Spec.Kind)
	title(w, "Iteration capacity and planned scope", a.p)
	for _, it := range a.iterations {
		var points float64
		count := 0
		for _, o := range items {
			if o.Ref("Iteration") == it.String("_ref") {
				points += o.Number("PlanEstimate")
				count++
			}
		}
		w.Row(36).Ratio(.5, .25, .25)
		w.Label(it.String("Name"), "LC")
		w.Label(fmt.Sprintf("%d items", count), "LC")
		w.Label(fmt.Sprintf("%g planned points", points), "RC")
	}
	title(w, "Planning backlog", a.p)
	a.table(w, v, items)
}
func (a *App) timeline(w *nucular.Window, v *rallyView, items []rally.Object) {
	title(w, "Feature timeline", a.p)
	for _, o := range items {
		w.Row(34).Ratio(.42, .29, .29)
		if w.ButtonText(o.ID() + " " + cut(o.String("Name"), 50)) {
			a.openArtifact(v, o)
		}
		w.Label(fallback(o.String("PlannedStartDate"), "No start date"), "LC")
		w.Label(fallback(o.String("PlannedEndDate"), "No end date"), "LC")
	}
}
func index(xs []string, s string) int {
	for i, x := range xs {
		if x == s {
			return i
		}
	}
	return 0
}
func contains(xs []string, s string) bool {
	for _, x := range xs {
		if x == s {
			return true
		}
	}
	return false
}
func remove(xs []string, s string) []string {
	out := []string{}
	for _, x := range xs {
		if x != s {
			out = append(out, x)
		}
	}
	return out
}
func fallback(s, d string) string {
	if s == "" {
		return d
	}
	return s
}
func stateColor(s string, p palette) color.RGBA {
	switch s {
	case "Accepted", "Delivered", "Completed":
		return p.Success
	case "In-Progress", "Implementing":
		return p.Accent
	case "Blocked":
		return p.Danger
	case "Defined", "Product Backlog":
		return p.Warning
	}
	return p.Muted
}
