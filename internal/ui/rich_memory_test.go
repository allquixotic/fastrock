//go:build nucular_headless

package ui

import (
	"strings"
	"testing"

	"github.com/allquixotic/fastrock/internal/rally"
)

func TestV7RichFieldsMaterializeOnlyWhenOpened(t *testing.T) {
	source := "<p>" + strings.Repeat("large description ", 10000) + "</p>"
	d := makeDetail(rally.Object{"Name": "story", "Description": source}, "HierarchicalRequirement", false)
	r := d.Rich["Description"]
	if r == nil || r.editor != nil || r.source != nil || r.content != "" || len(r.offsets) != 0 || d.Editors["Description"] != nil {
		t.Fatal("unopened rich field materialized duplicate buffers")
	}
	if d.dirty() || d.values()["Description"] != source || r.editor != nil {
		t.Fatal("dirty/save check opened or changed field")
	}
	r.ensureEditor()
	if text(r.editor) != string(r.doc.Text) || r.source != nil {
		t.Fatal("opening rich editor lost text or created source buffer")
	}
	r.editor.Cursor = len(r.editor.Buffer)
	r.editor.Paste("new text")
	if !d.dirty() {
		t.Fatal("lazy field changes not detected")
	}
	saved := d.values()
	next := d.Original.Clone()
	next["Description"] = saved["Description"]
	d.acceptSave(next, saved)
	if d.dirty() {
		t.Fatal("successful rich save left dirty state")
	}
	if d.Rich["Description"].editor != nil {
		t.Fatal("saved baseline created native editor")
	}
}

func TestV7UnopenedRichDraftTransfersWithoutMaterialization(t *testing.T) {
	a := snapshotFixture()
	v := addCheckpointDetail(a, "detail")
	r := v.Detail.Rich["Description"]
	r.doc.ApplyEdit(0, 0, []rune("preserved "))
	snapshot := a.sessionSnapshot()
	if r.editor != nil {
		t.Fatal("checkpoint created native buffer")
	}
	b := snapshotFixture()
	b.installTransfer(snapshot.Documents[0])
	restored := b.rallyViews[b.state.Tabs[0].ID].Detail.Rich["Description"]
	if restored.editor != nil || restored.html() != r.html() {
		t.Fatal("transfer opened or changed rich field")
	}
}

func BenchmarkV7OpenUneditedRichField(b *testing.B) {
	source := "<p>" + strings.Repeat("large description ", 10000) + "</p>"
	b.ReportAllocs()
	for b.Loop() {
		newRichEditor(source)
	}
}
