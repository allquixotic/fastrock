//go:build fltk_headless

package ui

import (
	"bufio"
	"context"
	"encoding/json"
	"image"
	"os"
	"strings"
	"testing"
	"time"

	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/command"
	"github.com/allquixotic/fastrock/internal/settings"
)

func TestSettingsRPCFixture(t *testing.T) {
	if len(os.Args) == 0 || os.Args[len(os.Args)-1] != "fastrock-settings-fixture" {
		return
	}
	scanner := bufio.NewScanner(os.Stdin)
	writer := json.NewEncoder(os.Stdout)
	var mcpCalls []string
	var providerCalls []codex.Message
	var allowBedrock bool
	for scanner.Scan() {
		var request codex.Message
		if json.Unmarshal(scanner.Bytes(), &request) != nil || request.Method == "" {
			continue
		}
		if allowBedrock && (request.Method == "account/bedrock/setup" || request.Method == "account/login/start" || request.Method == "config/batchWrite" || request.Method == "account/bedrock/checkGovCloudRequirements") {
			providerCalls = append(providerCalls, request)
		}
		var result any = map[string]any{}
		var failure *codex.RPCError
		switch request.Method {
		case "fixture/allowBedrock":
			allowBedrock = true
		case "fixture/providerCalls":
			result = providerCalls
		case "model/list":
			result = map[string]any{"data": []any{map[string]any{"id": "fixture", "model": "fixture", "displayName": "Fixture"}}}
		case "fixture/mcpCalls":
			result = mcpCalls
		case "config/value/write", "config/mcpServer/reload":
			mcpCalls = append(mcpCalls, request.Method)
		case "skills/config/write":
			result = map[string]any{"effectiveEnabled": true}
		case "config/read":
			result = map[string]any{"config": map[string]any{"model_provider": "openai", "approval_policy": "on-request"}}
		case "configRequirements/read":
			result = map[string]any{"requirements": map[string]any{"allowedLoginMethods": []string{"chatgpt"}, "allowedSandboxModes": []string{"read-only"}, "featureRequirements": map[string]bool{"memories": false}}}
		case "account/read":
			result = map[string]any{"requiresOpenaiAuth": true, "account": map[string]any{"type": "chatgpt", "email": "fixture@example.test", "planType": "team"}}
		case "account/rateLimits/read":
			result = map[string]any{"rateLimits": map[string]any{"primary": map[string]any{"usedPercent": 99}}, "rateLimitsByLimitId": map[string]any{"codex": map[string]any{"limitName": "Fresh name", "primary": map[string]any{"usedPercent": 20, "windowDurationMins": 300, "resetsAt": 2000000000}, "secondary": map[string]any{"usedPercent": 25, "windowDurationMins": 10080}}}}
		case "account/bedrock/setup":
			if !allowBedrock {
				failure = &codex.RPCError{Code: -32000, Message: "fixture credentials rejected"}
			}
		case "fixture/fail":
			failure = &codex.RPCError{Code: -32000, Message: "fixture write rejected"}
		case "config/batchWrite":
			result = map[string]any{"version": "next"}
		case "skills/list":
			result = map[string]any{"data": []any{}}
		}
		encoded, _ := json.Marshal(result)
		_ = writer.Encode(codex.Message{ID: request.ID, Result: encoded, Error: failure})
	}
	os.Exit(0)
}

func settingsFixture(t *testing.T) *App {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	t.Cleanup(cancel)
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	client, err := codex.StartCommand(ctx, executable, []string{"-test.run=^TestSettingsRPCFixture$", "--", "fastrock-settings-fixture"})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(client.Close)
	a := transferFixture()
	a.ctx, a.client, a.updates = ctx, client, make(chan func(), 128)
	a.prefs = settings.Defaults()
	a.p = colors(false)
	a.settingsView = newSettingsView(a.prefs)
	return a
}

func TestV31GroupedSettingsAndDefaultPage(t *testing.T) {
	if newSettingsView(settings.Defaults()).Page != "Common" {
		t.Fatal("settings did not open on Common")
	}
	for _, goos := range []string{"windows", "darwin", "linux"} {
		groups := settingsNavigation(goos)
		seen := map[string]bool{}
		if len(groups) < 4 || groups[0].Title != "Configuration" || groups[0].Pages[0] != "Common" {
			t.Fatal("missing grouped configuration navigation")
		}
		for _, g := range groups {
			for _, page := range g.Pages {
				if seen[page] {
					t.Fatal("duplicate page", page)
				}
				seen[page] = true
			}
		}
		if seen["Sandbox"] != (goos == "windows") || !seen["Appearance"] || !seen["Account"] || !seen["MCP servers"] {
			t.Fatal("platform navigation lost pages", goos)
		}
	}
	a := settingsFixture(t)
	s := a.settingsView
	s.Items = []settingsItem{{Name: "old"}}
	setText(s.Output, "old data")
	a.settingsPage("Appearance")
	if len(s.Items) != 0 || text(s.Output) != "" || s.Busy {
		t.Fatal("navigation kept stale page state")
	}
}

func TestV31ServerBannersAndProviderRestart(t *testing.T) {
	a := &App{}
	if msg, failed := a.settingsServerMessage(); !failed || !strings.Contains(msg, "not running") {
		t.Fatal(msg)
	}
	a.connecting = true
	if msg, failed := a.settingsServerMessage(); failed || !strings.Contains(msg, "starting") {
		t.Fatal(msg)
	}
	a.connecting = false
	a.serverPaused = true
	a.serverStarting = true
	if _, failed := a.settingsServerMessage(); failed {
		t.Fatal("restart was shown as a failure")
	}
	a.serverStarting = false
	a.serverError = "invalid TOML"
	if msg, failed := a.settingsServerMessage(); !failed || !strings.Contains(msg, "invalid TOML") {
		t.Fatal(msg)
	}
	a.observeProvider("openai")
	a.observeProvider("amazon-bedrock")
	if !strings.Contains(a.restartNote, "amazon-bedrock") || !strings.Contains(a.restartNote, "Restart") {
		t.Fatal("provider change lacked persistent restart notice")
	}
	a.observeProvider("openai")
	if a.restartNote != "" {
		t.Fatal("reverting provider kept stale restart requirement")
	}
	a.settingsView = newSettingsView(settings.Defaults())
	s := a.settingsView
	s.LoginBusy = true
	s.LoginID = "old-login"
	a.resetSettingsConnection()
	if s.LoginBusy || s.LoginID != "" || s.LoginGeneration == 0 || s.LoginError == "" {
		t.Fatal("server restart retained a defunct login")
	}
}

func TestV31CatalogLoadsManagedPolicyAndRejectsStaleResults(t *testing.T) {
	a := settingsFixture(t)
	a.refreshCatalog()
	drain(t, a, func() bool { return len(a.catalog.Models) > 0 })
	if a.catalog.Policy.LoginAllowed("apiKey", "") || a.catalog.Policy.Allows("sandbox_mode", "workspace-write") {
		t.Fatal("catalog dropped organization requirements")
	}
	// A disconnected/restarted service cannot publish an older catalog.
	b := settingsFixture(t)
	b.refreshCatalog()
	b.catalogGeneration++
	select {
	case f := <-b.updates:
		f()
	case <-time.After(3 * time.Second):
		t.Fatal("catalog did not complete")
	}
	if len(b.catalog.Models) != 0 {
		t.Fatal("stale catalog published")
	}
}

func TestV31CommonChoicesAndWriteEnforcePolicy(t *testing.T) {
	f := &configField{Spec: &configSpec{Choices: []string{"read-only", "workspace-write", "danger-full-access"}}}
	labels, values := configChoices(f, []string{"read-only"}, true)
	if len(labels) != 2 || len(values) != 1 || values[0] != "read-only" {
		t.Fatal(labels, values)
	}
	_, fresh := configChoices(f, []string{"server-added-choice"}, true)
	if len(fresh) != 1 || fresh[0] != "server-added-choice" {
		t.Fatal("schema snapshot hid a server-allowed value")
	}
	a := settingsFixture(t)
	a.configWrite("sandbox_mode", "read-only")
	if a.settingsView.ConfigWriting || !a.settingsView.ConfigFailed["sandbox_mode"] {
		t.Fatal("editing was allowed before policy loaded")
	}
	a.catalog.PolicyLoaded = true
	a.catalog.Policy.AllowedSandboxModes = []string{"read-only"}
	a.configWrite("sandbox_mode", "danger-full-access")
	if a.settingsView.ConfigWriting || len(a.settingsView.ConfigQueue) != 0 || !a.settingsView.ConfigFailed["sandbox_mode"] {
		t.Fatal("disallowed setting was queued")
	}
	a.catalog.Policy.AllowedLoginMethods = []string{"chatgpt"}
	a.catalog.PolicyLoaded = true
	a.login(map[string]any{"type": "apiKey", "apiKey": "fixture-only"})
	if a.settingsView.LoginBusy || !strings.Contains(a.settingsView.LoginError, "not allowed") {
		t.Fatal("disallowed sign-in reached the server")
	}
}

func TestV31SignInPolicyLoadsWithoutModelCatalog(t *testing.T) {
	a := settingsFixture(t)
	a.refreshSignInPolicy()
	drain(t, a, func() bool { return !a.policyLoading })
	if !a.catalog.PolicyLoaded || len(a.catalog.Models) != 0 || a.catalog.Policy.LoginAllowed("apiKey", "") || !a.catalog.Policy.LoginAllowed("chatgpt", "") {
		t.Fatal("sign-in policy depended on authenticated model discovery")
	}
	if value, exists := a.catalog.Requirements["memories"]; !exists || value {
		t.Fatal("refreshed managed feature requirement was lost")
	}
}

func TestV31AccountSummaryAndUsageDoNotInventValues(t *testing.T) {
	for _, tc := range []struct {
		data map[string]any
		want string
	}{
		{map[string]any{"requiresOpenaiAuth": true}, "Not signed in"},
		{map[string]any{}, "No OpenAI sign-in needed"},
		{map[string]any{"account": map[string]any{"type": "apiKey"}}, "Signed in with an API key"},
		{map[string]any{"account": map[string]any{"type": "amazonBedrock", "usesCodexManagedCredentials": true}}, "Using Amazon Bedrock"},
	} {
		title, _, _ := describeAccount(tc.data, "openai")
		if title != tc.want {
			t.Fatal(title)
		}
	}
	if len(usageRows(usageLimits{}, time.Now())) != 0 {
		t.Fatal("absent usage became zero percent")
	}
	var data usageLimits
	if err := json.Unmarshal([]byte(`{"rateLimits":{"primary":{"usedPercent":99}},"rateLimitsByLimitId":{"codex":{"primary":{"usedPercent":-5,"windowDurationMins":300,"resetsAt":1600},"secondary":{"usedPercent":120,"windowDurationMins":10080},"credits":{"hasCredits":true,"unlimited":true}}}}`), &data); err != nil {
		t.Fatal(err)
	}
	rows := usageRows(data, time.Unix(1000, 0))
	if len(rows) != 3 || !strings.Contains(rows[0].Value, "0% used") || !strings.Contains(rows[0].Value, "10m") || !strings.Contains(rows[1].Value, "100% used") || rows[2].Value != "Unlimited" {
		t.Fatal(rows)
	}
	var missing usageLimits
	_ = json.Unmarshal([]byte(`{"rateLimits":{"primary":{"usedPercent":null}}}`), &missing)
	if got := usageRows(missing, time.Now()); len(got) != 1 || got[0].Value != "Usage unavailable" {
		t.Fatal(got)
	}
}

func TestV31UsageNotificationsSurvivePendingRead(t *testing.T) {
	a := settingsFixture(t)
	a.accountUsage.Data.ByID = map[string]usageSnapshot{"codex": {Name: "Old name"}}
	a.refreshUsage()
	a.settingsEvent("account/rateLimits/updated", map[string]any{"rateLimits": map[string]any{"limitId": "codex", "primary": map[string]any{"usedPercent": 42}}})
	drain(t, a, func() bool { return !a.accountUsage.Loading })
	snapshot := a.accountUsage.Data.ByID["codex"]
	if snapshot.Name != "Fresh name" || snapshot.Primary == nil || snapshot.Primary.UsedPercent == nil || *snapshot.Primary.UsedPercent != 42 || snapshot.Primary.Minutes == nil || *snapshot.Primary.Minutes != 300 || snapshot.Secondary == nil {
		t.Fatalf("read clobbered newer sparse update: %+v", snapshot)
	}
	a.refreshAccount()
	drain(t, a, func() bool { return !a.accountLoading })
	_, _, facts := describeAccount(a.accountData, "openai")
	if len(facts) != 3 || facts[1].Value != "fixture@example.test" {
		t.Fatal("account read omitted summary facts", facts)
	}
}

func TestV31SettingsWriteShowsInlineFailureAndSuccess(t *testing.T) {
	a := settingsFixture(t)
	s := a.settingsView
	s.Page = "Skills"
	key := settingsRowKey("Skills", "fixture")
	a.settingsRequestAt(key, "fixture/fail", nil)
	if !s.ActionFeedback[key].Pending {
		t.Fatal("missing immediate Saving feedback")
	}
	drain(t, a, func() bool { return !s.ActionFeedback[key].Pending })
	if !s.ActionFeedback[key].Failed || !strings.Contains(s.ActionFeedback[key].Message, "rejected") {
		t.Fatal("failure was reported as saved")
	}
	a.settingsRequestAt(key, "skills/config/write", map[string]any{"path": "fixture", "enabled": true})
	drain(t, a, func() bool { return !s.ActionFeedback[key].Pending })
	if s.ActionFeedback[key].Failed || s.ActionFeedback[key].Message != "Saved" || a.toast == "Saved" {
		t.Fatal("successful save lacked inline confirmation")
	}
}

func TestV31AccountAndErrorRendering(t *testing.T) {
	a := &App{p: colors(false), prefs: settings.Defaults(), accountLoaded: true, accountData: map[string]any{"account": map[string]any{"type": "chatgpt", "email": "fixture@example.test", "planType": "team"}}}
	s := newSettingsView(a.prefs)
	var text []string
	h := desktop.NewHeadlessHarness(0, image.Pt(850, 900), func(w *desktop.Window) {
		a.drawAccount(w, s)
		a.drawSettingsError(w, "inline fixture failure")
		for _, c := range w.Commands().Commands {
			if c.Kind == command.TextCmd {
				text = append(text, c.Text.String)
				if c.Text.String == "inline fixture failure" && c.Text.Foreground != a.p.Danger {
					t.Error("inline error was not red")
				}
			}
		}
	})
	h.Master().SetStyle(makeStyle(a.p, 13))
	h.Frame(false)
	joined := strings.Join(text, " ")
	for _, want := range []string{"Signed in with ChatGPT", "fixture@example.test", "Usage and limits", "not available"} {
		if !strings.Contains(joined, want) {
			t.Fatal("missing account card text", want, joined)
		}
	}
}
