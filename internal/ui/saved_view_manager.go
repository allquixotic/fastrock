package ui

import (
	"context"
	"errors"
	"fmt"
	"reflect"
	"slices"
	"sort"
	"strings"
	"unicode"
	"unicode/utf8"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/label"
	"github.com/allquixotic/fastrock/internal/desktop/rect"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/settings"
	"github.com/allquixotic/fastrock/internal/workspace"
	"golang.org/x/mobile/event/mouse"
)

func savedViewPage(id string) (rally.PageSpec, bool) {
	for _, p := range rally.Pages {
		if p.ID == id && p.ID != "customviews" {
			return p, true
		}
	}
	return rally.PageSpec{}, false
}

// A dialog carries the complete reviewed record, so a remote-window update
// cannot be overwritten by an old rename, edit or delete confirmation.
func (a *App) changeSavedView(before, next *settings.SavedView) error {
	oldIndex := -1
	if before != nil {
		for i, v := range a.prefs.Views {
			if v.Page == before.Page && v.Name == before.Name {
				if !reflect.DeepEqual(v, *before) {
					return errors.New("This view changed in another window. Reopen it before editing.")
				}
				oldIndex = i
				break
			}
		}
		if oldIndex < 0 {
			return errors.New("This saved view no longer exists.")
		}
	}
	if next != nil {
		copy := next.Clone()
		copy.Name = strings.TrimSpace(copy.Name)
		if copy.Name == "" || strings.EqualFold(copy.Name, "Standard View") || utf8.RuneCountInString(copy.Name) > 120 || strings.ContainsFunc(copy.Name, unicode.IsControl) {
			return errors.New("Use a name of 1–120 characters other than Standard View.")
		}
		if _, ok := savedViewPage(copy.Page); !ok {
			return errors.New("Choose a supported Rally page for this view.")
		}
		for i, v := range a.prefs.Views {
			if i != oldIndex && v.Page == copy.Page && v.Name == copy.Name {
				return errors.New("A view with this name already exists on that page.")
			}
		}
		next = &copy
	} else if before == nil {
		return errors.New("Choose a saved view to delete.")
	}
	previous := a.prefs.Views
	views := slices.Clone(previous)
	switch {
	case next == nil:
		views = slices.Delete(views, oldIndex, oldIndex+1)
	case oldIndex < 0:
		views = append(views, *next)
	default:
		views[oldIndex] = *next
	}
	if before != nil {
		for _, v := range a.rallyViews {
			if v.Spec.ID == before.Page && v.ViewName == before.Name {
				v.ViewName = ""
				if next != nil && next.Page == before.Page {
					v.ViewName = next.Name
				}
			}
		}
	}
	a.prefs.Views = views
	a.savedViewsRevision++
	a.savePrefs()
	return nil
}

func (a *App) reconcileSavedViewNames(old, next []settings.SavedView) {
	for _, v := range a.rallyViews {
		if v.ViewName == "" {
			continue
		}
		found := false
		for _, s := range next {
			if s.Page == v.Spec.ID && s.Name == v.ViewName {
				found = true
				break
			}
		}
		if found {
			continue
		}
		var previous *settings.SavedView
		for _, s := range old {
			if s.Page == v.Spec.ID && s.Name == v.ViewName {
				x := s
				x.Name = ""
				previous = &x
				break
			}
		}
		v.ViewName = ""
		if previous == nil {
			continue
		}
		// A unique content-preserving rename can be recognized in another
		// window's preferences too. Never replace that window's live filters.
		matches := 0
		for _, s := range next {
			x := s
			x.Name = ""
			if reflect.DeepEqual(x, *previous) {
				v.ViewName = s.Name
				matches++
			}
		}
		if matches != 1 {
			v.ViewName = ""
		}
	}
}

func (a *App) openSavedView(saved settings.SavedView) {
	page, ok := savedViewPage(saved.Page)
	if !ok {
		a.report(errors.New("This view refers to an unavailable Rally page."))
		return
	}
	if current := a.findSavedView(saved.Page, saved.Name); current == nil || !reflect.DeepEqual(*current, saved) {
		a.report(errors.New("This saved view changed. Select its current version."))
		return
	}
	id := a.state.Open(workspace.Rally, page.Title, "", page.ID)
	v := a.rallyViews[id]
	if v == nil {
		v = newRallyView(page)
		v.Display = settings.DisplayOrDefault(a.prefs.RallyDisplay)
		a.rallyViews[id] = v
	}
	if len(v.PendingCards) > 0 || v.Mutating {
		a.report(errors.New("Wait for this page's Rally update before switching views."))
		return
	}
	a.leaveDetail(v, func() {
		if current := a.findSavedView(saved.Page, saved.Name); current == nil || !reflect.DeepEqual(*current, saved) {
			a.report(errors.New("This saved view changed. Select its current version."))
			return
		}
		a.disposeDetail(v.Detail)
		v.Detail = nil
		v.applySavedView(saved)
		a.refreshRally(v)
	})
}

func (a *App) deleteSavedViewDialog(saved settings.SavedView) {
	saved = saved.Clone()
	a.confirm("Delete saved view?", "Delete "+saved.Name+"? Work items and open page filters will be kept.", func() {
		if err := a.changeSavedView(&saved, nil); err != nil {
			a.report(err)
		}
	})
}

func (a *App) savedViewDialog(before *settings.SavedView, initial settings.SavedView, active *rallyView, choosePage bool) {
	initial = initial.Clone()
	if before != nil {
		copy := before.Clone()
		before = &copy
	}
	name := textEditor(initial.Name, false)
	page, mode, group, issue := initial.Page, initial.Mode, initial.Group, ""
	titleText := "Add saved view"
	if before != nil {
		titleText = "Edit saved view"
	}
	a.window.PopupOpen(titleText, desktop.WindowTitle|desktop.WindowClosable, a.modalBounds(520, 340), false, func(w *desktop.Window) {
		w.Row(24).Dynamic(1)
		w.Label("Personal view · stored in Fastrock preferences", "LC")
		w.Row(28).Ratio(.3, .7)
		w.Label("Name", "LC")
		name.Edit(w)
		if choosePage {
			var names, ids []string
			for _, p := range rally.Pages {
				if _, ok := savedViewPage(p.ID); ok {
					names = append(names, p.Title)
					ids = append(ids, p.ID)
				}
			}
			w.Row(28).Ratio(.3, .7)
			w.Label("Page", "LC")
			if i := w.ComboSimple(names, index(ids, page), 28); ids[i] != page {
				page = ids[i]
				initial = newRallyView(rally.FindPage(page)).savedView("")
				mode, group = initial.Mode, initial.Group
			}
		}
		view := newRallyView(rally.FindPage(page))
		choices := rallyModeChoices(view)
		modes := []string{view.Spec.Mode}
		if len(choices) > 0 {
			modes = nil
			for _, c := range choices {
				modes = append(modes, c.Key)
			}
		}
		if !slices.Contains(modes, mode) {
			modes = append(modes, mode)
		}
		w.Row(28).Ratio(.3, .7)
		w.Label("Display", "LC")
		mode = modes[w.ComboSimple(modes, index(modes, mode), 28)]
		view.Group = group
		groups, names := rallyGroupChoices(view)
		w.Row(28).Ratio(.3, .7)
		w.Label("Group by", "LC")
		group = groups[w.ComboSimple(names, index(groups, group), 28)]
		if issue != "" {
			w.Row(52).Dynamic(1)
			w.LabelWrap(issue)
		}
		w.Row(30).Dynamic(2)
		if w.ButtonText("Cancel") {
			w.Close()
		}
		if primary(w, "Save view", a.p) {
			next := initial.Clone()
			next.Name, next.Page, next.Mode, next.Group = strings.TrimSpace(text(name)), page, mode, group
			if err := a.changeSavedView(before, &next); err != nil {
				issue = err.Error()
				return
			}
			if active != nil && !active.Closed {
				active.applySavedView(next)
				a.refreshRally(active)
			}
			w.Close()
		}
	})
}

func (a *App) savedViewMenu(w *desktop.Window, saved settings.SavedView) {
	if menu := w.Menu(label.T("⋮"), 210, nil); menu != nil {
		menu.Row(28).Dynamic(1)
		if menu.MenuItem(label.T("Open")) {
			a.openSavedView(saved)
		}
		if menu.MenuItem(label.T("Edit / rename…")) {
			a.savedViewDialog(&saved, saved, nil, false)
		}
		if menu.MenuItem(label.T("Copy…")) {
			copy := saved.Clone()
			copy.Name += " Copy"
			a.savedViewDialog(nil, copy, nil, false)
		}
		if menu.MenuItem(label.T("Delete…")) {
			a.deleteSavedViewDialog(saved)
		}
	}
}

func (a *App) drawSavedViewActions(w *desktop.Window, v *rallyView) {
	w.Row(26).StaticScaled(rallyButtonWidth(w, "View actions", 24))
	if menu := w.Menu(label.T("View actions"), 230, nil); menu != nil {
		menu.Row(28).Dynamic(1)
		if menu.MenuItem(label.T("Add new view…")) {
			next := v.savedView("")
			a.savedViewDialog(nil, next, v, false)
		}
		saved := a.findSavedView(v.Spec.ID, v.ViewName)
		if saved != nil && menu.MenuItem(label.T("Edit / rename current view…")) {
			a.savedViewDialog(saved, v.savedView(v.ViewName), v, false)
		}
		if menu.MenuItem(label.T("Copy current view…")) {
			next := v.savedView(fallback(v.ViewName, "Standard View") + " Copy")
			a.savedViewDialog(nil, next, v, false)
		}
		if saved != nil && menu.MenuItem(label.T("Delete current view…")) {
			a.deleteSavedViewDialog(*saved)
		}
		if menu.MenuItem(label.T("Manage views")) {
			a.openRally("customviews")
		}
	}
}

type savedViewManager struct {
	query                string
	revision, generation uint64
	valid, loading       bool
	rows                 []settings.SavedView
	issue                string
	cancel               context.CancelFunc
}

func projectSavedViews(ctx context.Context, views []settings.SavedView, query string) ([]settings.SavedView, error) {
	query = strings.ToLower(strings.TrimSpace(query))
	rows := make([]settings.SavedView, 0, len(views))
	for i, s := range views {
		if i%256 == 0 {
			if err := ctx.Err(); err != nil {
				return nil, err
			}
		}
		page, ok := savedViewPage(s.Page)
		if !ok {
			page.Title = s.Page
		}
		if query == "" || strings.Contains(strings.ToLower(s.Name+" "+page.Title+" "+s.Mode+" "+s.Group), query) {
			rows = append(rows, s)
		}
	}
	sort.SliceStable(rows, func(i, j int) bool {
		if rows[i].Page != rows[j].Page {
			return rows[i].Page < rows[j].Page
		}
		return strings.ToLower(rows[i].Name) < strings.ToLower(rows[j].Name)
	})
	return rows, ctx.Err()
}

func (a *App) prepareSavedViewManager(v *rallyView) *savedViewManager {
	if v.SavedViewManager == nil {
		v.SavedViewManager = &savedViewManager{}
	}
	m := v.SavedViewManager
	query := text(v.Search)
	if m.valid && m.query == query && m.revision == a.savedViewsRevision {
		return m
	}
	if m.cancel != nil {
		m.cancel()
	}
	base := a.ctx
	if base == nil {
		base = context.Background()
	}
	ctx, cancel := context.WithCancel(base)
	m.cancel = cancel
	m.query, m.revision, m.valid, m.loading, m.issue = query, a.savedViewsRevision, true, true, ""
	m.generation++
	generation, revision, views := m.generation, m.revision, a.prefs.Views
	a.work(func() {
		defer cancel()
		rows, err := projectSavedViews(ctx, views, query)
		a.post(func() {
			if v.Closed || v.SavedViewManager != m || m.generation != generation || a.savedViewsRevision != revision || text(v.Search) != query {
				return
			}
			m.loading = false
			if err != nil {
				m.issue = err.Error()
				return
			}
			m.rows = rows
		})
	}, func() {
		if m.generation == generation {
			m.loading = false
			m.issue = errWorkQueueFull.Error()
		}
	})
	return m
}

func (a *App) drawSavedViewManager(w *desktop.Window, v *rallyView) {
	title(w, "Custom Views", a.p)
	muted(w, "Manage your personal Rally views. Open a view to edit its filters and fields.", a.p)
	w.Row(30).Ratio(.65, .35)
	v.Search.Placeholder = "Search view name or Rally page"
	v.Search.Edit(w)
	if w.ButtonText("Add view…") {
		a.savedViewDialog(nil, newRallyView(rally.FindPage("teamboard")).savedView(""), nil, true)
	}
	m := a.prepareSavedViewManager(v)
	if m.loading {
		muted(w, "Preparing saved views…", a.p)
	}
	if m.issue != "" {
		muted(w, m.issue, a.p)
		w.Row(28).Static(90)
		if w.ButtonText("Retry") {
			m.valid = false
		}
	}
	if len(m.rows) == 0 && !m.loading {
		muted(w, "No saved views match. Add a view or change the search.", a.p)
	}
	w.RowScaled(max(80, w.LayoutAvailableHeight())).Dynamic(1)
	if body := w.GroupBegin("saved-view-list", desktop.WindowNoHScrollbar); body != nil {
		scale := w.Master().Style().Scaling
		height, gap := max(int(62*scale), 2*(desktop.FontHeight(w.Master().Style().Font)+int(8*scale))), body.WindowStyle().Spacing.Y
		first, last := sidebarVisible(body.LayoutNextRowY(), body.Bounds.Y, body.Bounds.Y+body.Bounds.H, height+gap, len(m.rows))
		sidebarSkip(body, first, height+gap, gap)
		for _, saved := range m.rows[first:last] {
			body.RowScaled(height).Ratio(.86, .14)
			b, out := body.Custom(body.CustomState())
			if out != nil {
				face := w.Master().Style().Font
				labelAt(out, rect.Rect{X: b.X, Y: b.Y, W: b.W, H: height / 2}, saved.Name, face, a.p.Accent)
				page, ok := savedViewPage(saved.Page)
				if !ok {
					page.Title = saved.Page
				}
				labelAt(out, rect.Rect{X: b.X, Y: b.Y + height/2, W: b.W, H: height / 2}, fmt.Sprintf("%s · %s · %s", page.Title, saved.Mode, fallback(saved.Group, "None")), face, a.p.Muted)
				if body.Input().Mouse.HoveringRect(b) {
					body.Tooltip(saved.Name + " · " + page.Title)
				}
				if body.Input().Mouse.Clicked(mouse.ButtonLeft, b) {
					a.openSavedView(saved)
				}
			}
			a.savedViewMenu(body, saved)
		}
		sidebarSkip(body, len(m.rows)-last, height+gap, gap)
		body.GroupEnd()
	}
}
