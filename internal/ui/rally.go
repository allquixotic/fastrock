package ui

import (
	"context"
	"fmt"
	"image/color"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/settings"
)

type rallyView struct {
	SavedViewManager                                            *savedViewManager
	Display                                                     settings.BoardDisplay
	DisplayDraft                                                *boardDisplayDraft
	boardStride                                                 int
	boardHeader                                                 int
	tableLinks                                                  []tableLinkLayout
	boardWrites                                                 map[string]*boardWrite
	boardPlacements                                             []boardPlacement
	boardUndo                                                   *boardUndo
	boardRefreshQueued                                          bool
	planningRevision                                            uint64
	planningPrepared                                            bool
	planningSummary                                             planningSummary
	historyRevision                                             uint64
	CollapsedLanes                                              map[string]bool
	PendingCards                                                map[string]bool
	CardFields                                                  []string
	AIView                                                      bool
	CurrentIteration                                            bool
	PresetError                                                 string
	Mutating                                                    bool
	selectionRevision, selectedRevision, selectedFilterRevision uint64
	selectedVisible                                             int
	largestLane                                                 int
	Exporting                                                   bool
	ExportCancel                                                context.CancelFunc
	LaneScroll                                                  map[string]int
	RestoreLaneScroll                                           map[string]int
	Closed                                                      bool
	Refreshed                                                   time.Time
	RefreshAt                                                   time.Time
	searchTimer                                                 *time.Timer
	RetryAfter                                                  time.Time
	Failures                                                    int
	cancel                                                      context.CancelFunc
	Total, Start, Next                                          int
	RestoreCount                                                int
	More, Evicted, ServerFiltered                               bool
	AppliedSearch                                               string
	AppliedFilters                                              string
	signature, pendingSignature                                 string
	searchChanged                                               time.Time
	scrollAdjustment                                            map[string]int

	Spec                                     rally.PageSpec
	Mode, Group, Timebox, ViewName           string
	TimeboxName, ReleaseTimebox, ReleaseName string
	Query, Search                            *desktop.TextEditor
	Items                                    []rally.Object
	Loading                                  bool
	Error                                    string
	CredentialError                          bool
	Generation                               int
	Selected                                 map[string]bool
	SelectedItems                            map[string]rally.Object
	Fields                                   []rally.Field
	Workflow                                 []rally.Object
	TypeMetadata                             map[string]rallyTypeMetadata
	WorkflowError                            string
	QueryApplied                             string
	StructuredFilters                        []settings.RallyFilter
	FilterDraft                              *rallyFilterDraft
	metadataSignature                        string
	Filters, Widgets, ExitAgreements, Rules  bool
	Sort                                     string
	Descending                               bool
	Page, PageSize                           int
	DetailHistory                            []rally.Object
	Detail                                   *detailView
	Columns                                  []string
	ShowFields                               bool
	filterCache                              []rally.Object
	filterValid                              bool
	filterSource                             *rally.Object
	filterGeneration                         int
	filterText, filterOwner, filterState     string
	filterBlocked, filterReady               bool
	filterRevision                           uint64
	filterSearch                             []string
	filterSort, filterGroup                  string
	groupCounts                              map[string]int
	filterDesc                               bool
	OnlyBlocked, OnlyReady                   bool
	OwnerFilter, StateFilter                 string
	cards                                    map[string]*boardCard
	cardSource                               *rally.Object
	cardGeneration                           int
	dragCard                                 string
	dragCardX, dragCardY                     int
	cardDragging                             bool
	ownerOptions, stateOptions               []string
	boardGroups                              []boardGroup
	boardWidths                              []int
	boardRevision                            uint64
	boardGrouping                            string
	boardPrepared                            bool
	focusCard                                string
	focusCardIndex                           int
	cardFocusActive, revealCard, focusSearch bool
}

func newRallyView(s rally.PageSpec) *rallyView {
	v := &rallyView{Display: settings.DefaultBoardDisplay(), Spec: s, Mode: s.Mode, Group: "None", Query: textEditor("", false), Search: textEditor("", false), Selected: map[string]bool{}, Page: 1, PageSize: 25, Columns: []string{"FormattedID", "Name", "ScheduleState", "PlanEstimate", "Owner", "Iteration"}, Sort: "Rank"}
	switch s.Kind {
	case "Defect":
		v.Columns = []string{"FormattedID", "Name", "State", "Severity", "Priority", "Owner"}
	case "Task":
		v.Columns = []string{"FormattedID", "Name", "State", "Estimate", "ToDo", "Actuals", "Owner"}
	case "TestCase":
		v.Columns = []string{"FormattedID", "Name", "LastVerdict", "Method", "Owner"}
	case "Iteration", "Release":
		v.Columns = []string{"ObjectID", "Name", "StartDate", "EndDate", "Project"}
		v.Sort = "StartDate"
	case "Project":
		v.Columns = []string{"ObjectID", "Name", "State", "Owner"}
		v.Sort = "Name"
	case "User":
		v.Columns = []string{"ObjectID", "DisplayName", "UserName"}
		v.Sort = "DisplayName"
	case "PortfolioItem/Feature", "PortfolioItem/Epic":
		v.Columns = []string{"FormattedID", "Name", "State", "Owner", "PlannedStartDate", "PlannedEndDate"}
	}
	switch s.Kind {
	case "HierarchicalRequirement", "Defect", "Task", "TestCase", "PortfolioItem/Feature", "PortfolioItem/Epic":
		v.Columns = append([]string{"Rank"}, v.Columns...)
		if s.Kind != "TestCase" {
			v.Columns = append(v.Columns, "Blocked")
		}
	}
	v.Search.Placeholder = "Search ID, title, owner, description"
	if s.Kind == "User" {
		v.Search.Placeholder = "Search display name or username"
	}
	v.Query.Placeholder = "Advanced WSAPI query"
	v.Query.Flags |= desktop.EditSigEnter
	v.CardFields = []string{"Owner", "Iteration", "Tasks", "PlanEstimate"}
	if s.ID == "iterationstatus" {
		v.CurrentIteration, v.Widgets = true, true
	}
	return v
}
func (a *App) rallyQuery(v *rallyView) rally.Query {
	order := v.Sort
	if order == "Rank" {
		order = "DragAndDropRank"
	}
	q := rally.Query{Expression: v.QueryApplied, Workspace: a.prefs.RallyWorkspace, Project: a.prefs.RallyProject, Parents: a.prefs.ProjectParents, Children: a.prefs.ProjectChildren, Order: order + " ASC"}
	if types := v.Spec.ArtifactTypes(); len(types) > 0 {
		q.ArtifactTypes = strings.Join(v.queryTypes(), ",")
		if q.ArtifactTypes == "" {
			q.ArtifactTypes = strings.Join(types, ",")
			q.Expression = rally.And(q.Expression, "(ObjectID = 0)")
		}
	}
	filters, err := a.structuredFilterExpression(v)
	if err != nil {
		// Invalid restored filters must not silently broaden assistant/export
		// scope. Interactive loads report the validation error before querying.
		filters = "(ObjectID = 0)"
	}
	q.Expression = rally.And(q.Expression, filters)
	switch v.Spec.ID {
	case "backlog":
		q.Expression = rally.And(q.Expression, "(Iteration = null)")
	case "mywork":
		if ref := a.rallyUser.String("_ref"); ref != "" {
			q.Expression = rally.And(q.Expression, rally.Eq("Owner", ref))
		} else {
			q.Expression = rally.And(q.Expression, "(ObjectID = 0)")
		}
	}
	if v.CurrentIteration && v.Timebox == "" {
		q.Expression = rally.And(q.Expression, "(ObjectID = 0)")
	}
	if v.Descending {
		q.Order = order + " DESC"
	}
	if v.Group != "" && v.Group != "None" && v.Group != v.Sort {
		q.Order = v.Group + " ASC," + q.Order
	}
	iteration, iterationName, release, releaseName := v.Timebox, v.TimeboxName, v.ReleaseTimebox, v.ReleaseName
	if legacyRelease(iteration) {
		if release == "" {
			release, releaseName = iteration, iterationName
		}
		iteration, iterationName = "", ""
	}
	q.Expression = rally.And(q.Expression, timeboxPredicate("Iteration", iteration, iterationName, a.iterations))
	q.Expression = rally.And(q.Expression, timeboxPredicate("Release", release, releaseName, a.releases))
	if search := strings.TrimSpace(text(v.Search)); search != "" {
		fields := []string{"Name", "Description", "FormattedID", "Owner.Name"}
		switch v.Spec.Kind {
		case "User":
			fields = []string{"DisplayName", "UserName"}
		case "Project", "Workspace", "Iteration", "Release", "TestFolder":
			fields = []string{"Name"}
		}
		expression := ""
		for _, field := range fields {
			clause := "(" + field + " contains " + rally.Quote(search) + ")"
			if expression == "" {
				expression = clause
			} else {
				expression = "(" + expression + " OR " + clause + ")"
			}
		}
		q.Expression = rally.And(q.Expression, expression)
	}
	if v.OnlyBlocked {
		q.Expression = rally.And(q.Expression, "(Blocked = true)")
	}
	if v.OnlyReady {
		q.Expression = rally.And(q.Expression, "(Ready = true)")
	}
	if v.OwnerFilter != "" {
		if v.OwnerFilter == "Unassigned" {
			q.Expression = rally.And(q.Expression, "(Owner = null)")
		} else {
			field := "Owner.Name"
			if strings.Contains(v.OwnerFilter, "/") {
				field = "Owner"
			}
			q.Expression = rally.And(q.Expression, rally.Eq(field, v.OwnerFilter))
		}
	}
	if v.StateFilter != "" {
		q.Expression = rally.And(q.Expression, rally.Eq(v.stateField(), v.StateFilter))
	}
	if v.Spec.Kind == "Workspace" || v.Spec.Kind == "User" || v.Spec.Kind == "Project" {
		q.Project = ""

	}
	return q
}
func (a *App) refreshRally(v *rallyView) {
	if v == nil || v.Spec.ID == "customviews" {
		return
	}
	if v.Detail != nil {
		if v.Detail.inlineField != "" && a.rallySignature(v) != v.signature {
			a.refreshRallyItems(v)
		} else {
			a.reloadDetail(v)
		}
		return
	}
	a.refreshRallyItems(v)
}
func (a *App) refreshRallyItems(v *rallyView) {
	if v == nil || v.Closed || v.Spec.ID == "customviews" {
		return
	}
	if len(v.PendingCards) > 0 {
		v.boardRefreshQueued = true
		return
	}
	if v.cancel != nil {
		v.cancel()
	}
	v.Loading = false
	v.Generation++
	v.Error = ""
	v.CredentialError = false
	if !a.prepareRallyPreset(v) {
		return
	}
	signature := a.rallySignature(v)
	start := 1
	if signature == v.signature {
		start = max(1, v.Start)
		if v.Mode != "board" {
			if _, _, resident := v.tableRange(len(v.Items)); !resident {
				start = (v.Page-1)*v.PageSize + 1
			}
		}
	} else {
		v.Page = 1
		v.RestoreCount = 0
	}
	v.signature = signature
	v.ServerFiltered = true
	a.requestRallyPage(v, start, true)
	if len(v.Fields) == 0 || signature != v.metadataSignature {
		a.loadViewMetadata(v)
	}
}
func (a *App) rallySignature(v *rallyView) string {
	return v.Group + "\x00" + v.QueryApplied + "\x00" + v.structuredFilterSignature() + "\x00" + text(v.Search) + "\x00" + v.OwnerFilter + "\x00" + v.StateFilter + fmt.Sprint(v.OnlyBlocked, v.OnlyReady, v.CurrentIteration) + v.Sort + fmt.Sprint(v.Descending) + "\x00" + v.Timebox + "\x00" + v.TimeboxName + "\x00" + v.ReleaseTimebox + "\x00" + v.ReleaseName + "\x00" + strings.Join(v.Columns, ",") + "\x00" + a.prefs.RallyEndpoint + "\x00" + a.prefs.RallyWorkspace + "\x00" + a.prefs.RallyProject + fmt.Sprint(a.prefs.ProjectParents, a.prefs.ProjectChildren) + "\x00" + a.rallyUser.String("_ref")
}

func (a *App) drawRally(w *desktop.Window, v *rallyView) {
	if v == nil {
		return
	}
	s := a.assistant
	if s == nil || !s.Visible || s.View != v {
		a.drawRallyContent(w, v)
		return
	}
	scale := w.Master().Style().Scaling
	width, height := w.LayoutAvailableWidth(), w.LayoutAvailableHeight()
	stacked := width < int(740*scale)
	if stacked {
		w.RowScaled(max(120, height/2)).Dynamic(1)
	} else {
		panel := min(int(420*scale), width*38/100)
		w.RowScaled(height).StaticScaled(width-panel-w.Master().Style().GroupWindow.Spacing.X, panel)
	}
	if main := w.GroupBegin("rally-main", desktop.WindowNoHScrollbar); main != nil {
		a.drawRallyContent(main, v)
		main.GroupEnd()
	}
	if stacked {
		w.RowScaled(max(120, w.LayoutAvailableHeight())).Dynamic(1)
	}
	if panel := w.GroupBegin("rally-assistant-panel", desktop.WindowNoHScrollbar); panel != nil {
		a.drawAssistant(panel)
		panel.GroupEnd()
	}
}

func (a *App) drawRallyContent(w *desktop.Window, v *rallyView) {
	if v == nil {
		return
	}
	if v.Spec.ID == "customviews" {
		a.drawSavedViewManager(w, v)
		return
	}
	if v.focusSearch && a.rallyRowHidden("search") {
		a.setRallyRowHidden("search", false)
	}
	a.rallyNav(w, v)
	if !v.Loading && !v.Refreshed.IsZero() && time.Now().After(v.RetryAfter) && (v.RefreshAt.IsZero() && time.Since(v.Refreshed) > 60*time.Second || !v.RefreshAt.IsZero() && time.Now().After(v.RefreshAt)) && (v.Detail == nil || !v.Detail.dirty()) {
		a.refreshRally(v)
	}
	if v.Evicted && !v.Loading {
		a.refreshRally(v)
	}
	signature := a.rallySignature(v)
	if signature != v.signature {
		if signature != v.pendingSignature {
			v.pendingSignature = signature
			v.searchChanged = time.Now()
			if v.searchTimer != nil {
				v.searchTimer.Stop()
			}
			v.searchTimer = time.AfterFunc(305*time.Millisecond, func() {
				a.post(func() {
					if !v.Closed && v.pendingSignature == signature && a.rallySignature(v) == signature {
						a.refreshRally(v)
					}
				})
			})
		}
	}
	if a.rallyClient == nil {
		caption := "Connect to Rally"
		if a.rallyErr == "Connecting to Rally…" {
			caption = a.rallyErr
		}
		title(w, caption, a.p)
		w.Row(65).Dynamic(1)
		w.LabelWrap(a.rallyErr)
		w.Row(30).Static(160)
		if primary(w, "Open Settings", a.p) {
			a.openRallySettings()
		}
		return
	}
	if v.Detail != nil && v.Detail.inlineField == "" {
		a.drawDetail(w, v)
		return
	}
	if v.PresetError != "" {
		muted(w, v.PresetError, a.p)
		w.Row(28).Static(170)
		if enabledButton(w, "Retry page setup", !a.rallyUserLoading && (a.rallyUserError != "" || a.iterationError != ""), false, a.p) {
			if v.Spec.ID == "mywork" {
				a.loadRallyUser()
			} else {
				a.loadScope()
			}
		}
		return
	}
	a.drawInlineStatus(w, v)
	a.rallyRow(w, v, "saved-view", func(w *desktop.Window) {
		labelWidth := 110
		if v.AIView {
			labelWidth = 210
		}
		scale := w.Master().Style().Scaling
		labelPixels, savePixels := int(float64(labelWidth)*scale), rallyButtonWidth(w, "Save view", 20)
		available := w.LayoutAvailableWidth()
		narrow := available < labelPixels+savePixels+int(130*scale)
		if narrow {
			w.Row(22).Dynamic(1)
		} else {
			w.Row(28).StaticScaled(labelPixels, min(int(320*scale), available-labelPixels-savePixels-int(12*scale)), savePixels)
		}
		a.drawRallyViewLabel(w, v)
		if narrow {
			w.Row(28).StaticScaled(0, savePixels)
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
				v.applySavedView(settings.SavedView{})
			} else {
				for _, sv := range a.prefs.Views {
					if sv.Name == v.ViewName && sv.Page == v.Spec.ID {
						v.applySavedView(sv)
						break
					}
				}
			}
			a.refreshRally(v)
		}
		if w.ButtonText("Save view") {
			a.inputDialog("Save a private view", v.ViewName, func(name string) {
				name = strings.TrimSpace(name)
				if name == "" || name == "Standard View" {
					a.toast = "Choose a name other than Standard View"
					return
				}
				sv := v.savedView(name)
				if before := a.findSavedView(v.Spec.ID, name); before != nil {
					a.confirm("Save over existing view?", "Replace the saved settings for "+name+"?", func() { a.storeSavedView(v, sv, before) })
				} else {
					a.storeSavedView(v, sv, nil)
				}
			})
		}
	})
	if rallyTimeboxesSupported(v) {
		a.rallyRow(w, v, "timeboxes", func(w *desktop.Window) { a.drawTimeboxSelectors(w, v) })
	}
	a.rallyRow(w, v, "view-actions", func(w *desktop.Window) { a.drawSavedViewActions(w, v) })
	a.drawRallyModes(w, v)
	if saved := a.findSavedView(v.Spec.ID, v.ViewName); saved != nil {
		a.rallyRow(w, v, "saved-changes", func(w *desktop.Window) {
			w.Row(28).Static(160, 110, 85, 100)
			dirty := !savedViewEqual(v.savedView(v.ViewName), *saved)
			if dirty {
				w.Label("Unsaved view changes", "LC")
			} else {
				w.Label("Saved view", "LC")
			}
			if w.ButtonText("Save changes") {
				a.storeSavedView(v, v.savedView(v.ViewName), saved)
			}
			if w.ButtonText("Revert") {
				v.applySavedView(*saved)
				a.refreshRally(v)
			}
			if w.ButtonText("Delete view") {
				a.deleteSavedViewDialog(*saved)
			}
		})
	}
	filters := a.activeRallyFilters(v)
	a.rallyRow(w, v, "search", func(w *desktop.Window) {
		captions := []string{"+ Add New", rallyFilterCaption(v.Filters, len(filters)), "Show Fields", "Export CSV"}
		widths := []int{0}
		used := 0
		for _, caption := range captions {
			n := rallyButtonWidth(w, caption, 20)
			widths = append(widths, n)
			used += n + 5
		}
		narrow := w.LayoutAvailableWidth()-used < 140
		if narrow {
			w.Row(28).Dynamic(1)
		} else {
			w.Row(28).StaticScaled(widths...)
		}
		if v.focusSearch {
			w.Master().ActivateEditor(w, v.Search)
			v.focusSearch = false
		}
		v.Search.Edit(w)
		if v.Search.Active || v.Query.Active {
			v.cardFocusActive = false
		}
		draw := func(i int) {
			switch i {
			case 0:
				if primary(w, captions[i], a.p) {
					a.newArtifact(v)
				}
			case 1:
				if button(w, captions[i], v.Filters, a.p) {
					v.Filters = !v.Filters
					if v.Filters && a.rallyRowHidden("filters") {
						a.setRallyRowHidden("filters", false)
					}
				}
			case 2:
				if w.ButtonText(captions[i]) {
					v.ShowFields = !v.ShowFields
					if v.ShowFields && a.rallyRowHidden("fields") {
						a.setRallyRowHidden("fields", false)
					}
				}
			case 3:
				if w.ButtonText(captions[i]) {
					a.exportRally(v)
				}
			}
		}
		if narrow {
			compactRallyButtons(w, 28, captions, 20, draw)
		} else {
			for i := range captions {
				draw(i)
			}
		}
	})
	a.rallyRow(w, v, "swimlanes", func(w *desktop.Window) { a.drawRallyGrouping(w, v) })
	if len(filters) > 0 {
		a.rallyRow(w, v, "filter-chips", func(w *desktop.Window) { a.drawRallyFilterChips(w, v, filters) })
	}
	if v.Filters {
		a.rallyRow(w, v, "filters", func(w *desktop.Window) {
			v.prepareCards()
			w.Row(28).Static(100, 100, 200, 180)
			w.CheckboxText("Blocked", &v.OnlyBlocked)
			w.CheckboxText("Ready", &v.OnlyReady)
			if a.scopeChoices == nil || a.scopeChoices["OwnerFilter"] == nil {
				a.picker("OwnerFilter", append([]rally.Object{{"_ref": "Unassigned", "Name": "Unassigned"}}, a.users...))
			}
			owners, ownerRefs, selectedOwner := a.scopeChoices["OwnerFilter"].options("All owners", v.OwnerFilter)
			if n := w.ComboSimple(owners, selectedOwner, 28); n != selectedOwner {
				v.OwnerFilter = ownerRefs[n]
			}
			states := v.stateOptions
			if n := w.ComboSimple(states, index(states, fallback(v.StateFilter, "All states")), 28); n == 0 {
				v.StateFilter = ""
			} else {
				v.StateFilter = states[n]
			}
			a.drawRallyFilterBuilder(w, v)
			a.drawRallyAdvancedQuery(w, v)
		})
	}
	if v.ShowFields {
		a.rallyRow(w, v, "fields", func(w *desktop.Window) {
			if v.Mode == "board" {
				w.Row(28).Dynamic(4)
				for _, name := range []string{"Owner", "Iteration", "Tasks", "PlanEstimate"} {
					on := contains(v.CardFields, name)
					if w.CheckboxText(rallyFieldLabel(v, name), &on) {
						if on {
							v.CardFields = append(v.CardFields, name)
						} else {
							v.CardFields = remove(v.CardFields, name)
						}
					}
				}
			}
			w.Row(28).Dynamic(6)
			for _, name := range rallyColumnOptions(v) {
				on := contains(v.Columns, name)
				if w.CheckboxText(rallyFieldLabel(v, name), &on) {
					if on {
						v.Columns = append(v.Columns, name)
					} else {
						v.Columns = remove(v.Columns, name)
					}
				}
			}
		})
	}
	items := v.filtered()
	if v.CurrentIteration && v.Timebox == "" {
		muted(w, "No current iteration in this project scope. Choose a timebox to view other work.", a.p)
	}
	if v.Exporting {
		w.Row(28).Dynamic(2)
		w.Label("Exporting matching items…", "LC")
		if w.ButtonText("Cancel export") && v.ExportCancel != nil {
			v.ExportCancel()
		}
	}
	if v.Loading {
		muted(w, "Loading Rally work items…", a.p)
	}
	if v.Error != "" {
		w.Row(45).Dynamic(1)
		w.LabelWrap(v.Error)
		w.Row(28).Static(120, 180)
		if enabledButton(w, "Retry", !v.Loading, false, a.p) {
			a.refreshRally(v)
		}
		if v.CredentialError && w.ButtonText("Update API token") {
			a.openRallySettings()
		}
	}
	selected := len(v.Selected)
	if selected > 0 {
		w.Row(30).Static(130, 160, 120)
		w.Label(fmt.Sprintf("%d selected", selected), "LC")
		if enabledButton(w, "Edit selected…", !v.Mutating && len(v.PendingCards) == 0, false, a.p) {
			a.openSelectionEditor(v)
		}
		if w.ButtonText("Clear selection") {
			clear(v.Selected)
			clear(v.SelectedItems)
			v.selectionRevision++
		}
	}
	if v.Mode == "board" {
		a.rallyRow(w, v, "widgets", func(w *desktop.Window) {
			w.Row(26).Dynamic(1)
			w.CheckboxText("Show Widgets", &v.Widgets)
			if v.Widgets {
				if v.Spec.ID == "iterationstatus" {
					muted(w, "Iteration summary · loaded work items", a.p)
				}
				a.metrics(w, items, v.Spec.Kind)
			}
		})
	}
	footer := 0
	if !a.rallyRowHidden("summary") {
		footer = int(31 * w.Master().Style().Scaling)
	}
	h := max(150, w.LayoutAvailableHeight()-footer)
	w.RowScaled(h).Dynamic(1)
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
	a.rallyRow(w, v, "summary", func(w *desktop.Window) {
		w.Row(28).Static(260, 130, 100, 100)
		w.Label(fmt.Sprintf("%d loaded · %d matching work items", len(items), v.Total), "LC")
		if v.Mode != "board" {
			pages := max(1, (v.Total+v.PageSize-1)/v.PageSize)
			w.Label(fmt.Sprintf("Page %d of %d", v.Page, pages), "LC")
			if w.ButtonText("Previous") && !v.Loading && v.Page > 1 {
				a.rallyTablePage(v, v.Page-1)
			}
			if w.ButtonText("Next") && !v.Loading && v.Page < pages {
				a.rallyTablePage(v, v.Page+1)
			}
		}
	})

}
func (a *App) rallyNav(w *desktop.Window, v *rallyView) {
	a.drawRallyRowRestore(w, v)
	a.rallyRow(w, v, "sections", func(w *desktop.Window) {
		groups := []string{"Home", "Plan", "Track", "Quality", "Portfolio", "Reports"}
		compactRallyButtons(w, 25, groups, 20, func(i int) {
			group := groups[i]
			if button(w, group, v.Spec.Group == group, a.p) && group != v.Spec.Group {
				page := ""
				for _, p := range rally.Pages {
					if p.Group == group {
						page = p.ID
						break
					}
				}
				if group == "Track" {
					page = "teamboard"
				}
				a.openRally(page)
			}
		})
	})
	a.rallyRow(w, v, "pages", func(w *desktop.Window) {
		var pages []rally.PageSpec
		var labels []string
		for _, p := range rally.Pages {
			if p.Group == v.Spec.Group {
				pages = append(pages, p)
				labels = append(labels, p.Title)
			}
		}
		compactRallyButtons(w, 25, labels, 20, func(i int) {
			if button(w, labels[i], pages[i].ID == v.Spec.ID, a.p) {
				a.openRally(pages[i].ID)
			}
		})
	})
	a.rallyRow(w, v, "project", func(w *desktop.Window) {
		w.Row(28).Static(210, 100, 100)
		projects, refs, old := a.picker("Project", a.projects).options("All teams", a.prefs.RallyProject)
		next := w.ComboSimple(projects, old, 28)
		if next != old {
			a.prefs.RallyProject = refs[next]
			a.savePrefs()
			a.loadScope()
			a.reloadRally()
		}
		parentsChanged := w.CheckboxText("↑ Parents", &a.prefs.ProjectParents)
		childrenChanged := w.CheckboxText("↓ Children", &a.prefs.ProjectChildren)
		if parentsChanged || childrenChanged {
			a.savePrefs()
			a.loadScope()
			a.reloadRally()
		}
	})
	a.rallyRow(w, v, "page", func(w *desktop.Window) {
		w.Row(26).StaticScaled(0, rallyButtonWidth(w, "Refresh", 20), rallyButtonWidth(w, "Ask AI", 20))
		w.LabelColored(v.Spec.Group+"  /  "+v.Spec.Title, "LC", a.p.Muted)
		_, canRefresh, _ := rallyRefreshState(v)
		if enabledButton(w, "Refresh", a.rallyClient != nil && canRefresh, false, a.p) {
			a.refreshRally(v)
		}
		if enabledButton(w, "Ask AI", a.rallyClient != nil && a.client != nil, a.assistant != nil && a.assistant.Visible && a.assistant.View == v, a.p) {
			a.openAssistant(v)
		}
	})
	a.rallyRow(w, v, "freshness", func(w *desktop.Window) { a.drawRallyFreshness(w, v) })
}
func searchable(o rally.Object) string {
	return strings.ToLower(o.ID() + " " + o.String("Name") + " " + o.String("DisplayName") + " " + o.String("UserName") + " " + o.String("Owner") + " " + plainHTML(o.String("Description")))
}
func (v *rallyView) filtered() []rally.Object {
	var source *rally.Object
	if len(v.Items) > 0 {
		source = &v.Items[0]
	}
	queryText := text(v.Search)
	changed := source != v.filterSource || v.Generation != v.filterGeneration || len(v.filterSearch) != len(v.Items)
	if v.filterValid && !changed && queryText == v.filterText && v.OwnerFilter == v.filterOwner && v.StateFilter == v.filterState && v.OnlyBlocked == v.filterBlocked && v.OnlyReady == v.filterReady && v.Sort == v.filterSort && v.Descending == v.filterDesc && v.Group == v.filterGroup {
		return v.filterCache
	}
	if changed {
		v.filterSearch = make([]string, len(v.Items))
		for i, o := range v.Items {
			v.filterSearch[i] = searchable(o)
		}
		v.filterSource = source
		v.filterGeneration = v.Generation
	}
	v.filterText, v.filterOwner, v.filterState = queryText, v.OwnerFilter, v.StateFilter
	v.filterBlocked, v.filterReady = v.OnlyBlocked, v.OnlyReady
	v.filterRevision++
	v.filterSort, v.filterGroup = v.Sort, v.Group
	v.filterDesc = v.Descending
	serverMatched := v.ServerFiltered && v.AppliedFilters == v.quickFilterSignature()
	query := strings.ToLower(queryText)
	if serverMatched {
		query = ""
	}
	out := v.filterCache[:0]
	for i, o := range v.Items {
		if query != "" && !strings.Contains(v.filterSearch[i], query) {
			continue
		}
		if !serverMatched && v.OnlyBlocked && !o.Bool("Blocked") {
			continue
		}
		if !serverMatched && v.OnlyReady && !o.Bool("Ready") {
			continue
		}
		if !serverMatched && v.OwnerFilter != "" && fallback(o.String("Owner"), "Unassigned") != v.OwnerFilter && o.Ref("Owner") != v.OwnerFilter {
			continue
		}
		if !serverMatched && v.StateFilter != "" && o.String(v.stateField()) != v.StateFilter {
			continue
		}
		out = append(out, o)
	}
	sort.SliceStable(out, func(i, j int) bool {
		if v.Group != "" && v.Group != "None" {
			ki, ni, _ := boardGroupIdentity(out[i], v.Group)
			kj, nj, _ := boardGroupIdentity(out[j], v.Group)
			if ni != nj {
				return ni < nj
			}
			if ki != kj {
				return ki < kj
			}
		}
		order := compareRally(out[i], out[j], v.Sort, v.Fields)
		if order == 0 {
			return out[i].String("_ref") < out[j].String("_ref")
		}
		if v.Descending {
			return order > 0
		}
		return order < 0
	})
	v.groupCounts = nil
	if v.Group != "" && v.Group != "None" {
		v.groupCounts = make(map[string]int)
		for _, o := range out {
			key, _, _ := boardGroupIdentity(o, v.Group)
			v.groupCounts[key]++
		}
	}
	if len(out) < len(v.filterCache) {
		clear(v.filterCache[len(out):])
	}
	v.filterCache = out
	v.filterValid = true
	return out
}
func (a *App) table(w *desktop.Window, v *rallyView, items []rally.Object) {
	columns := v.Columns
	ratios := []float64{.04}
	for _, k := range columns {
		width := .12
		switch k {
		case "Name":
			width = .35
		case "Rank":
			width = .065
		case "ScheduleState", "State", "Owner":
			width = .16
		case "Iteration", "Release":
			width = .18
		}
		ratios = append(ratios, width)
	}
	ratios = append(ratios, .06)
	total := 0.0
	for _, r := range ratios {
		total += r
	}
	for i := range ratios {
		ratios[i] /= total
	}
	w.Row(28).Ratio(ratios...)
	widths := make([]int, len(columns))
	if v.selectedRevision != v.selectionRevision || v.selectedFilterRevision != v.filterRevision {
		v.selectedVisible = 0
		for _, o := range items {
			if v.Selected[o.String("_ref")] {
				v.selectedVisible++
			}
		}
		v.selectedRevision, v.selectedFilterRevision = v.selectionRevision, v.filterRevision
	}
	all := len(items) > 0 && v.selectedVisible == len(items)
	if w.CheckboxText("", &all) {
		for _, o := range items {
			v.selectItem(o, all)
		}
	}
	for i, k := range columns {
		widths[i] = w.WidgetBounds().W
		caption := rallyFieldLabel(v, k)
		if v.Sort == k {
			if v.Descending {
				caption += " ↓"
			} else {
				caption += " ↑"
			}
		}
		if k == "Tasks" || k == "Discussion" {
			w.Label(caption, "LC")
			continue
		}
		if w.ButtonText(caption) {
			if v.Sort == k {
				v.Descending = !v.Descending
			} else {
				v.Sort = k
				v.Descending = false
			}
		}
	}
	w.Label("Actions", "LC")
	start, end, resident := v.tableRange(len(items))
	if !resident {
		muted(w, "This page is not loaded. Use Retry to load it.", a.p)
		return
	}
	previous := ""
	linkCount := 0
	defer func() {
		clear(v.tableLinks[linkCount:])
		v.tableLinks = v.tableLinks[:linkCount]
	}()
	for row, o := range items[start:end] {
		key, name, _ := boardGroupIdentity(o, v.Group)
		if v.Group != "" && v.Group != "None" && (row == 0 || key != previous) {
			previous = key
			title(w, fmt.Sprintf("%s (%d loaded)", name, v.groupCounts[key]), a.p)
		}
		scale, face := w.Master().Style().Scaling, w.Master().Style().Font
		padding := max(1, int(6*scale))
		height := int(33 * scale)
		if v.Display.Density == "Compact" {
			padding = max(1, int(4*scale))
			height = int(28 * scale)
		}
		height = max(height, desktop.FontHeight(face)+2*padding)
		links := make([]*tableLinkLayout, len(columns))
		for i, k := range columns {
			if k == "Name" || k == "FormattedID" || k == "ObjectID" {
				if linkCount == len(v.tableLinks) {
					v.tableLinks = append(v.tableLinks, tableLinkLayout{})
				}
				layout := &v.tableLinks[linkCount]
				layout.prepare(fallback(o.String(k), "—"), max(1, widths[i]-2*padding), face)
				links[i] = layout
				height = max(height, len(layout.lines)*desktop.FontHeight(face)+2*padding)
				linkCount++
			}
		}
		w.RowScaled(height).Ratio(ratios...)
		ref := o.String("_ref")
		on := v.Selected[ref]
		if w.CheckboxText("", &on) {
			v.selectItem(o, on)
		}
		for i, k := range columns {
			if k != "Name" && a.drawInlineCell(w, v, o, k) {
				continue
			}
			value := o.String(k)
			if k == "Rank" {
				// Rank is a position in the current result, not Rally's opaque sort key.
				value = strconv.Itoa((max(1, v.Page)-1)*v.PageSize + row + 1)
			}
			if k == "Tasks" || k == "Discussion" {
				value = strconv.Itoa(o.Count(k))
			}
			if k == "Blocked" && o.Bool(k) {
				value = "Blocked"
			}
			if links[i] != nil {
				if k == "Name" && v.Detail != nil && v.Detail.inlineField == k && v.Detail.Original.String("_ref") == ref {
					a.drawInlineCell(w, v, o, k)
				} else if tableLink(w, links[i], padding, a.p) {
					if _, editable := inlineField(v, o, k); k == "Name" && editable {
						a.startInline(v, o, k, nil)
					} else {
						a.openArtifact(v, o)
					}
				}
			} else {
				fg := a.p.Text
				if k == "ScheduleState" || k == "State" {
					fg = stateColor(value, a.p)
				}
				w.LabelColored(fallback(value, "—"), "LC", fg)
			}
		}
		a.tableRowMenu(w, v, o)
	}
	if end > start {
		muted(w, fmt.Sprintf("Page totals · %d items", end-start), a.p)
		totals := tableTotals(columns, items[start:end])
		w.Row(28).Ratio(ratios...)
		w.Spacing(1)
		for _, name := range columns {
			w.Label(totals[name], "LC")
		}
		w.Spacing(1)
	}
	if len(items) == 0 && !v.Loading {
		muted(w, "No work items match the current scope and filters.", a.p)
	}
}
func (a *App) board(w *desktop.Window, v *rallyView, items []rally.Object) {
	a.drawTeamBoard(w, v, items)
}
func (a *App) metrics(w *desktop.Window, items []rally.Object, kind string) {
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
func (a *App) charts(w *desktop.Window, v *rallyView, items []rally.Object) {
	a.metrics(w, items, v.Spec.Kind)
	title(w, "Work by state", a.p)
	counts := map[string]int{}
	for _, o := range items {
		counts[o.String(v.stateField())]++
	}
	largest := 1
	for _, n := range counts {
		largest = max(largest, n)
	}
	states := make([]string, 0, len(counts))
	for state := range counts {
		states = append(states, state)
	}
	sort.Strings(states)
	for _, s := range states {
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
	ownerNames := make([]string, 0, len(owners))
	for owner := range owners {
		ownerNames = append(ownerNames, owner)
	}
	sort.Strings(ownerNames)
	for _, owner := range ownerNames {
		points := owners[owner]
		w.Row(28).Ratio(.75, .25)
		w.Label(owner, "LC")
		w.Label(fmt.Sprintf("%g points", points), "RC")
	}
	muted(w, "Partial totals: calculated only from the loaded work items.", a.p)
}
func (a *App) planning(w *desktop.Window, v *rallyView, items []rally.Object) {
	summary := v.preparePlanning(items)
	w.Row(64).Dynamic(4)
	for _, s := range []string{fmt.Sprintf("%d\nWork items", len(items)), fmt.Sprintf("%g\nPlan estimate", summary.points), fmt.Sprintf("%d\nAccepted", summary.accepted), fmt.Sprintf("%d\nBlocked", summary.blocked)} {
		w.LabelWrap(s)
	}
	title(w, "Planned scope by iteration", a.p)
	muted(w, "Totals use loaded items; capacity data is unavailable.", a.p)
	spacing := w.Master().Style().GroupWindow.Spacing.Y
	height := int(36 * w.Master().Style().Scaling)
	stride := height + spacing
	start, end := sidebarVisible(w.LayoutNextRowY(), w.Bounds.Y, w.Bounds.Y+w.Bounds.H, stride, len(a.iterations))
	sidebarSkip(w, start, stride, spacing)
	for _, it := range a.iterations[start:end] {
		points, count := summary.iterations[it.String("_ref")].points, summary.iterations[it.String("_ref")].count
		w.RowScaled(height).Ratio(.5, .25, .25)
		w.Label(it.String("Name"), "LC")
		w.Label(fmt.Sprintf("%d items", count), "LC")
		w.Label(fmt.Sprintf("%g planned points", points), "RC")
	}
	sidebarSkip(w, len(a.iterations)-end, stride, spacing)
	title(w, "Planning backlog", a.p)
	a.table(w, v, items)
}
func (a *App) timeline(w *desktop.Window, v *rallyView, items []rally.Object) {
	title(w, "Feature timeline", a.p)
	spacing := w.Master().Style().GroupWindow.Spacing.Y
	height := int(34 * w.Master().Style().Scaling)
	stride := height + spacing
	start, end := sidebarVisible(w.LayoutNextRowY(), w.Bounds.Y, w.Bounds.Y+w.Bounds.H, stride, len(items))
	sidebarSkip(w, start, stride, spacing)
	for _, o := range items[start:end] {
		w.RowScaled(height).Ratio(.42, .29, .29)
		if w.ButtonText(o.ID() + " " + cut(o.String("Name"), 50)) {
			a.openArtifact(v, o)
		}
		w.Label(fallback(o.String("PlannedStartDate"), "No start date"), "LC")
		w.Label(fallback(o.String("PlannedEndDate"), "No end date"), "LC")
	}
	sidebarSkip(w, len(items)-end, stride, spacing)
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

func (v *rallyView) quickFilterSignature() string {
	return text(v.Search) + "\x00" + v.OwnerFilter + "\x00" + v.StateFilter + fmt.Sprint(v.OnlyBlocked, v.OnlyReady)
}

func rallyColumnOptions(v *rallyView) []string {
	names := append([]string(nil), v.Columns...)
	if v.Sort == "Rank" && !contains(names, "Rank") {
		names = append(names, "Rank")
	}
	for _, f := range v.Fields {
		if !contains(names, f.Name) {
			names = append(names, f.Name)
		}
	}
	if len(v.Fields) == 0 {
		for _, name := range []string{"FormattedID", "Name", "Owner", "Project", "Discussion", "LastUpdateDate"} {
			if !contains(names, name) {
				names = append(names, name)
			}
		}
	}
	return names
}
func rallyFieldLabel(v *rallyView, name string) string {
	for _, f := range v.Fields {
		if f.Name == name && f.DisplayName != "" {
			return f.DisplayName
		}
	}
	switch name {
	case "FormattedID":
		return "ID"
	case "PlanEstimate":
		return "Plan estimate"
	case "ScheduleState":
		return "Schedule state"
	case "ObjectID":
		return "Object ID"
	}
	return name
}
