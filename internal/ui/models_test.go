package ui

import (
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/settings"
	"github.com/allquixotic/fastrock/internal/workspace"
	"testing"
)

func TestReferenceThemeTokensAndDarkDefault(t *testing.T) {
	p := colors(false)
	if p.Window != hex(0x1b1c1f) || p.Accent != hex(0x5b8def) || p.Text != hex(0xe6e7ea) {
		t.Fatal("Codex GUI theme changed")
	}
	if colors(true).Window == p.Window {
		t.Fatal("light theme unavailable")
	}
}
func TestDetailChangesRetainCustomFieldsAndValidateNumbers(t *testing.T) {
	o := rally.Object{"Name": "Story", "PlanEstimate": 5.0, "c_TeamNote": "Original"}
	d := makeDetail(o, "HierarchicalRequirement", false)
	d.Fields = []rally.Field{{Name: "c_TeamNote", AttributeType: "STRING"}}
	setText(d.Editors["c_TeamNote"], "New note")
	fields, e := d.changes()
	if e != nil || fields.String("c_TeamNote") != "New note" {
		t.Fatal(fields, e)
	}
	if _, ok := fields["PlanEstimate"]; ok {
		t.Fatal("unmodified estimate sent")
	}
	setText(d.Editors["PlanEstimate"], "not a number")
	if _, e = d.changes(); e == nil {
		t.Fatal("invalid estimate accepted")
	}
}
func TestFiltersAndViewsDoNotInventRecords(t *testing.T) {
	v := newRallyView(rally.FindPage("teamboard"))
	if len(v.Items) != 0 {
		t.Fatal("fictional data in production view")
	}
	v.Items = []rally.Object{{"Name": "First", "FormattedID": "US1"}, {"Name": "Second", "FormattedID": "US2"}}
	setText(v.Search, "US2")
	if got := v.filtered(); len(got) != 1 || got[0].ID() != "US2" {
		t.Fatal(got)
	}
}
func TestSettingsScrubCredentials(t *testing.T) {
	v := map[string]any{"model": "gpt-6.1-sol", "provider": map[string]any{"api_key": "hidden", "accessToken": "hidden"}}
	scrub(v)
	nested := v["provider"].(map[string]any)
	if nested["api_key"] != "[redacted]" || nested["accessToken"] != "[redacted]" {
		t.Fatal(v)
	}
}

func TestReadOnlySchemaFieldIsNeverSent(t *testing.T) {
	d := makeDetail(rally.Object{"Name": "Story", "c_External": "original"}, "HierarchicalRequirement", false)
	d.Fields = []rally.Field{{Name: "c_External", ReadOnly: true}}
	setText(d.Editors["c_External"], "changed")
	fields, err := d.changes()
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := fields["c_External"]; ok {
		t.Fatal("read-only field submitted")
	}
}

func TestSessionKeepsDraftsAndQueuesWithoutTranscript(t *testing.T) {
	a := &App{store: &settings.Store{Dir: t.TempDir()}, state: workspace.NewState(), chats: map[string]*chatView{}}
	c := &workspace.Conversation{ID: "test", Title: "A chat", Cwd: "/project", Status: "running", TurnID: "active", Blocks: []workspace.Block{{Text: "transcript owned by Codex"}}}
	c.Enqueue("next prompt", []string{"image.png"})
	a.state.Chats[c.ID] = c
	a.chats[c.ID] = newChatView()
	setText(a.chats[c.ID].Editor, "unfinished prompt")
	a.state.Open(workspace.Chat, c.Title, c.ID, "")
	a.saveSession()
	b := &App{store: a.store, state: workspace.NewState()}
	b.loadSession()
	got := b.state.Chats[c.ID]
	if got == nil || got.Draft != "unfinished prompt" || len(got.Queue) != 1 || len(got.Blocks) != 0 || got.TurnID != "" || got.Status != "idle" {
		t.Fatalf("invalid restored state: %#v", got)
	}
}
