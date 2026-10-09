//go:build fltk_headless

package ui

import (
	"fmt"
	"image"
	"strings"
	"testing"
	"unsafe"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/rally"
)

func TestV25DiffDrawBoundsAndRetainsSelection(t *testing.T) {
	var source strings.Builder
	for i := range 10000 {
		fmt.Fprintf(&source, "diff --git a/file%d b/file%d\n--- a/file%d\n+++ b/file%d\n@@ -1 +1 @@\n-before\n+after\n", i, i, i, i)
	}
	raw := source.String()
	files, _ := parseDiff(raw)
	v := &fileView{Diff: files, DiffSource: raw, DiffColumns: 80}
	if len(v.Diff) != 10000 {
		t.Fatalf("files: %d", len(v.Diff))
	}
	offset := 0
	for _, f := range v.Diff {
		if unsafe.Pointer(unsafe.StringData(f.Text)) != unsafe.Add(unsafe.Pointer(unsafe.StringData(raw)), offset) {
			t.Fatal("diff eagerly copied source or made an editor")
		}
		offset += len(f.Text)
	}
	a := &App{p: colors(false)}
	scroll := 0
	h := desktop.NewHeadlessHarness(0, image.Pt(900, 600), func(w *desktop.Window) {
		v.DiffScroll = image.Pt(0, scroll)
		v.DiffRestoreScroll = true
		a.drawDiff(w, v)
	})
	h.Master().SetStyle(makeStyle(a.p, 13))
	frame := func() {
		t.Helper()
		h.Frame(false)
		if n := h.Frame(true); n > 500 || n < 10 {
			t.Fatalf("rendered %d commands at scroll %d", n, scroll)
		}
		if v.Editor != nil {
			t.Fatal("diff allocated a full text editor")
		}
	}
	frame()
	v.DiffSelection = diffSelection{Anchor: 8, End: 12, Focus: true}
	scroll = v.DiffLayout.offsets[5000]
	frame()
	scroll = 0
	frame()
	if v.DiffSelection.Anchor != 8 || v.DiffSelection.End != 12 || v.diffSelectedText() != raw[8:12] {
		t.Fatal("selection lost while scrolling")
	}
	for i := range v.Diff {
		v.Diff[i].Collapsed = true
	}
	v.invalidateDiffLayout()
	scroll = 100000
	frame()

}

func TestV25CollectionDrawUsesActualRowHeights(t *testing.T) {
	for _, tab := range []string{"Tasks", "Revisions"} {
		for _, scale := range []float64{1, 1.5, 2} {
			t.Run(fmt.Sprintf("%s/%.1f", tab, scale), func(t *testing.T) {
				a := &App{p: colors(false)}
				d := makeDetail(rally.Object{}, "HierarchicalRequirement", false)
				d.Tab = tab
				for i := range 10000 {
					d.Items = append(d.Items, rally.Object{"_ref": fmt.Sprintf("/item/%d", i), "Name": "Task", "Text": "Revision"})
				}
				scroll, extent := 0, 0
				v := newRallyView(rally.FindPage("teamboard"))
				h := desktop.NewHeadlessHarness(0, image.Pt(900, 650), func(w *desktop.Window) {
					w.RowScaled(600).Dynamic(1)
					if body := w.GroupBegin("collection", desktop.WindowNoHScrollbar); body != nil {
						body.Scrollbar.Y = scroll
						if tab == "Revisions" && d.collectionLayout == nil {
							style := body.Master().Style()
							layout := &detailCollectionLayout{Source: &d.Items[0], Length: len(d.Items), Width: body.LayoutAvailableWidth(), Size: fontPointSize(style.Font), Scale: style.Scaling, Spacing: body.WindowStyle().Spacing.Y, Tab: tab, Face: style.Font}
							prepareDetailCollection(layout, d.Items)
							d.collectionLayout = layout
						}
						top := body.LayoutNextRowY()
						a.detailCollection(body, v, d)
						extent = body.LayoutNextRowY() - top
						body.GroupEnd()
					}
				})
				style := makeStyle(a.p, 13)
				style.Scale(scale)
				h.Master().SetStyle(style)
				stride := int(33*scale) + style.GroupWindow.Spacing.Y
				if tab == "Revisions" {
					stride = max(int(34*scale), desktop.FontHeight(style.Font)+19) + style.GroupWindow.Spacing.Y
				}
				for _, pos := range []int{0, stride * 5000, stride * 9995} {
					scroll = pos
					h.Frame(false)
					if n := h.Frame(true); n > 400 || n < 5 {
						t.Fatalf("rendered %d commands at %d", n, pos)
					}
					var want int
					if tab == "Tasks" {
						want = d.taskLayout.Offsets[len(d.Items)] + int((28+30)*scale) + 2*style.GroupWindow.Spacing.Y // Add task and column headers.
					} else {
						want = d.collectionLayout.Offsets[len(d.Items)] + int(30*scale) + style.GroupWindow.Spacing.Y
					}
					if extent != want {
						t.Fatalf("wrong virtual extent: got %d, want %d", extent, want)
					}
				}
			})
		}
	}
}
