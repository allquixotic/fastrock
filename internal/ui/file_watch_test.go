package ui

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
	"unicode/utf8"

	"github.com/allquixotic/fastrock/internal/workspace"
)

func fileFixture(t *testing.T, value string) (*App, *fileView) {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	t.Cleanup(cancel)
	a := transferFixture()
	a.ctx, a.updates = ctx, make(chan func(), 32)
	path := filepath.Join(t.TempDir(), "notes.md")
	if err := os.WriteFile(path, []byte(value), 0600); err != nil {
		t.Fatal(err)
	}
	id := a.state.Open(workspace.File, "notes.md", path, "")
	v := &fileView{Path: path, Wrap: true, Find: textEditor("", false)}
	a.files[id] = v
	a.reloadFile(v)
	drain(t, a, func() bool { return !v.Loading })
	if v.Error != "" || !v.Loaded {
		t.Fatalf("initial load failed: %s", v.Error)
	}
	return a, v
}

func pollFiles(a *App) {
	a.fileWatcher.poll(a.ctx)
	for {
		select {
		case update := <-a.updates:
			update()
		default:
			return
		}
	}
}

func TestV30FileChangesKeepLastLoadedText(t *testing.T) {
	a, v := fileFixture(t, "first version")
	v.Editor.Cursor, v.Editor.SelectStart, v.Editor.SelectEnd = 7, 2, 7
	v.Editor.Scrollbar.Y = 40
	if err := os.WriteFile(v.Path, []byte("a different version"), 0600); err != nil {
		t.Fatal(err)
	}
	pollFiles(a)
	if v.FileNotice != fileChangedNotice || v.fileText() != "first version" {
		t.Fatal("change silently replaced loaded text or lacked a banner")
	}
	// Eviction must not reread changed data, or lose the last version after a
	// later deletion. It only discards reconstructible layout/rune storage.
	v.evictEditor()
	if v.Editor != nil {
		t.Fatal("native editor was not evicted")
	}
	if err := os.Remove(v.Path); err != nil {
		t.Fatal(err)
	}
	pollFiles(a)
	a.ensureFileEditor(v)
	drain(t, a, func() bool { return !v.Loading })
	if v.FileNotice != fileMissingNotice || text(v.Editor) != "first version" || v.Editor.Cursor != 7 || v.Editor.SelectStart != 2 || v.Editor.Scrollbar.Y != 40 {
		t.Fatal("eviction/deletion lost last loaded text or its position")
	}
	oldOffset := v.Offset
	a.reloadFile(v)
	drain(t, a, func() bool { return !v.Loading })
	if v.Error == "" || v.Offset != oldOffset || text(v.Editor) != "first version" {
		t.Fatal("failed reload changed the loaded version")
	}
	if err := os.WriteFile(v.Path, []byte("new version"), 0600); err != nil {
		t.Fatal(err)
	}
	a.reloadFile(v)
	drain(t, a, func() bool { return !v.Loading })
	if v.Error != "" || v.FileNotice != "" || v.fileText() != "new version" {
		t.Fatal("successful reload failed to rebaseline")
	}
	pollFiles(a)
	if v.FileNotice != "" {
		t.Fatal("old watch callback revived a stale banner")
	}
}

func TestV30FilePagingRejectsChangedVersionsAndBoundsUTF8(t *testing.T) {
	a, v := fileFixture(t, strings.Repeat("a", 300<<10))
	before, offset := v.fileText(), v.Offset
	if !v.More {
		t.Fatal("fixture did not span pages")
	}
	if err := os.WriteFile(v.Path, []byte("changed"), 0600); err != nil {
		t.Fatal(err)
	}
	a.loadFilePage(v, true)
	drain(t, a, func() bool { return !v.Loading })
	if !strings.Contains(v.Error, "changed") || v.fileText() != before || v.Offset != offset || !v.More {
		t.Fatal("paging combined different file versions")
	}
	path := filepath.Join(t.TempDir(), "utf8.txt")
	if err := os.WriteFile(path, []byte(strings.Repeat("a", maxFileBytes-1)+"界tail"), 0600); err != nil {
		t.Fatal(err)
	}
	page, err := readFilePage(path, maxFileBytes-2, "", fileStamp{}, false)
	if err != nil || !page.limit || page.more || !utf8.ValidString(page.value) || page.value != "a" {
		t.Fatalf("UTF-8 boundary failed: %+v, %v", page, err)
	}
}

func TestV30FileWatcherReplacementCloseAndStaleResults(t *testing.T) {
	a, v := fileFixture(t, "original")
	info, err := os.Stat(v.Path)
	if err != nil {
		t.Fatal(err)
	}
	replacement := v.Path + ".new"
	if err := os.WriteFile(replacement, []byte("replaced"), 0600); err != nil {
		t.Fatal(err)
	}
	if err := os.Chtimes(replacement, info.ModTime(), info.ModTime()); err != nil {
		t.Fatal(err)
	}
	if err := os.Rename(replacement, v.Path); err != nil {
		t.Fatal(err)
	}
	pollFiles(a)
	if v.FileNotice != fileChangedNotice {
		t.Fatal("same-size/time atomic replacement was not detected")
	}
	// Complete a load but close before publishing it.
	a.reloadFile(v)
	var publish func()
	select {
	case publish = <-a.updates:
	case <-time.After(3 * time.Second):
		t.Fatal("load did not publish")
	}
	a.closeTabNow(a.state.Current().ID)
	publish()
	if !v.Closed || v.fileText() != "original" || len(a.files) != 0 {
		t.Fatal("closed view accepted a late load")
	}
	a.fileWatcher.mu.Lock()
	remaining := len(a.fileWatcher.entries)
	a.fileWatcher.mu.Unlock()
	if remaining != 0 {
		t.Fatal("closed file retained its watch")
	}
}

func TestV30WatcherUnsubscribeAndRebaseline(t *testing.T) {
	path := filepath.Join(t.TempDir(), "file")
	if err := os.WriteFile(path, []byte("original"), 0600); err != nil {
		t.Fatal(err)
	}
	info, _ := os.Stat(path)
	w := &fileWatcher{}
	var notices []string
	stop := w.watch(path, stampFile(info), "", func(s string) { notices = append(notices, s) })
	w.poll(context.Background())
	if len(notices) != 0 {
		t.Fatal("unchanged file emitted a banner")
	}
	if err := os.Remove(path); err != nil {
		t.Fatal(err)
	}
	w.poll(context.Background())
	w.poll(context.Background())
	if len(notices) != 1 || notices[0] != fileMissingNotice {
		t.Fatal("missing file notice repeated or absent")
	}
	stop()
	if err := os.WriteFile(path, []byte("new"), 0600); err != nil {
		t.Fatal(err)
	}
	w.poll(context.Background())
	if len(notices) != 1 {
		t.Fatal("unsubscribed file emitted a notice")
	}
	if _, err := readFilePage(path, 0, "", stampFile(info), false); !errors.Is(err, errFileChanged) {
		t.Fatal("old file stamp accepted a replacement")
	}
}

func TestV30FileTransferRetainsWatchAndEvictedText(t *testing.T) {
	a, v := fileFixture(t, "last loaded text")
	v.FileNotice = fileMissingNotice
	v.Editor.Cursor = 4
	v.evictEditor()
	saved := transferJSON(t, a.tabSnapshot(*a.state.Current()))
	b := transferFixture()
	if err := b.installTransfer(saved); err != nil {
		t.Fatal(err)
	}
	got := b.files[a.state.Current().ID]
	if got.fileText() != "last loaded text" || got.Editor.Cursor != 4 || got.FileNotice != fileMissingNotice || !got.Stamp.matches(v.Stamp) {
		t.Fatal("transfer dropped file text, position, or disk baseline")
	}
	// Finalization replaces the initially installed transfer in the same tab.
	// Its old subscription must not retain the discarded view.
	if err := a.installTransfer(saved); err != nil {
		t.Fatal(err)
	}
	a.fileWatcher.mu.Lock()
	watches := len(a.fileWatcher.entries)
	a.fileWatcher.mu.Unlock()
	if !v.Closed || watches != 1 {
		t.Fatal("transfer finalization leaked the replaced file view/watch")
	}
}

func TestV30TextSaveNamesAndAtomicResult(t *testing.T) {
	for input, want := range map[string]string{
		"Notes": "Notes.md", "/tmp/example.txt": "example.txt", `C:\Users\test\report.md`: "report.md",
		"CON": "_CON.md", "COM1.txt": "_COM1.txt", "  ..  ": "document.md", "a:*?b": "a___b.md", "A:notes": "A_notes.md",
	} {
		if got := suggestedTextName(input); got != want {
			t.Errorf("%q => %q, want %q", input, got, want)
		}
	}
	long := suggestedTextName(strings.Repeat("界", 200))
	if !utf8.ValidString(long) || len(long) > 183 {
		t.Fatal("suggested filename broke UTF-8 or exceeded byte limit")
	}
	a, v := fileFixture(t, "saved text")
	path := filepath.Join(t.TempDir(), "saved.md")
	a.writeTextFile(path, v.fileText())
	drain(t, a, func() bool { return a.toast != "" })
	data, err := os.ReadFile(path)
	if err != nil || string(data) != "saved text" || a.toast != "Saved" {
		t.Fatalf("save did not report committed contents: %s, %v", data, err)
	}
	a.toast = ""
	a.writeTextFile(filepath.Join(path, "invalid"), "must fail")
	drain(t, a, func() bool { return a.toast != "" })
	if a.toast == "Saved" {
		t.Fatal("failed save reported success")
	}
}

func TestV30LossyTextPreservesPhysicalPageOffsets(t *testing.T) {
	path := filepath.Join(t.TempDir(), "windows-text.txt")
	// An invalid byte before the page boundary expands to a three-byte replacement
	// character. The next seek must still use the original file byte offset.
	raw := append([]byte("caf\xe9"), []byte(strings.Repeat("x", (256<<10)-4))...)
	raw = append(raw, []byte("tail\xff")...)
	if err := os.WriteFile(path, raw, 0600); err != nil {
		t.Fatal(err)
	}
	if _, err := readFilePage(path, 0, "", fileStamp{}, false); !errors.Is(err, errFileNotUTF8) {
		t.Fatal("non-UTF-8 input was silently decoded")
	}
	first, err := readFilePage(path, 0, "", fileStamp{}, true)
	if err != nil || first.next != 256<<10 || !first.more || !strings.HasPrefix(first.value, "caf�") {
		t.Fatalf("invalid first page: offset=%d more=%v error=%v", first.next, first.more, err)
	}
	last, err := readFilePage(path, first.next, first.value, first.stamp, true)
	if err != nil || last.more || last.next != int64(len(raw)) || !strings.HasSuffix(last.value, "tail�") || !utf8.ValidString(last.value) {
		t.Fatal("lossy decoding corrupted offsets or dropped data")
	}
	// A user-selected retry clears the decoding failure and can transfer the
	// same setting to another window without a second lossy conversion.
	a, v := fileFixture(t, "initial")
	if err := os.WriteFile(v.Path, []byte("caf\xe9"), 0600); err != nil {
		t.Fatal(err)
	}
	a.reloadFile(v)
	drain(t, a, func() bool { return !v.Loading })
	if !v.NonUTF8 || v.fileText() != "initial" {
		t.Fatal("failed decoding hid the old version or the recovery choice")
	}
	v.Lossy = true
	a.reloadFile(v)
	drain(t, a, func() bool { return !v.Loading })
	if v.NonUTF8 || v.Error != "" || v.fileText() != "caf�" {
		t.Fatal("lossy retry did not publish readable text")
	}
	b := transferFixture()
	if err := b.installTransfer(transferJSON(t, a.tabSnapshot(*a.state.Current()))); err != nil {
		t.Fatal(err)
	}
	got := b.files[a.state.Current().ID]
	if !got.Lossy || got.fileText() != "caf�" {
		t.Fatal("transfer lost explicit lossy decoding choice")
	}
}
