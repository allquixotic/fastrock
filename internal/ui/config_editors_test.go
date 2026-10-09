//go:build nucular_headless

package ui

import (
	"encoding/json"
	"image"
	"reflect"
	"strings"
	"testing"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/command"
	"github.com/allquixotic/fastrock/internal/workspace"
)

func TestV33ConfigTableAndCommandRoundTrips(t *testing.T) {
	config := fixtureData(t, `{"mcp_servers":{"dot.name":{"command":"server","args":["space here",""],"enabled":true}},"features":{"memories":false},"notify":["notify","a b","it's done"],"future":{"nested":{"count":1}}}`)
	fields := schemaConfigFields(config)
	var table, notify *configField
	for i := range fields {
		f := &fields[i]
		if f.Key == "mcp_servers" {
			table = f
		}
		if strings.HasPrefix(f.Key, "mcp_servers.") {
			t.Fatal("table still flattened", f.Key)
		}
		if f.Key == "notify" {
			notify = f
		}
	}
	if table == nil || table.Kind != "toml" || !strings.Contains(text(table.Editor), "[mcp_servers.") {
		t.Fatal("missing TOML editor", table)
	}
	value, err := configFieldValue(table)
	if err != nil {
		t.Fatal(err)
	}
	got, _ := json.Marshal(value)
	want, _ := json.Marshal(config["mcp_servers"])
	if string(got) != string(want) {
		t.Fatal(string(got), string(want))
	}
	if notify == nil || notify.Kind != "words" {
		t.Fatal("missing command editor")
	}
	value, err = configFieldValue(notify)
	if err != nil || !reflect.DeepEqual(value, []string{"notify", "a b", "it's done"}) {
		t.Fatal(value, err)
	}
	setText(notify.Editor, "")
	value, err = configFieldValue(notify)
	if value != nil || err != nil {
		t.Fatal("blank should remove notify", value, err)
	}
	cases := []struct {
		source  string
		windows bool
		want    []string
	}{
		{`"C:\Program Files\App\app.exe" --path C:\work\repo\`, true, []string{`C:\Program Files\App\app.exe`, "--path", `C:\work\repo\`}},
		{`say "he said \"hi\"" ""`, true, []string{"say", `he said "hi"`, ""}},
		{`say 'a b' "it's done" empty\ value`, false, []string{"say", "a b", "it's done", "empty value"}},
		{`say "C:\work\repo"`, false, []string{"say", `C:\work\repo`}},
	}
	for _, tc := range cases {
		got, err := splitWordsForPlatform(tc.source, tc.windows)
		if err != nil || !reflect.DeepEqual(got, tc.want) {
			t.Fatalf("%q: %q %v", tc.source, got, err)
		}
	}
	for _, windows := range []bool{false, true} {
		if _, err := splitWordsForPlatform(`say "open`, windows); err == nil {
			t.Fatal("unbalanced quote accepted")
		}
	}
}
func TestV33ScopedSnippetRejectsOtherKeysAndProtectedData(t *testing.T) {
	f := configField{Key: `outer."inner.dot"`, Kind: "toml", Editor: textEditor(`["inner.dot"]
value = "kept"`, true)}
	value, err := configFieldValue(&f)
	if err != nil || object(value)["value"] != "kept" {
		t.Fatal(value, err)
	}
	setText(f.Editor, "other = 2")
	if _, err := configFieldValue(&f); err == nil {
		t.Fatal("unrelated key accepted")
	}
	setText(f.Editor, `"inner.dot" = 2`)
	if _, err := configFieldValue(&f); err == nil {
		t.Fatal("scalar accepted as table")
	}
	setText(f.Editor, "")
	if value, err := configFieldValue(&f); value != nil || err != nil {
		t.Fatal(value, err)
	}
	origin, locked := configFieldOrigin(fixtureData(t, `{"origins":{"outer.child":{"name":{"type":"project"}}}}`), &configField{Key: "outer", Kind: "toml"})
	if !locked || origin != "project" {
		t.Fatal("table could overwrite locked child", origin, locked)
	}
	fields := schemaConfigFields(fixtureData(t, `{"model_providers":{"custom":{"http_headers":{"Authorization":"[redacted]"}}}}`))
	for _, f := range fields {
		if f.Key == "model_providers" {
			if !f.Protected {
				t.Fatal("protected data not tracked")
			}
			if _, err := configFieldValue(&f); err == nil {
				t.Fatal("redacted placeholders writable")
			}
			return
		}
	}
	t.Fatal("provider table missing")
}
func TestV33SearchContextAndInlineHelp(t *testing.T) {
	a := settingsFixture(t)
	s := a.settingsView
	s.Page = "Codex configuration"
	a.catalog.PolicyLoaded = true
	a.prefs.WorkingDirectory = "/current"
	a.prefs.RecentFolders = []string{"/recent", "/current"}
	a.state.Chats["open"] = &workspace.Conversation{ID: "open", Cwd: "/open"}
	a.state.Open(workspace.Chat, "open", "open", "")
	s.ConfigContext = textEditor("/chosen", false)
	a.prepareConfigFolders()
	if !reflect.DeepEqual(s.ConfigFolders, []string{"/chosen", "/current", "/recent", "/open"}) {
		t.Fatal(s.ConfigFolders)
	}
	setText(s.Search, "notify")
	setText(s.PluginSearch, "plugin needle")
	if text(s.Search) != "notify" {
		t.Fatal("plugin search shared with settings")
	}
	s.Fields = []configField{{Key: "notify", Kind: "words", Editor: textEditor("notify done", false), Search: "notify", Spec: &configSpec{Description: "Run a command when a turn finishes."}}}
	render := func() string {
		var texts []string
		h := nucular.NewHeadlessHarness(0, image.Pt(950, 950), func(w *nucular.Window) {
			a.drawConfiguration(w, s)
			for _, c := range w.Commands().Commands {
				if c.Kind == command.TextCmd {
					texts = append(texts, c.Text.String)
				}
			}
		})
		h.Master().SetStyle(makeStyle(a.p, 13))
		h.Frame(false)
		return strings.Join(texts, " ")
	}
	shown := render()
	for _, want := range []string{"General", "Source: Default", "Run a command when a turn finishes."} {
		if !strings.Contains(shown, want) {
			t.Fatal(want, shown)
		}
	}
	setText(s.Search, "no matching configuration")
	if shown = render(); !strings.Contains(shown, "No settings match") {
		t.Fatal(shown)
	}
}
