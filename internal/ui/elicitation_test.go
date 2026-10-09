package ui

import (
	"reflect"
	"testing"

	"github.com/allquixotic/fastrock/internal/codex"
)

func TestElicitationWireValuesAndValidation(t *testing.T) {
	schema := codex.Decode([]byte(`{"type":"array","minItems":1,"maxItems":2,"items":{"anyOf":[{"const":"a","title":"Alpha"},{"const":"b","title":"Beta"}]},"default":["b"]}`))
	q := elicitationQuestion("choices", schema, true)
	if got, err := elicitationValue(q); err != nil || !reflect.DeepEqual(got, []string{"b"}) {
		t.Fatalf("default: %v, %v", got, err)
	}
	q.Form.Checked[1] = false
	if _, err := elicitationValue(q); err == nil {
		t.Fatal("required multiselect accepted empty")
	}
	q = elicitationQuestion("optional", codex.Decode([]byte(`{"type":"integer","minimum":1,"maximum":5}`)), false)
	if got, err := elicitationValue(q); got != nil || err != nil {
		t.Fatalf("empty optional: %v %v", got, err)
	}
	for _, invalid := range []string{"0", "6", "2.5", "NaN", "Inf"} {
		setText(q.Editor, invalid)
		if _, err := elicitationValue(q); err == nil {
			t.Fatalf("accepted %s", invalid)
		}
	}
	setText(q.Editor, "3")
	if got, err := elicitationValue(q); got != int64(3) || err != nil {
		t.Fatalf("integer: %v %v", got, err)
	}
	q = elicitationQuestion("date", codex.Decode([]byte(`{"type":"string","format":"date"}`)), true)
	setText(q.Editor, "2025-02-29")
	if _, err := elicitationValue(q); err == nil {
		t.Fatal("invalid date accepted")
	}
	setText(q.Editor, "2024-02-29")
	if _, err := elicitationValue(q); err != nil {
		t.Fatal(err)
	}
	q = elicitationQuestion("text", codex.Decode([]byte(`{"type":"string","minLength":2,"maxLength":2}`)), true)
	setText(q.Editor, "日本")
	if _, err := elicitationValue(q); err != nil {
		t.Fatal("length must count Unicode characters", err)
	}
	q = elicitationQuestion("single", codex.Decode([]byte(`{"type":"string","enum":["wire"],"enumNames":["Visible"],"default":"wire"}`)), true)
	if got, err := elicitationValue(q); got != "wire" || err != nil || q.Options[0] != "Visible" {
		t.Fatalf("enum labels: %v %v", got, err)
	}
}

func TestUserInputKeepsChoiceAndNote(t *testing.T) {
	q := question{Options: []string{"A", "B"}, Selected: 1, Editor: textEditor("also this", false)}
	if got := questionAnswers(q); !reflect.DeepEqual(got, []string{"B", "user_note: also this"}) {
		t.Fatal(got)
	}
	q.Selected = -1
	setText(q.Editor, "")
	if got := questionAnswers(q); len(got) != 0 {
		t.Fatal("unanswered question selected implicitly", got)
	}
}
