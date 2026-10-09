package settings

import (
	"encoding/json"
	"testing"
)

func TestV67BoardPreferencesRoundTripAndLegacy(t *testing.T) {
	s := &Store{Dir: t.TempDir()}
	p := Defaults()
	want := BoardDisplay{Density: "Compact", ColorBy: "Owner", WIPLimit: 5, AgeDays: 0}
	p.RallyDisplay = &want
	p.Views = []SavedView{{Name: "Mine", Page: "teamboard", Display: &want}}
	if err := s.Save(p); err != nil {
		t.Fatal(err)
	}
	got, err := s.Load()
	if err != nil || got.RallyDisplay == nil || *got.RallyDisplay != want || *got.Views[0].Display != want {
		t.Fatalf("round trip: %+v %v", got, err)
	}
	base := Defaults()
	merged, err := Apply(base, Diff(base, p))
	if err != nil || merged.RallyDisplay == nil || *merged.RallyDisplay != want {
		t.Fatalf("patch lost display: %+v %v", merged, err)
	}
	var legacy SavedView
	if err := json.Unmarshal([]byte(`{"name":"Old","page":"teamboard"}`), &legacy); err != nil || legacy.Display != nil {
		t.Fatalf("legacy: %+v %v", legacy, err)
	}
	if DisplayOrDefault(nil) != DefaultBoardDisplay() || DefaultBoardDisplay().AgeDays != 3 {
		t.Fatal("legacy defaults")
	}
	bad := BoardDisplay{Density: "bad", ColorBy: "bad", WIPLimit: -1, AgeDays: -10}.Normalized()
	if bad.Density != "Comfortable" || bad.ColorBy != "Work Item" || bad.WIPLimit != 0 || bad.AgeDays != 0 {
		t.Fatalf("invalid settings: %+v", bad)
	}
}
