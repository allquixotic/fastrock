package ui

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"slices"
	"sort"
	"strings"
	"time"

	"github.com/aarzilli/nucular"
	"github.com/allquixotic/fastrock/internal/codex"
)

type localProviderView struct {
	Provider, Status, Version                           string
	Endpoint, PullName                                  *nucular.TextEditor
	Models                                              []string
	Generation, PullGeneration, Page                    int
	Busy, Failed, Running, Checked, Compatible, Pulling bool
	ProbeEndpoint, ActiveModel                          string
	Warning, PullStatus                                 string
	PullFailed                                          bool
	Progress                                            float64
	Cancel, ProbeCancel                                 context.CancelFunc
}

func providerEndpoint(value, suffix string) (string, error) {
	u, err := url.Parse(strings.TrimSpace(value))
	if err != nil || u.Host == "" || (u.Scheme != "http" && u.Scheme != "https") || u.User != nil {
		return "", fmt.Errorf("Enter an HTTP(S) model-server address without credentials")
	}
	u.RawQuery, u.Fragment = "", ""
	u.Path = strings.TrimSuffix(strings.TrimRight(u.Path, "/"), "/v1") + suffix
	return u.String(), nil
}
func validPullName(name string) bool {
	return name != "" && len(name) <= 256 && !strings.ContainsAny(name[:1], "/:.-") && !strings.ContainsAny(name[len(name)-1:], "/:") && strings.IndexFunc(name, func(r rune) bool {
		return !(r >= 'a' && r <= 'z' || r >= 'A' && r <= 'Z' || r >= '0' && r <= '9' || strings.ContainsRune("._-:/", r))
	}) < 0
}
func providerJSON(ctx context.Context, endpoint, method string, body any, result any) error {
	var input io.Reader
	if body != nil {
		data, err := json.Marshal(body)
		if err != nil {
			return err
		}
		input = bytes.NewReader(data)
	}
	r, err := http.NewRequestWithContext(ctx, method, endpoint, input)
	if err != nil {
		return err
	}
	r.Header.Set("Content-Type", "application/json")
	client := http.Client{CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	response, err := client.Do(r)
	if err != nil {
		return err
	}
	defer response.Body.Close()
	if response.StatusCode/100 != 2 {
		return localHTTPError(response.StatusCode)
	}
	return json.NewDecoder(io.LimitReader(response.Body, 4<<20)).Decode(result)
}
func localProviderTitle(provider string) string {
	if provider == "lmstudio" {
		return "LM Studio"
	}
	return "Ollama"
}
func (a *App) localProviders(s *settingsView) []*localProviderView {
	if len(s.Locals) == 0 {
		s.Locals = []*localProviderView{
			{Provider: "ollama", Endpoint: textEditor("http://127.0.0.1:11434/v1", false), PullName: textEditor("", false), Progress: -1},
			{Provider: "lmstudio", Endpoint: textEditor("http://127.0.0.1:1234/v1", false), PullName: textEditor("", false), Progress: -1},
		}
	}
	return s.Locals
}

// The Ollama form is also used by the direct download action.
func (a *App) localProvider(s *settingsView) *localProviderView { return a.localProviders(s)[0] }
func (a *App) loadLocalProvider() {
	s := a.settingsView
	if s == nil {
		return
	}
	views := a.localProviders(s)
	s.LocalLoadGeneration++
	generation := s.LocalLoadGeneration
	a.rpcInline("config/read", map[string]any{"cwd": a.prefs.WorkingDirectory}, func(raw json.RawMessage) {
		if generation != s.LocalLoadGeneration {
			return
		}
		config, _ := codex.Decode(raw)["config"].(map[string]any)
		providers, _ := config["model_providers"].(map[string]any)
		for _, v := range views {
			provider, _ := providers[v.Provider].(map[string]any)
			if endpoint := str(provider, "base_url"); endpoint != "" {
				setText(v.Endpoint, endpoint)
			}
			v.ActiveModel = ""
			if str(config, "model_provider") == v.Provider {
				v.ActiveModel = str(config, "model")
			}
			a.refreshLocal(v)
		}
	}, func(err error) {
		if generation != s.LocalLoadGeneration {
			return
		}
		for _, v := range views {
			v.Status = "Could not read model-server configuration. Refresh to retry."
			v.Failed = true
			v.Busy = false
		}
	})
}
func (a *App) refreshLocal(v *localProviderView) {
	if v.ProbeCancel != nil {
		v.ProbeCancel()
	}
	v.Generation++
	a.probeLocal(v, v.Generation, v.Provider, text(v.Endpoint))
}
func (a *App) probeLocal(v *localProviderView, generation int, provider, endpoint string) {
	v.Busy = true
	v.Failed = false
	v.Status = "Checking…"
	ctx, cancel := context.WithTimeout(a.ctx, 10*time.Second)
	v.ProbeCancel = cancel
	a.work(func() {
		defer cancel()
		u, err := providerEndpoint(endpoint, "/v1/models")
		var response struct{ Data []struct{ ID string } }
		if err == nil {
			err = providerJSON(ctx, u, "GET", nil, &response)
		}
		models := make([]string, 0, len(response.Data))
		for _, m := range response.Data {
			if m.ID != "" {
				models = append(models, m.ID)
			}
		}
		sort.Strings(models)
		models = slices.Compact(models)
		version, warning := "", ""
		compatible := provider != "ollama"
		if provider == "ollama" && err == nil {
			var info struct{ Version string }
			u, _ = providerEndpoint(endpoint, "/api/version")
			versionErr := providerJSON(ctx, u, "GET", nil, &info)
			version = info.Version
			compatible = versionErr == nil && ollamaSupported(version)
			if !compatible {
				warning = "Ollama 0.13.4 or newer is required to use models with Codex. Upgrade Ollama and refresh."
				if version == "" {
					warning = "Could not verify the Ollama version. Install Ollama 0.13.4 or newer and refresh."
				}
			}
		}
		a.post(func() {
			if generation != v.Generation {
				return
			}
			v.Busy = false
			v.ProbeCancel = nil
			v.Checked = true
			v.Running = err == nil
			v.Compatible = compatible && err == nil
			v.ProbeEndpoint = endpoint
			v.Models = models
			v.Version = version
			v.Warning = warning
			v.Failed = err != nil
			v.Page = 0
			v.Status = "Running"
			if err != nil {
				v.Models = nil
				v.Status = localProviderError(provider, err)
			}
		})
	}, func() {
		cancel()
		if generation == v.Generation {
			v.Busy = false
			v.ProbeCancel = nil
			v.Failed = true
			v.Status = errWorkQueueFull.Error()
		}
	})
}
func localProviderCanUse(v *localProviderView) bool {
	return v.Running && v.Compatible && !v.Busy && strings.TrimSpace(text(v.Endpoint)) == strings.TrimSpace(v.ProbeEndpoint)
}
func (a *App) useLocalModel(v *localProviderView, model string) {
	if !localProviderCanUse(v) || !slices.Contains(v.Models, model) {
		v.Status = "Refresh this server and resolve its version warning before using a model."
		v.Failed = true
		return
	}
	provider, endpoint := v.Provider, text(v.Endpoint)
	a.confirm("Change model provider?", "Restart Codex in all windows and use "+model+"? Running turns will stop.", func() {
		if !localProviderCanUse(v) || endpoint != text(v.Endpoint) {
			return
		}
		edits := []map[string]any{{"keyPath": "model_provider", "value": provider, "mergeStrategy": "replace"}, {"keyPath": "model", "value": model, "mergeStrategy": "replace"}, {"keyPath": "model_providers." + provider + ".base_url", "value": endpoint, "mergeStrategy": "replace"}}
		a.settingsFormCall("provider", "config/batchWrite", map[string]any{"edits": edits, "reloadUserConfig": true}, func(json.RawMessage) { a.restartServer() })
	})
}
func (a *App) drawLocalProviders(w *nucular.Window, s *settingsView) {
	for _, v := range a.localProviders(s) {
		a.drawLocalProvider(w, v)
	}
}
func (a *App) drawLocalProvider(w *nucular.Window, v *localProviderView) {
	status := "Not checked"
	if v.Busy {
		status = "Checking…"
	} else if v.Running {
		status = "Running"
	} else if v.Checked {
		status = "Not running"
	}
	title(w, localProviderTitle(v.Provider)+" · "+status, a.p)
	a.field(w, "Server address", v.Endpoint, false)
	if v.Version != "" {
		muted(w, "Version "+v.Version, a.p)
	}
	if v.Failed {
		a.drawSettingsError(w, v.Status)
	}
	if v.Warning != "" {
		a.drawSettingsError(w, v.Warning)
	}
	w.Row(28).Dynamic(1)
	if v.Busy {
		w.Label("Checking server…", "LC")
	} else if w.ButtonText("Refresh " + localProviderTitle(v.Provider)) {
		a.refreshLocal(v)
	}
	if v.Running && len(v.Models) == 0 {
		muted(w, "No installed models found.", a.p)
	}
	pageCount := (len(v.Models) + 19) / 20
	v.Page = max(0, min(v.Page, pageCount-1))
	for _, model := range v.Models[min(v.Page*20, len(v.Models)):min((v.Page+1)*20, len(v.Models))] {
		w.Row(28).Ratio(.72, .28)
		w.Label(cut(model, 80), "LC")
		if v.ActiveModel == model {
			w.LabelColored("In use", "LC", a.p.Success)
		} else if localProviderCanUse(v) {
			if w.ButtonText("Use") {
				a.useLocalModel(v, model)
			}
		} else {
			w.Label("Unavailable", "LC")
		}
	}
	if pageCount > 1 {
		w.Row(27).Dynamic(3)
		if v.Page > 0 {
			if w.ButtonText("Previous models") {
				v.Page--
			}
		} else {
			w.Label("", "LC")
		}
		w.Label(fmt.Sprintf("%d of %d", v.Page+1, pageCount), "LC")
		if v.Page+1 < pageCount && w.ButtonText("More models") {
			v.Page++
		}
	}
	if v.Provider != "ollama" {
		return
	}
	if v.Pulling {
		muted(w, v.PullStatus, a.p)
		if v.Progress >= 0 {
			progress := int(v.Progress * 100)
			w.Row(14).Dynamic(1)
			w.Progress(&progress, 100, false)
		}
		w.Row(28).Dynamic(1)
		if w.ButtonText("Cancel download") {
			a.cancelLocalPull(v)
		}
		return
	}
	if v.PullStatus != "" {
		if v.PullFailed {
			a.drawSettingsError(w, v.PullStatus)
		} else {
			muted(w, v.PullStatus, a.p)
		}
	}
	if v.Running && !v.Busy && text(v.Endpoint) == v.ProbeEndpoint {
		a.field(w, "Download Ollama model", v.PullName, false)
		if validPullName(strings.TrimSpace(text(v.PullName))) {
			w.Row(28).Dynamic(1)
			if w.ButtonText("Pull model") {
				a.pullLocal(v)
			}
		}
	}
}
