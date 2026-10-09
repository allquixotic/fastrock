package richtext

import (
	"encoding/json"
	"reflect"
	"strings"
	"testing"
)

func TestCompactDraftRoundTripAndLegacyMigration(t *testing.T) {
	d := Parse("<p><b>Hello 世界</b></p>")
	d.ApplyEdit(6, 0, []rune("new "))
	d.Toggle(0, 5, Italic)
	for _, legacy := range []bool{false, true} {
		var value any = d.Save()
		if legacy {
			value = struct {
				Original string
				Text     []rune
				Marks    []Format
				Changed  bool
			}{d.original, d.Text, denseMarks(d), true}
		}
		encoded, err := json.Marshal(value)
		if err != nil {
			t.Fatal(err)
		}
		var saved Saved
		if err := json.Unmarshal(encoded, &saved); err != nil {
			t.Fatal(err)
		}
		restored := Restore(saved)
		if restored.HTML() != d.HTML() || !reflect.DeepEqual(restored.Text, d.Text) || !reflect.DeepEqual(denseMarks(restored), denseMarks(d)) {
			t.Fatalf("legacy=%v: draft changed: %s", legacy, restored.HTML())
		}
		// A restored snapshot remains independent of later editor mutations.
		restored.Toggle(0, 5, Strike)
		if Restore(saved).HTML() != d.HTML() {
			t.Fatal("restored editor mutated its saved snapshot")
		}
	}
}

func TestCompactDraftSizeAndInvalidSpans(t *testing.T) {
	d := Parse("<p>original</p>")
	d.Sync([]rune(strings.Repeat("hello ", 20000)))
	saved := d.Save()
	encoded, err := json.Marshal(saved)
	if err != nil {
		t.Fatal(err)
	}
	if len(saved.Spans) != 1 || len(encoded) > len(saved.Text)+256 {
		t.Fatalf("plain draft is not compact: text=%d JSON=%d spans=%d", len(saved.Text), len(encoded), len(saved.Spans))
	}
	for _, input := range []string{
		`{"Changed":true,"Text":"abc","Spans":[]}`,
		`{"Changed":true,"Text":"abc","Spans":[{"End":4}]}`,
		`{"Changed":true,"Text":"abc","Spans":[{"End":2},{"End":1},{"End":3}]}`,
		`{"Changed":true,"Text":[97,98],"Marks":[{}]}`,
	} {
		var got Saved
		if json.Unmarshal([]byte(input), &got) == nil {
			t.Fatalf("accepted malformed draft: %s", input)
		}
	}
	d.Sync(nil)
	encoded, _ = json.Marshal(d.Save())
	var empty Saved
	if err := json.Unmarshal(encoded, &empty); err != nil || string(Restore(empty).Text) != "" {
		t.Fatalf("empty edited document lost: %s (%v)", encoded, err)
	}
}
