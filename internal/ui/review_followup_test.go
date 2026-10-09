package ui

import (
	"strings"
	"testing"

	"github.com/allquixotic/fastrock/internal/platform"
	"github.com/allquixotic/fastrock/internal/rally"
	"golang.org/x/mobile/event/key"
)

func TestTablePageDoesNotSkipPartialServerPage(t *testing.T) {
	v := newRallyView(rally.FindPage("userstories"))
	v.Total = 400
	v.Start = 1
	v.Page = 6
	if _, _, ready := v.tableRange(128); ready {
		t.Fatal("partial page incorrectly declared ready")
	}
	v.Start = 126
	if start, end, ready := v.tableRange(128); !ready || start != 0 || end != 25 {
		t.Fatalf("boundary page = %d:%d, %v", start, end, ready)
	}
	v.Page = 7
	if start, end, ready := v.tableRange(128); !ready || start != 25 || end != 50 {
		t.Fatalf("next page = %d:%d, %v", start, end, ready)
	}
	v.Page = 5
	if _, _, ready := v.tableRange(128); ready {
		t.Fatal("previous page was outside the resident window")
	}
	v.Start = 376
	v.Page = 16
	if start, end, ready := v.tableRange(25); !ready || start != 0 || end != 25 {
		t.Fatal("last page is inaccessible")
	}
}
func TestConfigRefreshKeepsOnlyUnsavedEditors(t *testing.T) {
	old := configFields(map[string]any{"model": "old", "note": "original"})
	setText(old[1].Editor, "draft")
	fresh := mergeConfigFields(old, configFields(map[string]any{"model": "new", "note": "remote"}))
	if text(fresh[0].Editor) != "new" || text(fresh[1].Editor) != "draft" || fresh[1].Baseline != "remote" {
		t.Fatal("refresh lost an edit or a new baseline")
	}
}
func TestUnsupportedHTMLStartsWithOriginalSource(t *testing.T) {
	const source = `<table><tr><td>Keep &amp; preserve</td></tr></table>`
	r := newRichEditor(source)
	if r.html() != source {
		t.Fatalf("untouched HTML was rewritten: %s", r.html())
	}
}
func TestToolOutputRetainsLiteralPunctuation(t *testing.T) {
	source := "# heading\n*not emphasis* <file> _name_\n    spaces\n"
	l := prepareLiteralTranscript(source, 640, 13)
	if l.Plain != source {
		t.Fatalf("command output changed: %q", l.Plain)
	}
	for _, line := range l.Lines {
		if !line.Code {
			t.Fatal("tool line used Markdown styling")
		}
	}
}
func TestRallyProjectionAndUserQueryUseTheEntityFields(t *testing.T) {
	app := &App{}
	v := newRallyView(rally.FindPage("users"))
	setText(v.Search, "a user")
	query := app.rallyQuery(v)
	if query.Order != "DisplayName ASC" || strings.Contains(query.Expression, "Description") || !strings.Contains(query.Expression, "UserName") {
		t.Fatalf("invalid user query: %+v", query)
	}
	fetch := rallyFetch(v)
	if strings.Contains(fetch, "PlanEstimate") || !strings.Contains(fetch, "UserName") {
		t.Fatalf("invalid user projection %s", fetch)
	}
	v = newRallyView(rally.FindPage("timeline"))
	if !strings.Contains(rallyFetch(v), "PlannedEndDate") {
		t.Fatal("timeline dates were not requested")
	}
}
func TestServerConfirmedSearchSurvivesMissingDescription(t *testing.T) {
	v := newRallyView(rally.FindPage("userstories"))
	setText(v.Search, "in description only")
	v.ServerFiltered = true
	v.AppliedFilters = v.quickFilterSignature()
	v.Items = []rally.Object{{"Name": "title", "_ref": "/item/1"}}
	if len(v.filtered()) != 1 {
		t.Fatal("local projection removed a server-confirmed search match")
	}
}
func TestShortcutLabelsAndEditingReservations(t *testing.T) {
	if got := shortcutLabel(key.CodeT, key.ModControl|key.ModShift); got != "Ctrl+Shift+T" {
		t.Fatal(got)
	}
	if validateShortcut(key.CodeA, 0) == "" || validateShortcut(key.CodeC, platform.PrimaryModifier()) == "" {
		t.Fatal("text editing can be intercepted")
	}
	if validateShortcut(key.CodeF6, 0) != "" {
		t.Fatal("function key should be available")
	}
}

func TestUnknownActivityKeepsInspectableData(t *testing.T) {
	b := formatItem(map[string]any{"id": "hook-1", "type": "hookStarted", "command": "echo hello"})
	if b.Role != "activity" || !strings.Contains(b.Text, "echo hello") {
		t.Fatal("unknown item became empty")
	}
}
