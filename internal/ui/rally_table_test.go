//go:build nucular_headless

package ui

import (
	"fmt"
	"image"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/command"
	"github.com/allquixotic/fastrock/internal/rally"
	"golang.org/x/mobile/event/mouse"
)

func TestV56TableLinksWrapAndOpen(t *testing.T) {
	for _, scale := range []float64{1, 1.5, 2} {
		t.Run(fmt.Sprint(scale), func(t *testing.T) {
			s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { w.WriteHeader(http.StatusNotFound) }))
			defer s.Close()
			a := presetApp(t)
			a.rallyClient, _ = rally.New(s.URL, "test", nil)
			v := newRallyView(rally.FindPage("userstories"))
			v.Columns = []string{"FormattedID", "Name", "Owner"}
			name := "The full Unicode story title 🚀 extends beyond fifty-eight characters and remains visible across wrapped lines 尾"
			v.Items = []rally.Object{
				{"_ref": s.URL + rally.WSAPI + "hierarchicalrequirement/42", "FormattedID": "US12345678901234567890", "Name": name},
				{"_ref": s.URL + rally.WSAPI + "hierarchicalrequirement/43", "FormattedID": "US43", "Name": "Second row"},
			}
			var click image.Point
			h := nucular.NewHeadlessHarness(0, image.Pt(int(500*scale), int(650*scale)), func(w *nucular.Window) {
				if click != (image.Point{}) {
					m := &w.Input().Mouse
					m.Pos = click
					m.Buttons[mouse.ButtonLeft].Clicked = true
					m.Buttons[mouse.ButtonLeft].ClickedPos = click
				}
				a.table(w, v, v.Items)
			})
			style := makeStyle(a.p, 13)
			style.Scale(scale)
			h.Master().SetStyle(style)
			h.Frame(false)
			if len(v.tableLinks) != 4 || len(v.tableLinks[0].lines) <= 1 || len(v.tableLinks[1].lines) <= 1 {
				t.Fatal("ID or name failed to wrap", v.tableLinks)
			}
			var full []string
			lastTitleBottom, secondTop := 0, 0
			for _, c := range h.Commands() {
				if c.Kind != command.TextCmd || c.Text.Foreground != a.p.Accent {
					continue
				}
				for _, line := range v.tableLinks[1].lines {
					if c.Text.String == line {
						full = append(full, line)
						lastTitleBottom = max(lastTitleBottom, c.Rect.Y+c.Rect.H)
						if click == (image.Point{}) {
							click = image.Pt(c.Rect.X+2, c.Rect.Y+c.Rect.H/2)
						}
					}
				}
				if c.Text.String == "Second row" {
					secondTop = c.Rect.Y
				}
			}
			if strings.Join(strings.Fields(strings.Join(full, " ")), " ") != strings.Join(strings.Fields(name), " ") || secondTop < lastTitleBottom {
				t.Fatal("title was cut or overlapped the next row", full, lastTitleBottom, secondTop)
			}
			cached := &v.tableLinks[1].lines[0]
			clickTarget := click
			click = image.Point{}
			h.Frame(false)
			if &v.tableLinks[1].lines[0] != cached {
				t.Fatal("idle frame rewrapped cached text")
			}
			click = clickTarget
			h.Frame(false)
			if v.Detail == nil || v.Detail.Original.String("_ref") != v.Items[0].String("_ref") {
				t.Fatal("wrapped link did not open its work item")
			}
		})
	}
}

func TestV56TableLinkCacheReflowsAndDropsOldPage(t *testing.T) {
	a := presetApp(t)
	v := newRallyView(rally.FindPage("userstories"))
	v.Columns = []string{"Name"}
	v.Items = []rally.Object{{"Name": "A long enough title to reflow across narrow columns"}, {"Name": "Second"}}
	h := nucular.NewHeadlessHarness(0, image.Pt(700, 400), func(w *nucular.Window) { a.table(w, v, v.Items) })
	h.Master().SetStyle(makeStyle(a.p, 13))
	h.Frame(false)
	firstWidth := v.tableLinks[0].width
	v.Columns = []string{"Name", "Owner", "Iteration", "ScheduleState", "PlanEstimate"}
	v.Items = v.Items[:1]
	h.Frame(false)
	if len(v.tableLinks) != 1 || v.tableLinks[0].width >= firstWidth || len(v.tableLinks[0].lines) < 2 {
		t.Fatal("column change reused stale widths or retained old rows", v.tableLinks)
	}
	v.Items[0]["Name"] = "Changed"
	h.Frame(false)
	if v.tableLinks[0].value != "Changed" {
		t.Fatal("same-page refresh retained old text")
	}
}
