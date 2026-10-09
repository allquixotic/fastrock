//go:build fltk_headless

package ui

import (
	"context"
	"image"
	"strings"
	"testing"

	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/command"
	"github.com/allquixotic/fastrock/internal/desktop/rect"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/settings"
	"github.com/allquixotic/fastrock/internal/workspace"
	"golang.org/x/mobile/event/mouse"
)

func TestV54AssistantDockGeometryAndRetention(t *testing.T) {
	for _, scale := range []float64{1, 1.5, 2} {
		for _, width := range []int{360, 1200} {
			a := &App{ctx: context.Background(), p: colors(false), state: workspace.NewState()}
			a.prefs.FontSize = 13
			a.navigationFace = typeFace(13, regularFont)
			v := newRallyView(rally.FindPage("teamboard"))
			v.signature = a.rallySignature(v)
			s := &assistantView{Editor: textEditor("retained question", true), View: v, Visible: true}
			a.assistant = s
			var click image.Point
			h := desktop.NewHeadlessHarness(0, image.Pt(int(float64(width)*scale), int(900*scale)), func(w *desktop.Window) {
				if click != (image.Point{}) {
					m := &w.Input().Mouse
					m.Pos = click
					m.Buttons[mouse.ButtonLeft].Clicked = true
					m.Buttons[mouse.ButtonLeft].ClickedPos = click
				}
				a.drawRally(w, v)
			})
			style := makeStyle(a.p, 13)
			style.Scale(scale)
			h.Master().SetStyle(style)
			a.window = h.Master()
			h.Frame(false)
			var main, panel rect.Rect
			footer := false
			for _, c := range h.Commands() {
				if c.Kind != command.TextCmd {
					continue
				}
				if c.Text.String == "Connect to Rally" {
					main = c.Rect
				}
				if c.Text.String == "Rally assistant" {
					panel = c.Rect
				}
				if c.Text.String == "×" {
					click = image.Pt(c.Rect.X+2, c.Rect.Y+2)
				}
				if c.Text.String == "New conversation" && c.Rect.Y >= panel.Y && c.Rect.Y+c.Rect.H <= int(900*scale) {
					footer = true
				}
			}
			if main.W == 0 || panel.W == 0 || click == (image.Point{}) || !footer {
				t.Fatal("missing dock/main controls", scale, width, main, panel, click)
			}
			if panel.X < 0 || panel.X+panel.W > int(float64(width)*scale) || panel.Y >= int(900*scale) {
				t.Fatal("dock outside window", scale, width, panel)
			}
			if width == 1200 && panel.X <= main.X+main.W {
				t.Fatal("assistant covers main page", scale, main, panel)
			}
			if width == 360 && panel.Y <= main.Y {
				t.Fatal("narrow dock not stacked", scale, main, panel)
			}
			h.Frame(false)
			if s.Visible || text(s.Editor) != "retained question" {
				t.Fatal("closing dock discarded draft or failed to hide")
			}
			a.client = &codex.Client{}
			a.rallyClient = &rally.Client{}
			a.openAssistant(v)
			if a.assistant != s || !s.Visible || text(s.Editor) != "retained question" {
				t.Fatal("reopening lost assistant state")
			}
			a.client = nil
			a.rallyClient = nil
			click = image.Point{}
			h.Frame(false)
		}
	}
}

func TestV54AIViewAndDisclosures(t *testing.T) {
	a := &App{p: colors(false)}
	v := newRallyView(rally.FindPage("teamboard"))
	v.AIView = true
	var expanded bool
	var click image.Point
	h := desktop.NewHeadlessHarness(0, image.Pt(900, 240), func(w *desktop.Window) {
		if click != (image.Point{}) {
			m := &w.Input().Mouse
			m.Pos = click
			m.Buttons[mouse.ButtonLeft].Clicked = true
			m.Buttons[mouse.ButtonLeft].ClickedPos = click
		}
		w.Row(28).Static(210)
		a.drawRallyViewLabel(w, v)
		expanded = a.infoSection(w, "terminals", "Background terminals", "2 running")
	})
	h.Master().SetStyle(makeStyle(a.p, 13))
	h.Frame(false)
	labels, lines := []string{}, 0
	var arrow image.Point
	for _, c := range h.Commands() {
		if c.Kind == command.TextCmd {
			labels = append(labels, c.Text.String)
			if strings.HasPrefix(c.Text.String, "Background terminals") {
				arrow = image.Pt(c.Rect.X, c.Rect.Y+3)
			}
		}
		if c.Kind == command.LineCmd {
			lines++
		}
	}
	if !expanded || lines != 2 || !strings.Contains(strings.Join(labels, "|"), "AI view") || !strings.Contains(strings.Join(labels, "|"), "Saved view") || strings.ContainsAny(strings.Join(labels, "|"), "▸▾") {
		t.Fatal("labels/drawn disclosure missing", labels, lines)
	}
	click = arrow
	h.Frame(false)
	if !a.infoCollapsed["terminals"] {
		t.Fatal("drawn disclosure does not toggle")
	}
	if a.p.Hover == a.p.Selected {
		t.Fatal("selection indistinguishable from hover")
	}
	v.applySavedView(settings.SavedView{})
	if v.AIView {
		t.Fatal("manual view retained the AI label")
	}
}
