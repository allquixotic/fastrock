package ui

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/allquixotic/fastrock/internal/richtext"
	"github.com/allquixotic/fastrock/internal/settings"
	"github.com/allquixotic/fastrock/internal/workspace"
)

func recoveryFixture(draft string) session {
	return session{SchemaVersion: sessionSchemaVersion, Chats: map[string]*workspace.Conversation{"chat": {ID: "chat", Draft: draft, Queue: []workspace.Draft{{ID: "q", Text: "queued", Status: "pending"}}}}, Tabs: []workspace.Tab{{ID: "tab", Kind: workspace.Chat, Target: "chat"}}, Active: "tab"}
}

func TestV24SessionBackupAndInterruptedWriteRecovery(t *testing.T) {
	store := &settings.Store{Dir: t.TempDir()}
	path := filepath.Join(store.Dir, "session.json")
	if err := writeSessionFile(store, "session.json", recoveryFixture("first")); err != nil {
		t.Fatal(err)
	}
	if err := writeSessionFile(store, "session.json", recoveryFixture("second")); err != nil {
		t.Fatal(err)
	}
	// A crash before atomic rename leaves an unrelated temporary file.
	if err := os.WriteFile(filepath.Join(store.Dir, ".fastrock-interrupted"), []byte(`{"unfinished":`), 0600); err != nil {
		t.Fatal(err)
	}
	if got := readSessions(store.Dir); got.primary == nil || got.primary.Chats["chat"].Draft != "second" {
		t.Fatal("temporary file replaced committed snapshot")
	}
	if err := os.WriteFile(path, []byte(`{"damaged":`), 0600); err != nil {
		t.Fatal(err)
	}
	got := readSessions(store.Dir)
	if got.blocked || got.primary == nil || got.primary.Chats["chat"].Draft != "first" || !strings.Contains(got.problem, "last-good") {
		t.Fatalf("backup recovery failed: %+v", got)
	}
	saved, _ := filepath.Glob(path + ".recovery-*")
	if len(saved) != 1 {
		t.Fatal("damaged original not preserved")
	}
	a := transferFixture()
	a.applySessions(got)
	if !a.state.Chats["chat"].QueuePaused || a.state.Chats["chat"].Queue[0].Status != "unconfirmed" {
		t.Fatal("recovered queue resumed itself")
	}
}

func TestV24SessionSalvagesIndependentDocuments(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "session.json")
	good := tabTransfer{SchemaVersion: transferSchemaVersion, Tab: workspace.Tab{ID: "good", Kind: workspace.Rally, Page: "teamboard"}, Rally: &rallyTransfer{Detail: &detailTransfer{Original: map[string]any{"Name": "Story"}, Kind: "HierarchicalRequirement", Values: map[string]string{"Name": "unsaved"}, Rich: map[string]richtext.Saved{}}}}
	encoded, err := json.Marshal(good)
	if err != nil {
		t.Fatal(err)
	}
	raw := []byte(`{"schemaVersion":2,"Tabs":[{"ID":"good","Kind":"rally","Page":"teamboard"},{"ID":"bad","Kind":"rally","Page":"teamboard"}],"Active":"good","Documents":[` + string(encoded) + `,{"Tab":{"ID":"bad","Kind":"rally","Page":"teamboard"},"Rally":{"Detail":{"Rich":{"Description":{"Changed":true,"Text":"x","Spans":[{"End":500}]}}}}}],"Chats":{"broken":4,"chat":{"ID":"chat","Draft":"safe"}}}`)
	if err = os.WriteFile(path, raw, 0600); err != nil {
		t.Fatal(err)
	}
	loaded := readSessions(dir)
	if loaded.blocked || loaded.primary == nil || len(loaded.primary.Documents) != 1 || len(loaded.primary.Chats) != 1 || loaded.primary.Documents[0].Rally.Detail.Values["Name"] != "unsaved" || loaded.problem == "" {
		t.Fatalf("healthy records lost: %+v", loaded)
	}
	preserved, _ := filepath.Glob(path + ".recovery-*")
	if len(preserved) != 1 {
		t.Fatal("partial original missing")
	}
	original, err := os.ReadFile(preserved[0])
	if err != nil || !bytes.Equal(original, raw) {
		t.Fatal("partial original modified")
	}
	a := transferFixture()
	a.applySessions(loaded)
	if text(a.rallyViews["good"].Detail.Editors["Name"]) != "unsaved" {
		t.Fatal("salvaged document not installed")
	}
}

func TestV24FutureSessionAndTransferNeverOverwriteCurrent(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "session.json")
	original := []byte(`{"schemaVersion":900,"future":"unknown draft"}`)
	if err := os.WriteFile(path, original, 0600); err != nil {
		t.Fatal(err)
	}
	got := readSessions(dir)
	if !got.blocked || got.primary != nil {
		t.Fatal("future schema accepted")
	}
	if err := writeSessionFile(&settings.Store{Dir: dir}, "session.json", recoveryFixture("new")); err == nil {
		t.Fatal("future session overwritten")
	}
	unchanged, _ := os.ReadFile(path)
	if !bytes.Equal(original, unchanged) {
		t.Fatal("future bytes changed")
	}
	a := transferFixture()
	if err := a.installTransfer(tabTransfer{SchemaVersion: 900, Tab: workspace.Tab{ID: "future", Kind: workspace.Rally, Page: "teamboard"}, Rally: &rallyTransfer{}}); err == nil || len(a.state.Tabs) != 0 {
		t.Fatal("future transfer partially installed")
	}
}

func TestV24BackupFailureLeavesCommittedSession(t *testing.T) {
	store := &settings.Store{Dir: t.TempDir()}
	path := filepath.Join(store.Dir, "session.json")
	if err := writeSessionFile(store, "session.json", recoveryFixture("safe")); err != nil {
		t.Fatal(err)
	}
	before, _ := os.ReadFile(path)
	if err := os.Mkdir(path+".bak", 0700); err != nil {
		t.Fatal(err)
	}
	if err := writeSessionFile(store, "session.json", recoveryFixture("replacement")); err == nil {
		t.Fatal("backup failure ignored")
	}
	after, _ := os.ReadFile(path)
	if !bytes.Equal(before, after) {
		t.Fatal("backup failure replaced committed data")
	}
}

func TestV24SessionMigrationsAndMissingPrimary(t *testing.T) {
	for version := range sessionSchemaVersion + 1 {
		s := recoveryFixture("legacy")
		s.SchemaVersion = version
		data, _ := json.Marshal(s)
		got, issues, err := decodeSession(data)
		if err != nil || len(issues) != 0 || got.SchemaVersion != sessionSchemaVersion || got.Chats["chat"].Draft != "legacy" {
			t.Fatalf("version %d: %+v %v %v", version, got, issues, err)
		}
	}
	dir := t.TempDir()
	data, _ := json.Marshal(recoveryFixture("pop-out"))
	if err := os.WriteFile(filepath.Join(dir, "session-popout-one.json"), data, 0600); err != nil {
		t.Fatal(err)
	}
	loaded := readSessions(dir)
	if loaded.primary == nil || len(loaded.extras) != 1 {
		t.Fatal("missing primary hid surviving pop-out")
	}
	if err := os.WriteFile(filepath.Join(dir, "session.json.bak"), data, 0600); err != nil {
		t.Fatal(err)
	}
	loaded = readSessions(dir)
	if loaded.primary.Chats["chat"].Draft != "pop-out" || !strings.Contains(loaded.problem, "missing") {
		t.Fatal("missing primary backup not recovered")
	}
	for _, raw := range []string{"null", "null\n", "[]", "{}invalid"} {
		if _, _, err := decodeSession([]byte(raw)); err == nil {
			t.Fatalf("invalid session accepted: %q", raw)
		}
	}
}

func TestV24FutureBackupPreservedWithoutPrimary(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "session.json.bak")
	original := []byte(`{"schemaVersion":900,"future":"retained"}`)
	if err := os.WriteFile(path, original, 0600); err != nil {
		t.Fatal(err)
	}
	loaded := readSessions(dir)
	if !loaded.blocked {
		t.Fatal("future backup not protected")
	}
	if err := writeSessionFile(&settings.Store{Dir: dir}, "session.json", recoveryFixture("older app")); err == nil {
		t.Fatal("older app wrote beside future backup")
	}
	after, _ := os.ReadFile(path)
	if !bytes.Equal(original, after) {
		t.Fatal("future backup changed")
	}
}

func TestV24DamagedBackupSalvageAndPreservation(t *testing.T) {
	for _, primary := range []bool{false, true} {
		t.Run(map[bool]string{false: "missing-primary", true: "damaged-primary"}[primary], func(t *testing.T) {
			dir := t.TempDir()
			path := filepath.Join(dir, "session.json")
			if primary {
				if err := os.WriteFile(path, []byte(`{"unfinished":`), 0600); err != nil {
					t.Fatal(err)
				}
			}
			original := []byte(`{"Chats":{"good":{"ID":"good","Draft":"recover me"},"bad":42}}`)
			if err := os.WriteFile(path+".bak", original, 0600); err != nil {
				t.Fatal(err)
			}
			got := readSessions(dir)
			if got.blocked || got.primary == nil || got.primary.Chats["good"].Draft != "recover me" || !strings.Contains(got.problem, "damaged records") {
				t.Fatalf("partial backup not recovered: %+v", got)
			}
			preserved, _ := filepath.Glob(path + ".bak.recovery-*")
			if len(preserved) != 1 {
				t.Fatal("damaged backup not preserved")
			}
			data, err := os.ReadFile(preserved[0])
			if err != nil || !bytes.Equal(data, original) {
				t.Fatal("original backup bytes lost")
			}
		})
	}
	// Even a healthy primary must not cause a corrupt backup to be discarded.
	store := &settings.Store{Dir: t.TempDir()}
	if err := writeSessionFile(store, "session.json", recoveryFixture("current")); err != nil {
		t.Fatal(err)
	}
	backup := filepath.Join(store.Dir, "session.json.bak")
	if err := os.WriteFile(backup, []byte(`{"unfinished":`), 0600); err != nil {
		t.Fatal(err)
	}
	if err := writeSessionFile(store, "session.json", recoveryFixture("next")); err != nil {
		t.Fatal(err)
	}
	preserved, _ := filepath.Glob(backup + ".recovery-*")
	if len(preserved) != 1 {
		t.Fatal("save discarded damaged backup")
	}
}

func TestV24LegacyRichMigrationAndBackupOnlyPopout(t *testing.T) {
	data := []byte(`{"schemaVersion":1,"Documents":[{"Tab":{"ID":"legacy","Kind":"rally","Page":"teamboard"},"Rally":{"Detail":{"Original":{"Name":"Story"},"Kind":"HierarchicalRequirement","Rich":{"Description":{"Original":"<p>old</p>","Changed":true,"Text":[65,128578],"Marks":[{"Style":1},{}]}}}}}]}`)
	s, issues, err := decodeSession(data)
	if err != nil || len(issues) != 0 || s.SchemaVersion != sessionSchemaVersion || len(s.Documents) != 1 || s.Documents[0].SchemaVersion != transferSchemaVersion {
		t.Fatalf("legacy session migration: %+v %v %v", s, issues, err)
	}
	saved := s.Documents[0].Rally.Detail.Rich["Description"]
	doc := richtext.Restore(saved)
	if string(doc.Text) != "A🙂" || doc.FormatAt(0).Style&richtext.Bold == 0 || doc.FormatAt(1).Style&richtext.Bold != 0 {
		t.Fatalf("legacy rich formatting lost: %+v", saved)
	}
	written, err := json.Marshal(s)
	if err != nil || bytes.Contains(written, []byte(`"Marks"`)) || !bytes.Contains(written, []byte(`"Text":"A🙂"`)) {
		t.Fatalf("legacy representation was retained: %s %v", written, err)
	}
	dir := t.TempDir()
	popout, _ := json.Marshal(recoveryFixture("backup-only pop-out"))
	if err := os.WriteFile(filepath.Join(dir, "session-popout-one.json.bak"), popout, 0600); err != nil {
		t.Fatal(err)
	}
	loaded := readSessions(dir)
	if loaded.primary == nil || len(loaded.extras) != 1 || loaded.extras[0].Chats["chat"].Draft != "backup-only pop-out" {
		t.Fatalf("backup-only pop-out missing: %+v", loaded)
	}
}

func TestV24ConsumedWindowBackupDoesNotReappear(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "session-popout-consumed.json")
	data, _ := json.Marshal(recoveryFixture("consumed"))
	for _, name := range []string{path, path + ".bak"} {
		if err := os.WriteFile(name, data, 0600); err != nil {
			t.Fatal(err)
		}
	}
	a := transferFixture()
	a.importedSessions = []string{path}
	a.removeImportedSessions()
	if loaded := readSessions(dir); len(loaded.extras) != 0 {
		t.Fatal("consumed window restored from backup")
	}
}
