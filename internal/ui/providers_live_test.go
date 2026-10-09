//go:build nucular_headless

package ui

import (
	"context"
	"encoding/json"
	"fmt"
	"image"
	"io"
	"math"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/command"
)

func TestV36LocalVersionGate(t *testing.T) {
	for version, want := range map[string]bool{"0.13.3": false, "0.13.4": true, "v0.13.4": true, "0.13.4-rc1": false, "0.13.5-rc1": true, "0.14.0": true, "1.0.0": true, "0.13.4+custom": true, "": false, "nonsense": false, "0.13": false} {
		if got := ollamaSupported(version); got != want {
			t.Fatal(version, got, want)
		}
	}
	a := settingsFixture(t)
	v := a.localProvider(a.settingsView)
	v.Models = []string{"unsafe"}
	v.Running = true
	v.ProbeEndpoint = text(v.Endpoint)
	a.useLocalModel(v, "unsafe")
	if !v.Failed || !strings.Contains(v.Status, "version warning") {
		t.Fatal("old server allowed model switch", v.Status)
	}
}
func TestV36PullProgressAggregatesLayers(t *testing.T) {
	ptr := func(v uint64) *uint64 { return &v }
	p := localPullProgress{}
	for _, e := range []localPullEvent{{Digest: "a", Total: ptr(3000), Completed: ptr(1500)}, {Digest: "b", Total: ptr(1000), Completed: ptr(500)}, {Digest: "a", Completed: ptr(2400)}} {
		if err := p.apply(e); err != nil {
			t.Fatal(err)
		}
	}
	status, fraction := p.snapshot()
	if fraction != .725 || !strings.Contains(status, "2.9 KB of 4.0 KB") {
		t.Fatal(status, fraction)
	}
	_ = p.apply(localPullEvent{Digest: "a", Completed: ptr(math.MaxUint64)})
	_, fraction = p.snapshot()
	if fraction != .875 {
		t.Fatal("progress was not clamped", fraction)
	}
	_ = p.apply(localPullEvent{Digest: "c", Total: ptr(math.MaxUint64), Completed: ptr(math.MaxUint64)})
	_, fraction = p.snapshot()
	if fraction < .99 || fraction > 1 {
		t.Fatal("aggregate overflow", fraction)
	}
}
func TestV36ProbeBothServersAndBlockOldOllama(t *testing.T) {
	a := settingsFixture(t)
	versions := []string{"0.13.3", "0.13.4"}
	index := 0
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/v1/models":
			io.WriteString(w, `{"data":[{"id":"z"},{"id":"a"},{"id":"a"}]}`)
		case "/api/version":
			fmt.Fprintf(w, `{"version":%q}`, versions[index])
		default:
			http.NotFound(w, r)
		}
	}))
	defer server.Close()
	views := a.localProviders(a.settingsView)
	if len(views) != 2 {
		t.Fatal(views)
	}
	for _, v := range views {
		setText(v.Endpoint, server.URL+"/v1")
		a.refreshLocal(v)
	}
	drain(t, a, func() bool { return !views[0].Busy && !views[1].Busy })
	if !views[0].Running || views[0].Compatible || views[0].Warning == "" || !views[1].Running || !localProviderCanUse(views[1]) || strings.Join(views[1].Models, ",") != "a,z" {
		t.Fatal(views[0], views[1])
	}
	index = 1
	a.refreshLocal(views[0])
	drain(t, a, func() bool { return !views[0].Busy })
	if !localProviderCanUse(views[0]) || views[0].Warning != "" {
		t.Fatal(views[0])
	}
	setText(views[0].Endpoint, server.URL+"/other")
	if localProviderCanUse(views[0]) {
		t.Fatal("edited address used an old probe")
	}
}
func TestV36PullStreamCompletionAndFailure(t *testing.T) {
	for _, tc := range []struct {
		body    string
		wantErr bool
	}{{`{"status":"pulling","digest":"a","total":100,"completed":40}` + "\n" + `{"status":"success"}` + "\n", false}, {`{"status":"pulling"}` + "\n", true}, {`{"error":"model was not found"}` + "\n", true}} {
		server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			var body map[string]any
			if err := json.NewDecoder(r.Body).Decode(&body); err != nil || body["name"] != "fixture:latest" {
				t.Error(body, err)
			}
			io.WriteString(w, tc.body)
		}))
		var status string
		var fraction float64
		err := pullLocalStream(context.Background(), server.URL, "fixture:latest", func(s string, f float64) { status, fraction = s, f })
		server.Close()
		if (err != nil) != tc.wantErr {
			t.Fatal(err)
		}
		if !tc.wantErr && (status != "Download complete" || fraction != 1) {
			t.Fatal(status, fraction)
		}
		if tc.wantErr && strings.Contains(localProviderError("ollama", err), "unexpected EOF") {
			t.Fatal("raw transport error shown")
		}
	}
}
func TestV36CancelPullAndDiscardOldProbe(t *testing.T) {
	a := settingsFixture(t)
	v := a.localProvider(a.settingsView)
	started := make(chan struct{})
	ended := make(chan struct{})
	release := make(chan struct{})
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/api/pull" {
			_, _ = io.Copy(io.Discard, r.Body)
			close(started)
			select {
			case <-r.Context().Done():
			case <-release:
			}
			close(ended)
			return
		}
		io.WriteString(w, `{"data":[]}`)
	}))
	defer func() { close(release); server.Close() }()
	setText(v.Endpoint, server.URL)
	setText(v.PullName, "fixture:latest")
	v.ProbeEndpoint = server.URL
	v.Running = true
	a.pullLocal(v)
	select {
	case <-started:
	case <-time.After(3 * time.Second):
		t.Fatal("pull did not start")
	}
	a.cancelLocalPull(v)
	select {
	case <-ended:
	case <-time.After(3 * time.Second):
		t.Fatal("download connection was not cancelled")
	}
	select {
	case publish := <-a.updates:
		publish()
	case <-time.After(3 * time.Second):
		t.Fatal("missing cancellation completion")
	}
	if v.Pulling || v.PullStatus != "Download cancelled" || v.PullFailed {
		t.Fatal(v)
	}
	a.refreshLocal(v)
	select {
	case publish := <-a.updates:
		a.resetLocalProviders(a.settingsView)
		v.Status = "new connection"
		publish()
	case <-time.After(3 * time.Second):
		t.Fatal("missing probe")
	}
	if v.Status != "new connection" || v.Busy {
		t.Fatal("old probe survived reset")
	}
}
func TestV36LocalCardsAndContextualActions(t *testing.T) {
	a := settingsFixture(t)
	views := a.localProviders(a.settingsView)
	views[0].Running = true
	views[0].Checked = true
	views[0].Compatible = true
	views[0].ProbeEndpoint = text(views[0].Endpoint)
	views[0].Models = []string{"model"}
	views[0].ActiveModel = "model"
	views[1].Checked = true
	var labels []string
	h := nucular.NewHeadlessHarness(0, image.Pt(800, 1600), func(w *nucular.Window) {
		a.drawLocalProviders(w, a.settingsView)
		for _, c := range w.Commands().Commands {
			if c.Kind == command.TextCmd {
				labels = append(labels, c.Text.String)
			}
		}
	})
	h.Master().SetStyle(makeStyle(a.p, 13))
	h.Frame(false)
	text := strings.Join(labels, "\n")
	for _, want := range []string{"Ollama · Running", "LM Studio · Not running", "In use"} {
		if !strings.Contains(text, want) {
			t.Fatal(text, want)
		}
	}
	if strings.Contains(text, "Cancel download") || strings.Contains(text, "Pull model") {
		t.Fatal("irrelevant download actions displayed", text)
	}
}
