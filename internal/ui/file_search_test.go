//go:build fltk_headless

package ui

import (
	"context"
	"image"
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/command"
	"github.com/allquixotic/fastrock/internal/desktop/font"
	"github.com/allquixotic/fastrock/internal/desktop/rect"
	"github.com/allquixotic/fastrock/internal/platform"
	"github.com/allquixotic/fastrock/internal/workspace"
	xfont "golang.org/x/image/font"
	"golang.org/x/image/math/fixed"
	"golang.org/x/mobile/event/key"
)

func TestV52FileSearchUnicodeAndCancellation(t *testing.T) {
	r, err := analyzeFile(context.Background(), "Σςσ\n界🙂界\n", "σ", nil)
	if err != nil || !reflect.DeepEqual(r.matches, []uint32{0, 1, 2}) || !reflect.DeepEqual(r.lines, []uint32{0, 4, 8}) || r.runes != 8 || r.bytes != len("Σςσ\n界🙂界\n") {
		t.Fatalf("wrong Unicode index: %+v / %v", r, err)
	}
	r, err = analyzeFile(context.Background(), strings.Repeat("a", 200000)+"b", strings.Repeat("a", 2000)+"b", nil)
	if err != nil || !reflect.DeepEqual(r.matches, []uint32{198000}) {
		t.Fatal("repeated-prefix match lost", r.matches, err)
	}
	r, err = analyzeFile(context.Background(), "aaaa", "aa", nil)
	if err != nil || !reflect.DeepEqual(r.matches, []uint32{0, 1, 2}) {
		t.Fatal("overlapping matches lost")
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if _, err := analyzeFile(ctx, strings.Repeat("x", 10000), "x", nil); err != context.Canceled {
		t.Fatal("cancellation ignored", err)
	}
}

func fileSearchFixture(t *testing.T, value, query string) (*App, *fileView) {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	t.Cleanup(cancel)
	a := &App{ctx: ctx, state: workspace.NewState(), updates: make(chan func(), 64), p: colors(false)}
	v := &fileView{Editor: loadedFileEditor(value), Find: textEditor(query, false), FindOpen: true, Virtual: true}
	a.prepareFileAnalysis(v)
	drain(t, a, func() bool { return !v.Search.pending })
	return a, v
}

func TestV52FileSearchLifecycleAndNavigation(t *testing.T) {
	a, v := fileSearchFixture(t, "界 x\nx x\n", "x")
	if got := v.findLabel(); got != "1 of 3" {
		t.Fatal(got)
	}
	a.fileFind(v, false)
	if v.findLabel() != "2 of 3" {
		t.Fatal(v.findLabel())
	}
	a.fileFind(v, true)
	a.fileFind(v, true)
	if v.findLabel() != "3 of 3" {
		t.Fatal("previous did not wrap", v.findLabel())
	}
	if !strings.Contains(v.fileStatus(), "3 lines") || !strings.Contains(v.fileStatus(), "Ln 2, Col 4") {
		t.Fatal(v.fileStatus())
	}
	setText(v.Find, "missing")
	a.prepareFileAnalysis(v)
	drain(t, a, func() bool { return !v.Search.pending })
	if v.findLabel() != "No results" {
		t.Fatal(v.findLabel())
	}
	setText(v.Find, "old")
	a.prepareFileAnalysis(v)
	v.Editor = loadedFileEditor("new new")
	setText(v.Find, "new")
	a.prepareFileAnalysis(v)
	drain(t, a, func() bool { return !v.Search.pending })
	if v.Search.query != "new" || v.findLabel() != "1 of 2" {
		t.Fatal("obsolete search replaced a newer document/query", v.findLabel())
	}
	setText(v.Find, "界")
	a.prepareFileAnalysis(v)
	v.dispose()
	select {
	case update := <-a.updates:
		update()
		if v.Search.result != nil || v.Search.editor != nil {
			t.Fatal("closed file accepted a search result")
		}
	case <-time.After(3 * time.Second):
		t.Fatal("closed search did not finish")
	}

}

func TestV52FileSearchEvictionReleasesEditor(t *testing.T) {
	a, v := fileSearchFixture(t, "one two one", "one")
	v.Loaded, v.LoadedText = true, v.Editor.Snapshot()
	old := v.Editor
	a.decorateFile(v, 1, typeFace(12, monoFont))
	v.evictEditor()
	if v.Editor != nil || v.Search.editor != nil || v.Search.result != nil || old.PaintText != nil || old.PaintGutter != nil {
		t.Fatal("search retained the evicted editor")
	}
	v.installEditor(loadedFileEditor(v.LoadedText))
	a.prepareFileAnalysis(v)
	drain(t, a, func() bool { return !v.Search.pending })
	if v.findLabel() != "1 of 2" {
		t.Fatal(v.findLabel())
	}
}

func TestV52FileOverlappingMatchNavigation(t *testing.T) {
	a, v := fileSearchFixture(t, "aaaa", "aa")
	for _, want := range []string{"2 of 3", "3 of 3", "1 of 3"} {
		a.fileFind(v, false)
		if v.findLabel() != want {
			t.Fatal("overlapping match skipped", v.findLabel(), want)
		}
	}
	a.fileFind(v, true)
	if v.findLabel() != "3 of 3" {
		t.Fatal("previous did not wrap to overlapping match", v.findLabel())
	}
	v.Editor.SelectStart, v.Editor.SelectEnd, v.Editor.Cursor = 0, 0, 2
	a.fileFind(v, false)
	if v.findLabel() != "3 of 3" {
		t.Fatal("find ignored an unselected caret", v.findLabel())
	}
}

func TestV52FileFindKeyboard(t *testing.T) {
	a, v := fileSearchFixture(t, "one two one", "one")
	v.Find.Active = true
	id := a.state.Open(workspace.File, "File", "memory", "")
	a.files = map[string]*fileView{id: v}
	h := desktop.NewHeadlessHarness(0, image.Pt(800, 550), func(w *desktop.Window) {
		a.shortcuts(w)
		a.drawFile(w, v)
	})
	h.Master().SetStyle(makeStyle(a.p, 13))
	for _, tc := range []struct {
		code key.Code
		mods key.Modifiers
		want string
	}{{key.CodeF3, 0, "2 of 2"}, {key.CodeF3, key.ModShift, "1 of 2"}, {key.CodeReturnEnter, 0, "2 of 2"}, {key.CodeReturnEnter, key.ModShift, "1 of 2"}} {
		h.Key(tc.code, tc.mods)
		h.Frame(false)
		if v.findLabel() != tc.want {
			t.Fatal(tc.code, v.findLabel())
		}
	}
	h.Key(key.CodeEscape, 0)
	h.Frame(false)
	if v.FindOpen || !v.Editor.Active {
		t.Fatal("Escape did not return to the file")
	}
	h.Key(key.CodeF, platform.PrimaryModifier())
	h.Frame(false)
	if !v.FindOpen || !v.Find.Active {
		t.Fatal("Find did not request query focus")
	}
}

func TestV52FileDecorationsGeometry(t *testing.T) {
	for _, scale := range []float64{1, 1.5, 2} {
		a, v := fileSearchFixture(t, "alpha alpha\n\tbeta\n\nalpha\n", "alpha")
		v.Editor.SelectStart, v.Editor.SelectEnd, v.Editor.Cursor = 0, 0, 0
		h := desktop.NewHeadlessHarness(0, image.Pt(int(800*scale), int(500*scale)), func(w *desktop.Window) {
			a.drawFile(w, v)
		})
		style := makeStyle(a.p, 13)
		style.Scale(scale)
		h.Master().SetStyle(style)
		h.Frame(false)
		h.Frame(false)
		labels, highlights := map[string]bool{}, 0
		for _, c := range h.Commands() {
			if c.Kind == command.TextCmd {
				labels[c.Text.String] = true
			}
			if c.Kind == command.RectFilledCmd && c.RectFilled.Color == a.p.WarningSoft {
				highlights++
			}
		}
		if highlights != 3 || !labels["1"] || !labels["2"] || !labels["3"] || !labels["4"] || !labels["5"] {
			t.Fatalf("scale %v: highlights=%d labels=%v", scale, highlights, labels)
		}
		if v.Editor.GutterWidth < int(14*scale) || !strings.Contains(v.fileStatus(), "Ln 1, Col 1") {
			t.Fatal("gutter/status lost", v.Editor.GutterWidth, v.fileStatus())
		}
	}
}

type countedFileFace struct {
	xfont.Face
	advances int
}

func (f *countedFileFace) GlyphAdvance(r rune) (fixed.Int26_6, bool) {
	f.advances++
	return f.Face.GlyphAdvance(r)
}

func TestV52FileHighlightsBounded(t *testing.T) {
	value := strings.Repeat("x", 100000)
	r, err := analyzeFile(context.Background(), value, "x", nil)
	if err != nil {
		t.Fatal(err)
	}
	base := typeFace(12, monoFont)
	face := &countedFileFace{Face: base.Face}
	for _, offset := range []int{0, 50000} {
		face.advances = 0
		out := &command.Buffer{Clip: rect.Rect{W: 100, H: 20}}
		paintFileMatches(out, rect.Rect{X: -offset, W: 1000000, H: 16}, []rune(value), 0, font.Face{Face: face}, r, false, colors(false).WarningSoft)
		// An offscreen prefix may be visited once, but the suffix is never
		// measured, even with one match at every character.
		limit := offset/base.MeasureString("x") + 100
		if face.advances > limit || len(out.Commands) != 1 || out.Commands[0].Rect != (rect.Rect{W: 100, H: 16}) {
			t.Fatalf("offset %d: %d advances, commands=%+v", offset, face.advances, out.Commands)
		}
	}
}

func TestV52FileTabHighlights(t *testing.T) {
	a, v := fileSearchFixture(t, "a\tb \tc", "\t")
	v.Editor.SelectStart, v.Editor.SelectEnd, v.Editor.Cursor = 0, 0, 0
	h := desktop.NewHeadlessHarness(0, image.Pt(800, 500), func(w *desktop.Window) {
		a.drawFile(w, v)
	})
	h.Master().SetStyle(makeStyle(a.p, 13))
	h.Frame(false)
	h.Frame(false)
	highlights := 0
	for _, c := range h.Commands() {
		if c.Kind == command.RectFilledCmd && c.RectFilled.Color == a.p.WarningSoft {
			highlights++
			if c.W <= 0 {
				t.Fatal("empty tab highlight", c.Rect)
			}
		}
	}
	if highlights != 2 {
		t.Fatal("tab matches did not paint their actual tab stops", highlights)
	}
}
