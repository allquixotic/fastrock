//go:build nucular_headless

package ui

import (
	"encoding/json"
	"fmt"
	"image"
	"net/http"
	"net/http/httptest"
	"slices"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/command"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/richtext"
)

func TestV47CollectionSwitchKeepsResultsScoped(t *testing.T) {
	a, v := keyboardBoard(t, 0)
	d := makeDetail(rally.Object{"_ref": "/slm/webservice/v2.0/story/1", "Name": "Parent", "Tasks": map[string]any{"_ref": "/slm/webservice/v2.0/tasks"}, "Defects": map[string]any{"_ref": "/slm/webservice/v2.0/defects"}}, "HierarchicalRequirement", false)
	v.Detail = d
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/slm/webservice/v2.0/defects" {
			w.WriteHeader(403)
			return
		}
		json.NewEncoder(w).Encode(map[string]any{"QueryResult": map[string]any{"Results": []rally.Object{{"_ref": "/slm/webservice/v2.0/task/1", "Name": "previous task"}}, "TotalResultCount": 1}})
	}))
	defer server.Close()
	a.rallyClient, _ = rally.New(server.URL, "fixture", nil)
	d.Tab = "Tasks"
	a.loadCollection(d)
	drain(t, a, func() bool { return !d.CollectionLoading })
	if len(d.Items) != 1 || d.Error != "" {
		t.Fatal("fixture did not load its first collection", d.Error)
	}
	d.Tab = "Defects"
	a.loadCollection(d)
	drain(t, a, func() bool { return !d.CollectionLoading })
	h := nucular.NewHeadlessHarness(0, image.Pt(900, 600), func(w *nucular.Window) { a.detailCollection(w, v, d) })
	h.Master().SetStyle(makeStyle(a.p, 13))
	h.Frame(false)
	for _, c := range h.Commands() {
		if c.Kind == command.TextCmd && strings.Contains(c.Text.String, "previous task") {
			t.Fatal("old tasks appeared under Defects")
		}
	}
}

func TestV47CollectionConnectionChange(t *testing.T) {
	a, _ := keyboardBoard(t, 0)
	d := makeDetail(rally.Object{"Tasks": map[string]any{"_ref": "/slm/webservice/v2.0/tasks"}}, "HierarchicalRequirement", false)
	d.Tab = "Tasks"
	started, release := make(chan struct{}), make(chan struct{})
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		close(started)
		<-release
		fmt.Fprint(w, `{"QueryResult":{"Results":[{"Name":"old endpoint"}],"TotalResultCount":1}}`)
	}))
	defer server.Close()
	a.rallyClient, _ = rally.New(server.URL, "fixture", nil)
	a.loadCollection(d)
	<-started
	a.rallyClient = nil
	close(release)
	drain(t, a, func() bool { return !d.CollectionLoading })
	if len(d.Items) != 0 || !strings.Contains(d.Error, "connection changed") {
		t.Fatal("old endpoint data published", d.Error, d.Items)
	}
}

func TestV47DetailTabsFollowSchemaAndCounts(t *testing.T) {
	d := makeDetail(rally.Object{"Tasks": map[string]any{"_ref": "/tasks", "Count": float64(3)}, "TestCases": map[string]any{"Count": float64(2)}, "Defects": map[string]any{"Count": float64(1)}, "RevisionHistory": map[string]any{"_ref": "/history"}}, "HierarchicalRequirement", false)
	var keys []string
	for _, tab := range detailTabs(d) {
		keys = append(keys, tab.Key)
		if tab.Key == "Tasks" && d.tabLabel(tab) != "Tasks (3)" {
			t.Fatal(d.tabLabel(tab))
		}
		if tab.Key == "Revisions" && d.tabLabel(tab) != "Revision History (?)" {
			t.Fatal(d.tabLabel(tab))
		}
	}
	if !slices.Contains(keys, "Test Cases") || !slices.Contains(keys, "Defects") {
		t.Fatal(keys)
	}
	d.collectionCounts = map[string]int{"Revisions": 7}
	for _, tab := range detailTabs(d) {
		if tab.Key == "Revisions" && d.tabLabel(tab) != "Revision History (7)" {
			t.Fatal(d.tabLabel(tab))
		}
	}
	d.Kind = "Task"
	d.Fields = []rally.Field{{Name: "Children", AttributeType: "COLLECTION"}, {Name: "Tasks", AttributeType: "COLLECTION"}}
	for _, tab := range detailTabs(d) {
		if tab.Key == "Tasks" || tab.Key == "Children" {
			t.Fatal("task exposes invalid collection", tab)
		}
	}
	d.New = true
	if len(detailTabs(d)) != 2 {
		t.Fatal("new item exposes unsaved collections")
	}
	d.New, d.Kind = false, "HierarchicalRequirement"
	for _, scale := range []float64{1, 1.5, 2} {
		t.Run(fmt.Sprint(scale), func(t *testing.T) {
			a := &App{p: colors(false)}
			h := nucular.NewHeadlessHarness(0, image.Pt(int(380*scale), 450), func(w *nucular.Window) { a.drawDetailTabs(w, d) })
			style := makeStyle(a.p, 13)
			style.Scale(scale)
			h.Master().SetStyle(style)
			h.Frame(false)
			for _, c := range h.Commands() {
				if c.Kind == command.TextCmd && (c.Rect.X < 0 || c.Rect.X+c.Rect.W > int(380*scale)) {
					t.Fatal("tab overflows viewport", c.Text.String, c.Rect)
				}
			}
		})
	}
}

func TestV47FeatureAndEpicChildCollections(t *testing.T) {
	a, _ := keyboardBoard(t, 0)
	var paths []string
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		paths = append(paths, r.URL.Path)
		json.NewEncoder(w).Encode(map[string]any{"QueryResult": map[string]any{"Results": []any{}, "TotalResultCount": 0}})
	}))
	defer server.Close()
	a.rallyClient, _ = rally.New(server.URL, "fixture", nil)
	for _, kind := range []string{"PortfolioItem/Feature", "PortfolioItem/Epic"} {
		d := makeDetail(rally.Object{"UserStories": map[string]any{"_ref": "/slm/webservice/v2.0/userstories"}, "Children": map[string]any{"_ref": "/slm/webservice/v2.0/features"}}, kind, false)
		d.Tab = "Children"
		a.loadCollection(d)
		drain(t, a, func() bool { return !d.CollectionLoading })
		if d.Error != "" {
			t.Fatal(d.Error)
		}
	}
	if !slices.Equal(paths, []string{"/slm/webservice/v2.0/userstories", "/slm/webservice/v2.0/features"}) {
		t.Fatal(paths)
	}
}

func TestV47CollectionTextLayoutAndVirtualBounds(t *testing.T) {
	for _, tab := range []string{"Discussions", "Revisions"} {
		for _, scale := range []float64{1, 1.5, 2} {
			t.Run(fmt.Sprintf("%s/%g", tab, scale), func(t *testing.T) {
				a, v := keyboardBoard(t, 0)
				d := makeDetail(rally.Object{}, "HierarchicalRequirement", false)
				v.Detail = d
				d.Tab = tab
				for i := range 128 {
					d.Items = append(d.Items, rally.Object{"_ref": fmt.Sprintf("/post/%d", i), "User": map[string]any{"_refObjectName": "Alex Case"}, "CreationDate": "2026-10-09T12:30:00Z", "RevisionNumber": fmt.Sprint(i + 1), "Text": "<p><strong>Bold</strong> and literal *asterisks*</p><p>Second paragraph</p>"})
				}
				style := makeStyle(a.p, 13)
				style.Scale(scale)
				var layout *detailCollectionLayout
				offset, extent := 0, 0
				h := nucular.NewHeadlessHarness(0, image.Pt(900, 600), func(w *nucular.Window) {
					w.RowScaled(580).Dynamic(1)
					if body := w.GroupBegin("posts", nucular.WindowNoHScrollbar); body != nil {
						body.Scrollbar.Y = offset
						if layout == nil {
							layout = &detailCollectionLayout{Source: &d.Items[0], Length: len(d.Items), Width: body.LayoutAvailableWidth(), Size: fontPointSize(style.Font), Scale: scale, Spacing: style.GroupWindow.Spacing.Y, Tab: tab, Face: style.Font}
							prepareDetailCollection(layout, d.Items)
							d.collectionLayout = layout
						}
						start := body.LayoutNextRowY()
						a.drawCollectionText(body, v, d, d.Items)
						extent = body.LayoutNextRowY() - start
						body.GroupEnd()
					}
				})
				h.Master().SetStyle(style)
				h.Frame(false)
				if !strings.Contains(layout.Rows[0].Body.Plain, "literal *asterisks*") || layout.Rows[0].Initials != "AC" || layout.Rows[0].Date != "2026-10-09 12:30 UTC" {
					t.Fatal(layout.Rows[0])
				}
				bold := false
				for _, line := range layout.Rows[0].Body.Lines {
					for _, run := range line.Runs {
						bold = bold || run.Format.Style&richtext.Bold != 0
					}
				}
				if !bold {
					t.Fatal("HTML formatting lost")
				}

				for _, scroll := range []int{0, layout.Offsets[64], layout.Offsets[120]} {
					offset = scroll
					h.Frame(false)
					h.Frame(true)
					if count := len(h.Commands()); count > 350 || count < 8 {
						t.Fatal("virtual draw commands", count)
					}
					want := layout.Offsets[len(layout.Rows)]
					if tab == "Revisions" {
						want += int(30*scale) + style.GroupWindow.Spacing.Y
					}
					if extent != want {
						t.Fatal("virtual extent", extent, want)
					}
					for _, c := range h.Commands() {
						if c.Kind == command.TextCmd && (c.Text.String == "Edit" || c.Text.String == "Preview" || c.Text.String == "HTML") {
							t.Fatal("read-only collection exposes editor")
						}
					}
				}
			})
		}
	}
	short := &detailCollectionLayout{Width: 640, Size: 13, Scale: 1, Tab: "Discussions"}
	prepareDetailCollection(short, []rally.Object{{"Text": "<p>Short</p>"}, {"Text": strings.Repeat("long text ", 100)}, {"Text": strings.Repeat("X", 20000)}})
	if short.Rows[0].Height >= short.Rows[1].Height || !short.Rows[2].Truncated {
		t.Fatal("content heights or preview bound", short.Rows)
	}
}

func TestV47CollectionLayoutIgnoresStaleResults(t *testing.T) {
	a, v := keyboardBoard(t, 0)
	d := makeDetail(rally.Object{}, "HierarchicalRequirement", false)
	v.Detail = d
	d.Tab = "Revisions"
	d.Items = []rally.Object{{"Text": "old"}}
	h := nucular.NewHeadlessHarness(0, image.Pt(900, 600), func(w *nucular.Window) { a.collectionTextLayout(w, d, d.Items) })
	h.Master().SetStyle(makeStyle(a.p, 13))
	h.Frame(false)
	d.collectionGeneration++
	d.Tab = "Discussions"
	select {
	case f := <-a.updates:
		f()
	case <-time.After(time.Second):
		t.Fatal("layout did not finish")
	}
	if d.collectionLayout != nil {
		t.Fatal("old result crossed collection generation")
	}
	h.Frame(false)
	drain(t, a, func() bool { return d.collectionLayout != nil })
	if d.collectionLayout.Tab != "Discussions" {
		t.Fatal("wrong tab")
	}
	h.Master().SetStyle(makeStyle(a.p, 18))
	h.Frame(false)
	h.Master().SetStyle(makeStyle(a.p, 22))
	h.Frame(false)
	drain(t, a, func() bool { return d.collectionLayout != nil && d.collectionLayout.Size == 22 })
	if d.collectionLayout.Size != fontPointSize(h.Master().Style().Font) {
		t.Fatal("outdated font result was published")
	}
}

func TestV47DiscussionDeleteLifecycle(t *testing.T) {
	a, v := keyboardBoard(t, 0)
	d := makeDetail(rally.Object{"_ref": "/slm/webservice/v2.0/hierarchicalrequirement/1"}, "HierarchicalRequirement", false)
	v.Detail = d
	d.Tab = "Discussions"
	post := rally.Object{"_ref": "/slm/webservice/v2.0/conversationpost/7"}
	d.Items = []rally.Object{post}
	var mu sync.Mutex
	deletes := 0
	fail := false
	started, release := make(chan struct{}), make(chan struct{})
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method == "DELETE" {
			mu.Lock()
			deletes++
			n := deletes
			reject := fail
			mu.Unlock()
			if n == 1 {
				close(started)
				<-release
			}
			if reject {
				w.WriteHeader(403)
				fmt.Fprint(w, `{"OperationResult":{"Errors":["denied"]}}`)
				return
			}
			fmt.Fprint(w, `{"OperationResult":{"Errors":[]}}`)
			return
		}
		fmt.Fprint(w, `{"QueryResult":{"Results":[],"TotalResultCount":0}}`)
	}))
	defer server.Close()
	a.rallyClient, _ = rally.New(server.URL, "fixture", nil)
	a.deleteDiscussion(v, d, post)
	select {
	case <-started:
	case <-time.After(time.Second):
		t.Fatal("delete did not start")
	}
	a.deleteDiscussion(v, d, post)
	d.CommentRich = newRichEditor("<p>Draft while deleting</p>")
	close(release)
	drain(t, a, func() bool { return !d.Pending && !d.CollectionLoading })
	mu.Lock()
	n := deletes
	fail = true
	mu.Unlock()
	if n != 1 || len(d.Items) != 0 || d.collectionCounts["Discussions"] != 0 || d.CommentRich.html() != "<p>Draft while deleting</p>" {
		t.Fatal("delete lost draft, duplicated, or failed refresh")
	}
	d.Items = []rally.Object{post}
	a.deleteDiscussion(v, d, post)
	drain(t, a, func() bool { return !d.Pending })
	if !strings.Contains(d.Error, "denied") || len(d.Items) != 1 {
		t.Fatal("failed deletion discarded rows", d.Error)
	}
	a.cancel()
	a.deleteDiscussion(v, d, post)
	if d.Pending {
		t.Fatal("cancelled admission left delete pending")
	}
}
