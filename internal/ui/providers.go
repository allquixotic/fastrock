package ui

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/aarzilli/nucular"
	"github.com/allquixotic/fastrock/internal/codex"
)

type localProviderView struct {
	Provider, Status, Version string
	Endpoint, PullName        *nucular.TextEditor
	Models                    []string
	Selected, Generation      int
	Busy                      bool
	Cancel                    context.CancelFunc
}

func providerEndpoint(value, suffix string) (string, error) {
	u, err := url.Parse(value)
	if err != nil || u.Host == "" || (u.Scheme != "http" && u.Scheme != "https") || u.User != nil {
		return "", fmt.Errorf("enter an HTTP(S) model-server endpoint")
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
		return fmt.Errorf("model server returned HTTP %d", response.StatusCode)
	}
	return json.NewDecoder(io.LimitReader(response.Body, 4<<20)).Decode(result)
}
func (a *App) localProvider(s *settingsView) *localProviderView {
	if s.Local == nil {
		s.Local = &localProviderView{Provider: "ollama", Endpoint: textEditor("http://127.0.0.1:11434/v1", false), PullName: textEditor("", false)}
	}
	return s.Local
}
func (a *App) loadLocalProvider() {
	s := a.settingsView
	v := a.localProvider(s)
	v.Busy = true
	v.Generation++
	generation := v.Generation
	provider := v.Provider
	a.rpcResult("config/read", map[string]any{"cwd": a.prefs.WorkingDirectory}, func(raw json.RawMessage) {
		r := codex.Decode(raw)
		config, _ := r["config"].(map[string]any)
		providers, _ := config["model_providers"].(map[string]any)
		p, _ := providers[provider].(map[string]any)
		endpoint := str(p, "base_url")
		if endpoint == "" {
			endpoint = text(v.Endpoint)
		}
		setText(v.Endpoint, endpoint)
		a.probeLocal(v, generation, provider, endpoint)
	}, func(err error) { v.Busy = false; v.Status = err.Error() })
}
func (a *App) probeLocal(v *localProviderView, generation int, provider, endpoint string) {
	v.Busy = true
	a.work(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 10*time.Second)
		defer cancel()
		u, err := providerEndpoint(endpoint, "/v1/models")
		var response struct{ Data []struct{ ID string } }
		if err == nil {
			err = providerJSON(ctx, u, "GET", nil, &response)
		}
		models := make([]string, 0, len(response.Data))
		for _, m := range response.Data {
			models = append(models, m.ID)
		}
		sort.Strings(models)
		version := ""
		if provider == "ollama" && err == nil {
			var info struct{ Version string }
			u, _ = providerEndpoint(endpoint, "/api/version")
			if providerJSON(ctx, u, "GET", nil, &info) == nil {
				version = info.Version
			}
		}
		a.post(func() {
			if generation != v.Generation {
				return
			}
			v.Busy = false
			v.Models = models
			v.Version = version
			v.Status = "Connected"
			if err != nil {
				v.Status = err.Error()
			}
		})
	})
}
func (a *App) pullLocal(v *localProviderView) {
	name, endpoint := strings.TrimSpace(text(v.PullName)), text(v.Endpoint)
	if !validPullName(name) {
		a.toast = "Enter a model name such as gpt-oss:20b"
		return
	}
	u, err := providerEndpoint(endpoint, "/api/pull")
	if err != nil {
		a.report(err)
		return
	}
	ctx, cancel := context.WithCancel(a.ctx)
	v.Cancel = cancel
	v.Busy = true
	v.Generation++
	generation := v.Generation
	v.Status = "Starting download…"
	a.work(func() {
		defer cancel()
		body, _ := json.Marshal(map[string]any{"name": name, "stream": true})
		req, err := http.NewRequestWithContext(ctx, "POST", u, bytes.NewReader(body))
		if err == nil {
			req.Header.Set("Content-Type", "application/json")
			client := http.Client{CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
			var response *http.Response
			response, err = client.Do(req)
			if err == nil {
				defer response.Body.Close()
				if response.StatusCode/100 != 2 {
					err = fmt.Errorf("download returned HTTP %d", response.StatusCode)
				} else {
					scan := bufio.NewScanner(response.Body)
					scan.Buffer(make([]byte, 4096), 1<<20)
					last := time.Time{}
					success := false
					for scan.Scan() {
						var event struct {
							Status, Error    string
							Completed, Total uint64
						}
						if e := json.Unmarshal(scan.Bytes(), &event); e != nil {
							err = e
							break
						}
						if event.Error != "" {
							err = fmt.Errorf("%s", event.Error)
							break
						}
						if event.Status == "success" {
							success = true
							break
						}
						if time.Since(last) > 150*time.Millisecond {
							status := event.Status
							if event.Total > 0 {
								status += " · " + strconv.FormatUint(event.Completed*100/event.Total, 10) + "%"
							}
							a.post(func() {
								if generation == v.Generation {
									v.Status = status
								}
							})
							last = time.Now()
						}
					}
					if err == nil {
						err = scan.Err()
					}
					if err == nil && !success {
						err = fmt.Errorf("download ended before success")
					}
				}
			}
		}
		a.post(func() {
			if generation != v.Generation {
				return
			}
			v.Cancel = nil
			v.Busy = false
			if err != nil {
				v.Status = err.Error()
				return
			}
			v.Status = "Download complete"
			a.probeLocal(v, generation, "ollama", endpoint)
		})
	})
}
func (a *App) drawLocalProviders(w *nucular.Window, s *settingsView) {
	v := a.localProvider(s)
	w.Row(30).Dynamic(2)
	for _, p := range []string{"ollama", "lmstudio"} {
		if button(w, p, v.Provider == p, a.p) && !v.Busy {
			v.Provider = p
			if p == "ollama" {
				setText(v.Endpoint, "http://127.0.0.1:11434/v1")
			} else {
				setText(v.Endpoint, "http://127.0.0.1:1234/v1")
			}
			a.loadLocalProvider()
		}
	}
	a.field(w, "Server endpoint (from Codex configuration)", v.Endpoint, false)
	w.Row(30).Static(140)
	if w.ButtonText("Refresh models") && !v.Busy {
		v.Generation++
		a.probeLocal(v, v.Generation, v.Provider, text(v.Endpoint))
	}
	muted(w, v.Status, a.p)
	if v.Provider == "ollama" && v.Version != "" {
		muted(w, "Ollama "+v.Version+" · Codex Responses support requires 0.13.4+", a.p)
	}
	if len(v.Models) > 0 {
		w.Row(30).Dynamic(1)
		v.Selected = w.ComboSimple(v.Models, min(v.Selected, len(v.Models)-1), 28)
		w.Row(30).Static(200)
		if w.ButtonText("Use model and restart…") {
			provider, model, endpoint := v.Provider, v.Models[v.Selected], text(v.Endpoint)
			a.confirm("Change model provider?", "Restart Codex in all windows and use "+model+"? Running turns will stop.", func() {
				edits := []map[string]any{{"keyPath": "model_provider", "value": provider, "mergeStrategy": "replace"}, {"keyPath": "model", "value": model, "mergeStrategy": "replace"}, {"keyPath": "model_providers." + provider + ".base_url", "value": endpoint, "mergeStrategy": "replace"}}
				a.rpc("config/batchWrite", map[string]any{"edits": edits, "reloadUserConfig": true}, func(json.RawMessage) { a.rpc("fastrock/restart", map[string]any{}, nil) })
			})
		}
	}
	if v.Provider == "ollama" {
		a.field(w, "Download Ollama model", v.PullName, false)
		w.Row(30).Dynamic(2)
		if w.ButtonText("Download") && !v.Busy {
			a.pullLocal(v)
		}
		if w.ButtonText("Cancel download") && v.Cancel != nil {
			v.Cancel()
			v.Cancel = nil
			v.Generation++
			v.Busy = false
			v.Status = "Download cancelled"
		}
	}
}
