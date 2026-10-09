package settings

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func TestIndependentWindowPreferencesMerge(t *testing.T) {
	initial := Defaults()
	left, right := initial, initial
	left.Theme = "light"
	right.FontSize = 19
	merged, err := Apply(initial, Diff(initial, left))
	if err != nil {
		t.Fatal(err)
	}
	merged, err = Apply(merged, Diff(initial, right))
	if err != nil {
		t.Fatal(err)
	}
	if merged.Theme != "light" || merged.FontSize != 19 {
		t.Fatalf("lost independent preference: %+v", merged)
	}
	right.Views = []SavedView{{Name: "x"}}
	merged, err = Apply(merged, Diff(initial, right))
	if err != nil || len(merged.Views) != 1 {
		t.Fatal(merged, err)
	}
	cleared := right
	cleared.Views = nil
	merged, err = Apply(merged, Diff(right, cleared))
	if err != nil || len(merged.Views) != 0 {
		t.Fatal(merged, err)
	}
}
func TestFuturePreferencesAreNotOverwritten(t *testing.T) {
	s := Store{Dir: t.TempDir()}
	data := []byte(`{"schemaVersion":99,"theme":"future"}`)
	path := filepath.Join(s.Dir, "settings.json")
	if err := os.WriteFile(path, data, 0600); err != nil {
		t.Fatal(err)
	}
	p, err := s.Load()
	if err == nil {
		t.Fatal("accepted unsupported schema")
	}
	if err = s.Save(p); err == nil {
		t.Fatal("overwrote unsupported schema")
	}
	got, _ := os.ReadFile(path)
	if string(got) != string(data) {
		t.Fatal("changed original")
	}
	if _, err = Apply(Defaults(), Patch{"unexpected": json.RawMessage("true")}); err == nil {
		t.Fatal("accepted unknown preference")
	}
}
