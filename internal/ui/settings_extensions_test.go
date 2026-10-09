//go:build nucular_headless

package ui

import (
	"context"
	"encoding/json"
	"fmt"
	"image"
	"reflect"
	"strings"
	"testing"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/command"
)

func fixtureData(t *testing.T, source string) map[string]any {
	t.Helper()
	var data map[string]any
	if err := json.Unmarshal([]byte(source), &data); err != nil {
		t.Fatal(err)
	}
	return data
}
func TestV32FeatureGroupsAndManagedWrites(t *testing.T) {
	data := fixtureData(t, `{"data":[{"name":"old","stage":"removed","enabled":true},{"name":"z","displayName":"Feature Z","stage":"stable","enabled":true,"defaultEnabled":false},{"name":"b","stage":"beta","enabled":false,"defaultEnabled":false},{"name":"a","stage":"beta","enabled":true,"defaultEnabled":true},{"name":"dev","stage":"underDevelopment"},{"name":"gone","stage":"deprecated"}]}`)
	items := settingsItems("Features", data)
	if len(items) != 5 || items[0].ID != "a" || items[1].ID != "b" || items[2].Group != "Stable" || items[2].Status != "Changed" || items[0].Status != "" || !strings.Contains(items[3].Group, "unstable") {
		t.Fatalf("groups/defaults: %+v", items)
	}
	a := settingsFixture(t)
	a.settingsView.Page = "Features"
	a.setExtensionEnabled("Features", items[0], false)
	if len(a.settingsView.ActionFeedback) != 0 {
		t.Fatal("write allowed without policy")
	}
	a.refreshSignInPolicy()
	drain(t, a, func() bool { return !a.policyLoading })
	a.setExtensionEnabled("Features", settingsItem{ID: "memories"}, true)
	if len(a.settingsView.ActionFeedback) != 0 {
		t.Fatal("managed feature changed")
	}
	a.setExtensionEnabled("Features", items[0], false)
	key := settingsRowKey("Features", "a")
	drain(t, a, func() bool { return !a.settingsView.ActionFeedback[key].Pending })
	if a.settingsView.ActionFeedback[key].Message != "Saved" {
		t.Fatal(a.settingsView.ActionFeedback[key])
	}
}
func TestV32ExtensionMetadataAndProblems(t *testing.T) {
	skills := settingsItems("Skills", fixtureData(t, `{"data":[{"skills":[{"name":"one","path":"/tmp/one/SKILL.md","scope":"user","enabled":false,"interface":{"displayName":"One"}},{"name":"project","path":"/tmp/project/SKILL.md","scope":"repo","enabled":true,"pluginId":"p"}],"errors":[{"path":"/tmp/broken/SKILL.md","message":"bad front matter"}]}]}`))
	if len(skills) != 3 || skills[0].Group != "Project skills" || skills[0].Detail != "Plugin p" || skills[1].Name != "One" || skills[1].Status != "Disabled" || !skills[2].ReadOnly || skills[2].Error != "bad front matter" {
		t.Fatalf("skills: %+v", skills)
	}
	hooks := settingsItems("Hooks", fixtureData(t, `{"data":[{"hooks":[{"key":"trusted","eventName":"stop","handlerType":"command","command":"echo done","async":true,"trustStatus":"trusted","currentHash":"t","displayOrder":3},{"key":"untrusted","eventName":"start","handlerType":"mcpTool","server":"s","tool":"t","trustStatus":"untrusted","currentHash":"u","displayOrder":1},{"key":"managed","eventName":"stop","handlerType":"agent","trustStatus":"modified","isManaged":true,"currentHash":"m","displayOrder":2}],"warnings":["check source"],"errors":[{"path":"hooks.json","message":"invalid"}]}]}`))
	if len(hooks) != 5 || hooks[0].ID != "untrusted" || !hooks[0].NeedsTrust || hooks[0].Subtitle != "MCP tool s/t" || hooks[1].NeedsTrust || hooks[2].NeedsTrust || hooks[2].Subtitle != "echo done (async)" || hooks[3].Error != "invalid" || hooks[4].Status != "Warning" {
		t.Fatalf("hooks: %+v", hooks)
	}
	plugins := settingsItems("Plugins", fixtureData(t, `{"marketplaces":[{"name":"m","path":"/tmp/market.json","interface":{"displayName":"Market"},"plugins":[{"id":"p","name":"p","installed":true,"enabled":true,"localVersion":"2","interface":{"displayName":"Plugin","developerName":"Dev"}},{"id":"b","name":"b","availability":"DISABLED_BY_ADMIN","installed":false},{"id":"a","name":"a","availability":"AVAILABLE","installPolicy":"AVAILABLE","installed":false}]}],"marketplaceLoadErrors":[{"marketplacePath":"/tmp/broken","message":"offline"}]}`))
	if len(plugins) != 4 || plugins[0].Group != "Market" || !plugins[0].CanRemove || plugins[0].Subtitle != "p · v2" || plugins[0].Detail != "Dev" || plugins[1].CanInstall || plugins[1].Status != "Disabled by admin" || !plugins[2].CanInstall || plugins[3].Error != "offline" {
		t.Fatalf("plugins: %+v", plugins)
	}
	if got := pluginParams(plugins[0])["marketplacePath"]; got != "/tmp/market.json" {
		t.Fatal("plugin source missing", got)
	}
}
func TestV32MCPConfiguredRowsAndStructuredForm(t *testing.T) {
	data := fixtureData(t, `{"data":[{"name":"local","authStatus":"unsupported","tools":{"a":{}}},{"name":"remote","runtimeStatus":"authenticationRequired","authStatus":"notLoggedIn","httpOrigin":"https://example.test","pluginId":"plugin"}],"_configuration":{"config":{"mcp_servers":{"local":{"command":"C:\\Program Files\\tools\\server.exe","args":["--root","C:\\data dir"],"enabled":false},"inherited":{"url":"https://inherited.test"}}},"layers":[{"name":{"type":"user"},"config":{"mcp_servers":{"local":{}}}}],"origins":{"mcp_servers.inherited":{"name":{"type":"project"}}}}}`)
	rows := settingsItems("MCP servers", data)
	if len(rows) != 3 || rows[0].ID != "inherited" || !rows[0].ReadOnly || rows[0].CanRemove || rows[0].Detail != "project" {
		t.Fatalf("inherited: %+v", rows)
	}
	if rows[1].Status != "Disabled" || rows[1].ReadOnly || !rows[1].CanRemove || !strings.Contains(rows[1].Subtitle, "Program Files") || rows[1].Detail != "User config · 1 tools" {
		t.Fatalf("local: %+v", rows[1])
	}
	if !rows[2].CanLogin || rows[2].Status != "Sign-in required" || !strings.Contains(rows[2].Detail, "Plugin plugin") {
		t.Fatal(rows[2])
	}
	f := newMCPForm()
	setText(f.Name, "literal.dot")
	setText(f.Command, `C:\tools\server.exe`)
	f.Args = []*nucular.TextEditor{textEditor(`C:\data dir\`, false), textEditor("", false), textEditor("a\"b", false)}
	f.Env = []settingPair{newSettingPair("PATH", `C:\tools`), newSettingPair("MODE", "a=b c")}
	config, err := mcpConfig(f)
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(config["args"], []string{`C:\data dir\`, "", "a\"b"}) || config["env"].(map[string]string)["MODE"] != "a=b c" {
		t.Fatal(config)
	}
	f.Transport = 1
	setText(f.URL, "https://server.test/mcp")
	setText(f.Bearer, "MCP_TOKEN")
	f.Headers = []settingPair{newSettingPair("X-Mode", "plain")}
	config, err = mcpConfig(f)
	if err != nil || config["bearer_token_env_var"] != "MCP_TOKEN" || config["http_headers"].(map[string]string)["X-Mode"] != "plain" {
		t.Fatal(config, err)
	}
	setText(f.Bearer, "a secret")
	if _, err = mcpConfig(f); err == nil {
		t.Fatal("literal token accepted as environment name")
	}
	setText(f.Bearer, "")
	f.Headers[0] = newSettingPair("X-Mode", "a\r\nb")
	if _, err = mcpConfig(f); err == nil {
		t.Fatal("header injection accepted")
	}
	if _, err = settingPairs([]settingPair{newSettingPair("A", "a"), newSettingPair("A", "b")}, "environment"); err == nil {
		t.Fatal("duplicate environment key accepted")
	}
	if quoteWindowsWord(`C:\Program Files\tool\`) != `"C:\Program Files\tool\\"` {
		t.Fatal(quoteWindowsWord(`C:\Program Files\tool\`))
	}
}

type settingsCallerFunc func(context.Context, string, any, any) error

func (f settingsCallerFunc) Call(ctx context.Context, method string, params, out any) error {
	return f(ctx, method, params, out)
}
func assignFixture(out any, value any) error {
	b, err := json.Marshal(value)
	if err != nil {
		return err
	}
	return json.Unmarshal(b, out)
}
func TestV32BoundedPaginationAndLiveStatus(t *testing.T) {
	calls := 0
	c := settingsCallerFunc(func(_ context.Context, method string, params, out any) error {
		if method == "config/read" {
			return assignFixture(out, map[string]any{"config": map[string]any{"mcp_servers": map[string]any{"configured": map[string]any{"command": "server"}}}})
		}
		calls++
		p := object(params)
		if p["limit"] != 100 || p["detail"] != "toolsAndAuthOnly" {
			t.Fatal(p)
		}
		if calls == 1 {
			return assignFixture(out, map[string]any{"data": []any{map[string]any{"name": "one"}}, "nextCursor": "next"})
		}
		if p["cursor"] != "next" {
			t.Fatal(p)
		}
		return assignFixture(out, map[string]any{"data": []any{map[string]any{"name": "two"}}})
	})
	data, err := settingsPayload(context.Background(), c, "MCP servers", "mcpServerStatus/list", map[string]any{}, "/tmp")
	if err != nil || calls != 2 || len(settingsItems("MCP servers", data)) != 3 {
		t.Fatal(data, err, calls)
	}
	repeated := settingsCallerFunc(func(_ context.Context, _ string, _ any, out any) error {
		return assignFixture(out, map[string]any{"nextCursor": "same"})
	})
	if _, err = settingsPayload(context.Background(), repeated, "Features", "experimentalFeature/list", nil, ""); err == nil || !strings.Contains(err.Error(), "repeated") {
		t.Fatal(err)
	}
	count := 0
	forever := settingsCallerFunc(func(_ context.Context, _ string, _ any, out any) error {
		count++
		return assignFixture(out, map[string]any{"nextCursor": fmt.Sprint(count)})
	})
	if _, err = settingsPayload(context.Background(), forever, "Features", "experimentalFeature/list", nil, ""); err == nil || count != 100 {
		t.Fatal(err, count)
	}
	a := settingsFixture(t)
	a.settingsEvent("mcpServer/startupStatus/updated", map[string]any{"name": "one", "status": "failed", "error": "cannot connect"})
	if got := a.settingsView.MCPLive["one"]; got.Status != "Failed" || got.Error != "cannot connect" {
		t.Fatal(got)
	}
	a.settingsEvent("mcpServer/startupStatus/updated", map[string]any{"name": "one", "status": "ready"})
	if got := a.settingsView.MCPLive["one"]; got.Status != "Connected" || got.Error != "" {
		t.Fatal(got)
	}
	a.resetSettingsConnection()
	if len(a.settingsView.MCPLive) != 0 {
		t.Fatal("old server status retained")
	}
}
func TestV32MCPWritesReloadInOrder(t *testing.T) {
	a := settingsFixture(t)
	s := a.settingsView
	s.Page = "MCP servers"
	key := settingsRowKey(s.Page, "one")
	a.settingsRequestAt(key, "config/value/write", map[string]any{"keyPath": "mcp_servers.one.enabled", "value": false})
	drain(t, a, func() bool { return !s.ActionFeedback[key].Pending })
	if s.ActionFeedback[key].Failed {
		t.Fatal(s.ActionFeedback[key])
	}
	var calls []string
	if err := a.client.Call(a.ctx, "fixture/mcpCalls", nil, &calls); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(calls, []string{"config/value/write", "config/mcpServer/reload"}) {
		t.Fatal(calls)
	}
	drain(t, a, func() bool { return !s.Busy })
	f := newMCPForm()
	f.Open = true
	setText(f.Name, "test")
	setText(f.Command, "server")
	a.addMCPServer(f)
	drain(t, a, func() bool { return !f.Busy })
	if f.Open || f.Error != "" {
		t.Fatal(f.Error)
	}
	if err := a.client.Call(a.ctx, "fixture/mcpCalls", nil, &calls); err != nil {
		t.Fatal(err)
	}
	if len(calls) != 4 || calls[2] != "config/value/write" || calls[3] != "config/mcpServer/reload" {
		t.Fatal(calls)
	}
}
func TestV32MCPFormValidationIsInline(t *testing.T) {
	a := settingsFixture(t)
	f := newMCPForm()
	f.Open = true
	a.addMCPServer(f)
	if f.Error == "" || f.Busy || a.toast != "" {
		t.Fatal("validation did not stay inline", f.Error, a.toast)
	}
	a.settingsView.Page = "Skills"
	key := settingsRowKey("Skills", "skill")
	a.settingsRequestAt(key, "skills/config/write", map[string]any{"path": "/tmp/skill", "enabled": false})
	drain(t, a, func() bool { return !a.settingsView.ActionFeedback[key].Pending })
	if !strings.Contains(a.settingsView.ActionFeedback[key].Message, "higher-precedence") {
		t.Fatal(a.settingsView.ActionFeedback[key])
	}
}
func TestV32ExtensionStatusRendering(t *testing.T) {
	a := settingsFixture(t)
	s := a.settingsView
	s.Page = "Features"
	a.catalog.PolicyLoaded = true
	s.Items = settingsItems("Features", fixtureData(t, `{"data":[{"name":"test","displayName":"Feature test","stage":"beta","enabled":true,"defaultEnabled":false}]}`))
	var texts []string
	h := nucular.NewHeadlessHarness(0, image.Pt(850, 700), func(w *nucular.Window) {
		a.drawExtensionSettings(w, s)
		for _, c := range w.Commands().Commands {
			if c.Kind == command.TextCmd {
				texts = append(texts, c.Text.String)
			}
		}
	})
	h.Master().SetStyle(makeStyle(a.p, 13))
	h.Frame(false)
	joined := strings.Join(texts, " ")
	for _, want := range []string{"Beta", "Feature test", "Changed", "default off"} {
		if !strings.Contains(joined, want) {
			t.Fatal(want, joined)
		}
	}
}
