package ui

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"strconv"
	"strings"
	"time"
)

type localHTTPError int

func (e localHTTPError) Error() string { return fmt.Sprintf("HTTP %d", int(e)) }

type localMessageError string

func (e localMessageError) Error() string { return string(e) }
func localProviderError(provider string, err error) string {
	name := localProviderTitle(provider)
	if errors.Is(err, context.Canceled) {
		return "Download cancelled"
	}
	if errors.Is(err, context.DeadlineExceeded) {
		return name + " did not respond in time. Check the server and try again."
	}
	var network net.Error
	if errors.As(err, &network) {
		return "Could not reach " + name + ". Start the server and check its address."
	}
	var httpError localHTTPError
	if errors.As(err, &httpError) {
		if httpError == 404 {
			return name + " does not provide the required API at this address. Check the address and server version."
		}
		if httpError == 401 || httpError == 403 {
			return name + " refused access. Check the server's authentication settings."
		}
		return fmt.Sprintf("%s returned HTTP %d. Check the server and try again.", name, httpError)
	}
	var message localMessageError
	if errors.As(err, &message) {
		return string(message)
	}
	if errors.Is(err, errWorkQueueFull) {
		return "The app is busy. Try again shortly."
	}
	return name + " returned an incomplete or invalid response. Check the server and try again."
}
func ollamaSupported(version string) bool {
	version = strings.TrimPrefix(strings.TrimSpace(version), "v")
	version = strings.SplitN(version, "+", 2)[0]
	base := strings.SplitN(version, "-", 2)
	parts := strings.Split(base[0], ".")
	if len(parts) != 3 {
		return false
	}
	var numbers [3]uint64
	for i, p := range parts {
		n, err := strconv.ParseUint(p, 10, 32)
		if err != nil {
			return false
		}
		numbers[i] = n
	}
	minimum := [3]uint64{0, 13, 4}
	for i, n := range numbers {
		if n > minimum[i] {
			return true
		}
		if n < minimum[i] {
			return false
		}
	}
	return len(base) == 1
}

type localPullEvent struct {
	Status, Error, Digest string
	Completed, Total      *uint64
}
type localPullLayer struct{ Completed, Total uint64 }
type localPullProgress struct {
	Status string
	Layers map[string]localPullLayer
}

func (p *localPullProgress) apply(e localPullEvent) error {
	if e.Error != "" {
		return localMessageError("Ollama could not download the model: " + cut(e.Error, 400))
	}
	if e.Status != "" {
		p.Status = e.Status
	}
	if e.Digest != "" {
		if p.Layers == nil {
			p.Layers = map[string]localPullLayer{}
		}
		if _, ok := p.Layers[e.Digest]; !ok && len(p.Layers) >= 4096 {
			return localMessageError("Ollama returned too many download layers. Stop and retry.")
		}
		layer := p.Layers[e.Digest]
		if e.Total != nil {
			layer.Total = *e.Total
		}
		if e.Completed != nil {
			layer.Completed = *e.Completed
		}
		p.Layers[e.Digest] = layer
	}
	return nil
}
func localBytes(n float64) string {
	units := []string{"B", "KB", "MB", "GB", "TB"}
	i := 0
	for n >= 1000 && i < len(units)-1 {
		n /= 1000
		i++
	}
	if i == 0 {
		return fmt.Sprintf("%.0f B", n)
	}
	return fmt.Sprintf("%.1f %s", n, units[i])
}
func (p *localPullProgress) snapshot() (string, float64) {
	var total, done float64
	for _, layer := range p.Layers {
		total += float64(layer.Total)
		done += float64(min(layer.Completed, layer.Total))
	}
	if total > 0 {
		fraction := done / total
		return fmt.Sprintf("Downloading %s of %s (%.0f%%)", localBytes(done), localBytes(total), fraction*100), fraction
	}
	status := strings.TrimSpace(p.Status)
	if status == "" {
		status = "Starting download…"
	}
	return status, -1
}
func pullLocalStream(ctx context.Context, endpoint, name string, progress func(string, float64)) error {
	body, _ := json.Marshal(map[string]any{"name": name, "stream": true})
	req, err := http.NewRequestWithContext(ctx, "POST", endpoint, bytes.NewReader(body))
	if err != nil {
		return err
	}
	req.Header.Set("Content-Type", "application/json")
	client := http.Client{CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	response, err := client.Do(req)
	if err != nil {
		return err
	}
	defer response.Body.Close()
	if response.StatusCode/100 != 2 {
		return localHTTPError(response.StatusCode)
	}
	scan := bufio.NewScanner(response.Body)
	scan.Buffer(make([]byte, 4096), 1<<20)
	state := localPullProgress{}
	last := time.Time{}
	for scan.Scan() {
		var event localPullEvent
		if err = json.Unmarshal(scan.Bytes(), &event); err != nil {
			return err
		}
		if err = state.apply(event); err != nil {
			return err
		}
		if event.Status == "success" {
			progress("Download complete", 1)
			return nil
		}
		if time.Since(last) > 150*time.Millisecond {
			status, fraction := state.snapshot()
			progress(status, fraction)
			last = time.Now()
		}
	}
	if err = scan.Err(); err != nil {
		return err
	}
	return io.ErrUnexpectedEOF
}
func (a *App) pullLocal(v *localProviderView) {
	name, endpoint := strings.TrimSpace(text(v.PullName)), text(v.Endpoint)
	if !validPullName(name) {
		v.Status = "Enter a model name such as gpt-oss:20b"
		v.Failed = true
		return
	}
	if v.Pulling {
		return
	}
	if v.Provider != "ollama" || !v.Running || endpoint != v.ProbeEndpoint {
		v.Status = "Refresh Ollama and connect to the server before downloading a model."
		v.Failed = true
		return
	}
	u, err := providerEndpoint(endpoint, "/api/pull")
	if err != nil {
		v.Status = err.Error()
		v.Failed = true
		return
	}
	ctx, cancel := context.WithCancel(a.ctx)
	v.Cancel = cancel
	v.Pulling = true
	v.PullGeneration++
	generation := v.PullGeneration
	v.PullStatus = "Starting download…"
	v.PullFailed = false
	v.Progress = -1
	a.work(func() {
		defer cancel()
		err := pullLocalStream(ctx, u, name, func(status string, fraction float64) {
			a.post(func() {
				if generation == v.PullGeneration {
					v.PullStatus = status
					v.Progress = fraction
				}
			})
		})
		a.post(func() {
			if generation != v.PullGeneration {
				return
			}
			v.Cancel = nil
			v.Pulling = false
			v.PullFailed = err != nil
			if err != nil {
				v.PullStatus = localProviderError("ollama", err)
				return
			}
			v.PullStatus = "Download complete"
			v.Progress = 1
			a.refreshLocal(v)
		})
	}, func() {
		cancel()
		if generation == v.PullGeneration {
			v.Pulling = false
			v.Cancel = nil
			v.PullStatus = localProviderError("ollama", errWorkQueueFull)
			v.PullFailed = true
		}
	})
}
func (a *App) cancelLocalPull(v *localProviderView) {
	if v.Cancel != nil {
		v.Cancel()
	}
	v.Cancel = nil
	v.PullGeneration++
	v.Pulling = false
	v.PullStatus = "Download cancelled"
	v.PullFailed = false
	v.Progress = -1
}
func (a *App) resetLocalProviders(s *settingsView) {
	s.LocalLoadGeneration++
	for _, v := range s.Locals {
		if v.ProbeCancel != nil {
			v.ProbeCancel()
		}
		v.ProbeCancel = nil
		v.Generation++
		v.Busy = false
		if v.Pulling {
			a.cancelLocalPull(v)
		}
	}
}
