//go:build fltk_headless

package ui

import (
	"context"
	"image"
	"strings"
	"testing"
	"time"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/workspace"
)

func TestV26DiffSingleLargeFileHasBoundedRows(t *testing.T) {
	raw := "@@ -0,0 +99999 @@\n" + strings.Repeat("+some text\n", 99998) + "+" + strings.Repeat("x\t🙂", 100000) + "\n"
	files, _ := parseDiff(raw)
	v := &fileView{Diff: files, DiffSource: raw, DiffColumns: 600000}
	a := &App{p: colors(false)}
	scroll := image.Point{}
	h := desktop.NewHeadlessHarness(0, image.Pt(1000, 650), func(w *desktop.Window) { v.DiffScroll = scroll; v.DiffRestoreScroll = true; a.drawDiff(w, v) })
	h.Master().SetStyle(makeStyle(a.p, 13))
	for _, at := range []image.Point{{}, {Y: 1000000}, {X: 1800000, Y: 99995 * 23}} {
		scroll = at
		h.Frame(false)
		if n := h.Frame(true); n < 10 || n > 700 {
			t.Fatalf("%d commands at %v", n, at)
		}
	}
	if v.Editor != nil {
		t.Fatal("structured diff allocated a full editor")
	}
}
func TestV26DiffTransferAndLatestPreparation(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	a := transferFixture()
	a.ctx = ctx
	a.updates = make(chan func(), 128)
	old := "diff --git a/old b/old\n@@ -1 +1 @@\n-old\n+new\n"
	current := "diff --git a/new b/new\n@@ -1 +1 @@\n-first\n+second\n"
	v := &fileView{}
	a.prepareDiff(v, old)
	a.prepareDiff(v, current)
	for range 2 {
		select {
		case apply := <-a.updates:
			apply()
		case <-time.After(time.Second):
			t.Fatal("preparation did not finish")
		}
	}
	if v.DiffSource != current || v.Diff[0].Path != "new" {
		t.Fatal("old preparation replaced newer diff")
	}
	id := a.state.Open(workspace.File, "Changes", "text:changes", "")
	v.Virtual = true
	v.Diff[0].Collapsed = true
	v.DiffScroll = image.Pt(80, 200)
	v.DiffSelection = diffSelection{Anchor: 1, End: 8, Focus: true}
	a.files[id] = v
	snapshot := transferJSON(t, a.tabSnapshot(*a.state.Current()))
	if snapshot.File == nil || snapshot.File.Text != current {
		t.Fatal("diff source missing from transfer")
	}
	b := transferFixture()
	b.ctx = ctx
	b.updates = make(chan func(), 128)
	if err := b.installTransfer(snapshot); err != nil {
		t.Fatal(err)
	}
	restored := b.files[id]
	drain(t, b, func() bool { return !restored.Loading })
	if restored.Editor != nil || restored.DiffSource != current || !restored.Diff[0].Collapsed || restored.DiffScroll != v.DiffScroll || restored.DiffSelection != v.DiffSelection {
		t.Fatalf("diff transfer lost state: %+v", restored)
	}
}
func TestV26DiffAnchorsExpandHiddenHunks(t *testing.T) {
	raw := "diff --git a/a b/a\n@@ -1 +1 @@\n-a\n+b\ndiff --git a/b b/b\n@@ -1 +1 @@\n-x\n+y\n"
	files, _ := parseDiff(raw)
	v := &fileView{Diff: files}
	v.Diff[1].Collapsed = true
	layout := v.prepareDiffLayout(1, 2, 20)
	v.DiffScroll.Y = layout.offsets[1] - 10
	v.nextDiffAnchor(false, false)
	if !v.DiffJump || v.DiffJumpFile != 1 || v.DiffJumpRow != 0 || v.Diff[1].Collapsed {
		t.Fatal("next hidden hunk not reached")
	}
}
