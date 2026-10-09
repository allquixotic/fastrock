package ui

import (
	"bytes"
	"errors"
	"image"
	"image/png"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/settings"
	"github.com/allquixotic/fastrock/internal/workspace"
)

func TestTranscriptViewportKeepsScaledLinePositions(t *testing.T) {
	l := &transcriptLayout{Height: 20000}
	for i := range 1000 {
		l.Lines = append(l.Lines, transcriptLine{Height: 20})
		l.LineOffsets = append(l.LineOffsets, i*20)
	}
	first, last, leading, trailing := visibleTranscriptLines(l, -10000, 0, 600, 1.5, 6)
	if first != 277 || last != 295 || leading != 277*36 || trailing != (1000-295)*36 {
		t.Fatalf("range %d:%d leading=%d trailing=%d", first, last, leading, trailing)
	}
	first, last, leading, trailing = visibleTranscriptLines(l, -40000, 0, 600, 1.5, 6)
	if first != 1000 || last != 1000 || leading != 36000 || trailing != 0 {
		t.Fatal("offscreen message changed its height")
	}
}
func TestLayoutEvictionReleasesTrimmedTextAndSelections(t *testing.T) {
	v := newChatView()
	v.Layouts = map[string]*transcriptLayout{"old": {Text: "old"}, "live": {Text: "live"}}
	v.RichSelection = transcriptSelection{BlockID: "old", Layout: v.Layouts["old"]}
	v.pruneLayouts([]workspace.Block{{ID: "live"}})
	if v.Layouts["old"] != nil || v.RichSelection.Layout != nil {
		t.Fatal("trimmed message retained")
	}
	for i := range 3 {
		v.Layouts[string(rune('a'+i))] = &transcriptLayout{Text: string(make([]byte, 4<<20)), Used: uint64(i)}
	}
	v.boundLayouts("c")
	total := 0
	for _, l := range v.Layouts {
		total += l.bytes()
	}
	if total > transcriptLayoutBudget {
		t.Fatal("byte budget exceeded")
	}
}
func TestFailedExportPreservesExistingDestination(t *testing.T) {
	path := filepath.Join(t.TempDir(), "export.csv")
	os.WriteFile(path, []byte("original"), 0600)
	err := exportFile(path, func(f *os.File) error { f.WriteString("incomplete"); return errors.New("page failed") })
	data, _ := os.ReadFile(path)
	if err == nil || string(data) != "original" {
		t.Fatal("failed export replaced destination")
	}
}
func TestSavedViewRestoresFiltersAndColumns(t *testing.T) {
	v := newRallyView(rally.FindPage("teamboard"))
	v.Timebox = "/iteration/1"
	v.OwnerFilter = "Sam"
	v.Columns = []string{"Name", "Discussion"}
	v.QueryApplied = "(Blocked = true)"
	v.Descending = true
	s := v.savedView("Mine")
	copy := newRallyView(v.Spec)
	copy.applySavedView(s)
	if copy.Timebox != v.Timebox || copy.OwnerFilter != v.OwnerFilter || len(copy.Columns) != 2 || copy.QueryApplied != v.QueryApplied || !copy.Descending {
		t.Fatal("view omitted working context")
	}
	copy.applySavedView(settings.SavedView{})
	if copy.Timebox != "" || copy.OwnerFilter != "" || copy.QueryApplied != "" || copy.Descending {
		t.Fatal("standard view kept active filters")
	}
}
func TestAttachmentCollectionPreservesLiveAndRecentFiles(t *testing.T) {
	root := t.TempDir()
	var pngBytes bytes.Buffer
	png.Encode(&pngBytes, image.NewRGBA(image.Rect(0, 0, 2, 2)))
	path, err := storeClipboardImage(root, pngBytes.Bytes())
	if err != nil {
		t.Fatal(err)
	}
	old := time.Now().Add(-48 * time.Hour)
	os.Chtimes(path, old, old)
	collectAttachments(root, map[string]bool{path: true})
	if _, err = os.Stat(path); err != nil {
		t.Fatal("removed referenced image")
	}
	collectAttachments(root, map[string]bool{})
	if _, err = os.Stat(path); !os.IsNotExist(err) {
		t.Fatal("retained expired unreferenced image")
	}
}
