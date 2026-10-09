//go:build nucular_headless

package ui

import (
	"fmt"
	"github.com/aarzilli/nucular"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/settings"
	"github.com/allquixotic/fastrock/internal/workspace"
	"image"
	"testing"
	"time"
)

func largeSidebar(n int) *App {
	a := &App{state: workspace.NewState(), sidebarSearch: textEditor("", false), collapsed: map[string]bool{}, p: colors(false), prefs: settings.Defaults(), chats: map[string]*chatView{}}
	for i := range n {
		id := fmt.Sprint(i)
		a.state.Chats[id] = &workspace.Conversation{ID: id, Title: "Conversation " + id, Cwd: "/large-project", Updated: int64(i)}
	}
	return a
}
func TestV13SidebarVirtualizationAndInvalidation(t *testing.T) {
	a := largeSidebar(100000)
	h := nucular.NewHeadlessHarness(nucular.WindowNoScrollbar, image.Pt(280, 900), a.drawSidebar)
	a.window = h.Master()
	a.window.SetStyle(makeStyle(a.p, 13))
	commands := h.Frame(true)
	if commands > 450 {
		t.Fatalf("offscreen rows generated %d commands", commands)
	}
	a.collapsed["/large-project"] = true
	if collapsed := h.Frame(true); collapsed >= commands {
		t.Fatalf("collapse did not remove rows: %d >= %d", collapsed, commands)
	}
	a.collapsed["/large-project"] = false
	if restored := h.Frame(true); restored != commands {
		t.Fatalf("expand changed row extent: %d != %d", restored, commands)
	}
	a.state.Chats["0"].Title = "Renamed"
	a.invalidateSidebar()
	setText(a.sidebarSearch, "renamed")
	if folders := a.sidebarFolders(); len(folders) != 1 || len(folders[0].rows) != 1 || folders[0].rows[0].ID != "0" {
		t.Fatal("stale metadata/search")
	}
	a.state.Chats["0"].Archived = true
	a.invalidateSidebar()
	if len(a.sidebarFolders()) != 0 {
		t.Fatal("archive membership stale")
	}
	a.archived = true
	if len(a.sidebarFolders()) != 1 {
		t.Fatal("archived view stale")
	}
}
func TestV13SessionDoesNotCopyHistory(t *testing.T) {
	a := largeSidebar(100000)
	a.state.Open(workspace.Chat, "open", "0", "")
	a.chats["0"] = newChatView()
	setText(a.chats["0"].Editor, "live draft")
	a.state.Chats["2"].Draft = "closed draft"
	a.rememberDraft("2")
	s := a.sessionSnapshot()
	if len(s.Chats) != 2 || s.Chats["0"].Draft != "live draft" || s.Chats["2"].Draft != "closed draft" {
		t.Fatal("snapshot copied history or lost drafts")
	}
}
func BenchmarkV13SidebarToggleAndRender(b *testing.B) {
	for _, n := range []int{100, 100000} {
		b.Run(fmt.Sprint(n), func(b *testing.B) {
			a := largeSidebar(n)
			h := nucular.NewHeadlessHarness(nucular.WindowNoScrollbar, image.Pt(280, 900), a.drawSidebar)
			a.window = h.Master()
			a.window.SetStyle(makeStyle(a.p, 13))
			h.Frame(true)
			b.ReportAllocs()
			b.ResetTimer()
			for b.Loop() {
				a.collapsed["/large-project"] = !a.collapsed["/large-project"]
				h.Frame(true)
			}
		})
	}
}
func TestV13VisibleRangeAtDeepScroll(t *testing.T) {
	first, last := sidebarVisible(-2000000, 0, 900, 34, 100000)
	if last-first > 31 || first < 50000 {
		t.Fatalf("unbounded range %d:%d", first, last)
	}
}
func TestV14FooterStartsHidden(t *testing.T) {
	if settings.Defaults().StatusBar {
		t.Fatal("footer starts visible")
	}
}

func BenchmarkV13SidebarVisibilityFullWindow(b *testing.B) {
	a := largeSidebar(100000)
	a.navigationFace = makeStyle(a.p, 16).Font
	a.lastMaintenance = time.Now().Add(time.Hour)
	a.lastCheckpoint = time.Now().Add(time.Hour)
	a.state.Open(workspace.Rally, "Team Board", "", "teamboard")
	a.rallyViews = map[string]*rallyView{a.state.Active: newRallyView(rally.FindPage("teamboard"))}
	a.rallyErr = "Connect Rally in Settings"
	h := nucular.NewHeadlessHarness(nucular.WindowNoScrollbar, image.Pt(1360, 800), a.draw)
	a.window = h.Master()
	a.window.SetStyle(makeStyle(a.p, 13))
	h.Frame(true)
	b.ReportAllocs()
	b.ResetTimer()
	for b.Loop() {
		a.prefs.Sidebar = !a.prefs.Sidebar
		h.Frame(true)
	}
}

func TestV14StatusCloseFitsWindow(t *testing.T) {
	a := largeSidebar(1)
	a.navigationFace = makeStyle(a.p, 16).Font
	a.prefs.StatusBar = true
	a.lastMaintenance = time.Now().Add(time.Hour)
	a.lastCheckpoint = time.Now().Add(time.Hour)
	a.state.Open(workspace.Rally, "Board", "", "teamboard")
	a.rallyViews = map[string]*rallyView{a.state.Active: newRallyView(rally.FindPage("teamboard"))}
	var x, y, width, height int
	h := nucular.NewHeadlessHarness(nucular.WindowNoScrollbar, image.Pt(1360, 800), func(w *nucular.Window) { a.draw(w); r := w.LastWidgetBounds; x, y, width, height = r.X, r.Y, r.W, r.H })
	a.window = h.Master()
	a.window.SetStyle(makeStyle(a.p, 13))
	h.Frame(true)
	if x < 0 || x+width > 1360 || y < 0 || y+height > 800 {
		t.Fatalf("close outside window: %d,%d %dx%d", x, y, width, height)
	}
	t.Logf("close bounds: %d,%d %dx%d", x, y, width, height)
}
