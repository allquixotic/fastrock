// Package codex speaks newline-delimited JSON-RPC to the installed Codex CLI.
package codex

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os/exec"
	"regexp"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"time"
)

const SupportedVersion = "0.162.0"

type RPCError struct {
	Code    int             `json:"code"`
	Message string          `json:"message"`
	Data    json.RawMessage `json:"data,omitempty"`
}

func (e *RPCError) Error() string { return fmt.Sprintf("Codex: %s (%d)", e.Message, e.Code) }

type Message struct {
	Sequence uint64          `json:"fastrockSequence,omitempty"`
	ID       json.RawMessage `json:"id,omitempty"`
	Method   string          `json:"method,omitempty"`
	Params   json.RawMessage `json:"params,omitempty"`
	Result   json.RawMessage `json:"result,omitempty"`
	Error    *RPCError       `json:"error,omitempty"`
}
type Client struct {
	cmd       *exec.Cmd
	input     io.WriteCloser
	ctx       context.Context
	cancel    context.CancelFunc
	seq       atomic.Uint64
	mu        sync.Mutex
	writes    sync.Mutex
	pending   map[string]chan Message
	Events    chan Message
	done      chan struct{}
	closeOnce sync.Once
	Version   string
}

var versionPattern = regexp.MustCompile(`(?:codex-cli\s+)?(\d+)\.(\d+)\.(\d+)([^\s]*)`)

func CheckVersion(output string) error {
	m := versionPattern.FindStringSubmatch(output)
	if m == nil {
		return errors.New("cannot determine Codex CLI version; install the latest Codex CLI")
	}
	major, _ := strconv.Atoi(m[1])
	minor, _ := strconv.Atoi(m[2])
	if major == 0 && minor < 162 {
		return fmt.Errorf("Codex %s is too old; Fastrock supports Codex %s or newer. Update the installed CLI", strings.Join(m[1:4], "."), SupportedVersion)
	}
	return nil
}

// Start never downloads Codex or changes CODEX_HOME/config.toml.
func Start(ctx context.Context) (*Client, error) {
	binary, e := exec.LookPath("codex")
	if e != nil {
		return nil, errors.New("Codex CLI was not found on PATH. Install the latest Codex CLI, then reopen Fastrock")
	}
	probeCtx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	probe := command(probeCtx, binary, "--version")
	out, e := probe.Output()
	if e != nil {
		return nil, fmt.Errorf("cannot run installed Codex CLI: %w", e)
	}
	if e = CheckVersion(string(out)); e != nil {
		return nil, e
	}
	c, e := StartCommand(ctx, binary, []string{"app-server"})
	if e != nil {
		return nil, e
	}
	c.Version = strings.TrimSpace(string(out))
	initCtx, stop := context.WithTimeout(ctx, 30*time.Second)
	defer stop()
	var result map[string]any
	e = c.Call(initCtx, "initialize", map[string]any{"clientInfo": map[string]any{"name": "fastrock", "title": "Fastrock", "version": "1.0.0"}, "capabilities": map[string]any{"experimentalApi": true}}, &result)
	if e == nil {
		e = c.Notify("initialized", map[string]any{})
	}
	if e != nil {
		c.Close()
		return nil, fmt.Errorf("installed Codex app-server is incompatible: %w; update Codex CLI", e)
	}
	return c, nil
}

// StartCommand is also used with a subprocess fixture; it does not initialize.
func StartCommand(parent context.Context, binary string, args []string) (*Client, error) {
	ctx, cancel := context.WithCancel(parent)
	cmd := command(ctx, binary, args...)
	input, e := cmd.StdinPipe()
	if e != nil {
		cancel()
		return nil, e
	}
	output, e := cmd.StdoutPipe()
	if e != nil {
		cancel()
		return nil, e
	}
	stderr, e := cmd.StderrPipe()
	if e != nil {
		cancel()
		return nil, e
	}
	c := &Client{cmd: cmd, input: input, ctx: ctx, cancel: cancel, pending: map[string]chan Message{}, Events: make(chan Message, 1024), done: make(chan struct{})}
	if e = cmd.Start(); e != nil {
		cancel()
		return nil, e
	}
	go func() { _, _ = io.Copy(io.Discard, stderr) }()
	go c.read(output)
	return c, nil
}
func (c *Client) read(output io.Reader) {
	defer close(c.done)
	defer close(c.Events)
	defer c.cancel()
	scanner := bufio.NewScanner(output)
	scanner.Buffer(make([]byte, 64<<10), 32<<20)
	var readErr error
	for scanner.Scan() {
		var m Message
		if e := json.Unmarshal(scanner.Bytes(), &m); e != nil {
			readErr = fmt.Errorf("invalid JSON from Codex app-server: %w", e)
			break
		}
		if m.Method != "" {
			select {
			case c.Events <- m:
			case <-c.ctx.Done():
				break
			}
			continue
		}
		c.mu.Lock()
		ch := c.pending[string(m.ID)]
		delete(c.pending, string(m.ID))
		c.mu.Unlock()
		if ch != nil {
			ch <- m
		}
	}
	if readErr == nil {
		readErr = scanner.Err()
	}
	if readErr == nil {
		readErr = io.EOF
	}
	c.mu.Lock()
	for id, ch := range c.pending {
		ch <- Message{Error: &RPCError{Code: -32000, Message: "app-server disconnected: " + readErr.Error()}}
		delete(c.pending, id)
	}
	c.mu.Unlock()
	_ = c.input.Close()
	c.cancel()
	if c.cmd != nil {
		_ = c.cmd.Wait()
	}
}
func (c *Client) write(v any) error {
	b, e := json.Marshal(v)
	if e != nil {
		return e
	}
	c.writes.Lock()
	defer c.writes.Unlock()
	_, e = c.input.Write(append(b, '\n'))
	return e
}
func (c *Client) Call(ctx context.Context, method string, params, out any) error {
	id := c.seq.Add(1)
	key := strconv.FormatUint(id, 10)
	ch := make(chan Message, 1)
	c.mu.Lock()
	c.pending[key] = ch
	c.mu.Unlock()
	defer func() { c.mu.Lock(); delete(c.pending, key); c.mu.Unlock() }()
	if e := c.write(map[string]any{"id": id, "method": method, "params": params}); e != nil {
		return e
	}
	select {
	case m := <-ch:
		if m.Error != nil {
			return m.Error
		}
		if out == nil {
			return nil
		}
		return json.Unmarshal(m.Result, out)
	case <-ctx.Done():
		return ctx.Err()
	case <-c.ctx.Done():
		return errors.New("Codex app-server stopped")
	}
}
func (c *Client) Notify(method string, params any) error {
	return c.write(map[string]any{"method": method, "params": params})
}
func (c *Client) Respond(id json.RawMessage, result any) error {
	return c.write(map[string]any{"id": id, "result": result})
}
func (c *Client) Reject(id json.RawMessage, message string) error {
	return c.write(map[string]any{"id": id, "error": RPCError{Code: -32601, Message: message}})
}
func (c *Client) Close() {
	c.closeOnce.Do(func() {
		_ = c.input.Close()
		c.cancel()
		select {
		case <-c.done:
		case <-time.After(3 * time.Second):
			if c.cmd != nil && c.cmd.Process != nil {
				_ = c.cmd.Process.Kill()
			}
		}
	})
}

type Tier struct {
	ID          string `json:"id"`
	Name        string `json:"name"`
	Description string `json:"description"`
}
type Effort struct {
	ID          string `json:"reasoningEffort"`
	Description string `json:"description"`
}
type Model struct {
	ID              string   `json:"id"`
	Model           string   `json:"model"`
	Name            string   `json:"displayName"`
	Default         bool     `json:"isDefault"`
	Hidden          bool     `json:"hidden"`
	Tiers           []Tier   `json:"serviceTiers"`
	AdditionalTiers []string `json:"additionalSpeedTiers"`
	Efforts         []Effort `json:"supportedReasoningEfforts"`
	DefaultEffort   string   `json:"defaultReasoningEffort"`
}
type Catalog struct {
	Models                []Model
	Config                map[string]any
	Features              map[string]bool
	Requirements          map[string]bool
	IndependentSpeedModes bool
}

func (c *Client) Catalog(ctx context.Context, cwd string) (Catalog, error) {
	catalog := Catalog{Features: map[string]bool{}, Requirements: map[string]bool{}}
	cursor := ""
	for {
		var page struct {
			Data []Model `json:"data"`
			Next *string `json:"nextCursor"`
		}
		params := map[string]any{"limit": 100}
		if cursor != "" {
			params["cursor"] = cursor
		}
		if e := c.Call(ctx, "model/list", params, &page); e != nil {
			return catalog, fmt.Errorf("model catalog: %w", e)
		}
		catalog.Models = append(catalog.Models, page.Data...)
		if page.Next == nil || *page.Next == "" {
			break
		}
		if *page.Next == cursor {
			return catalog, errors.New("Codex returned a repeated model catalog cursor")
		}
		cursor = *page.Next
	}
	var conf struct {
		Config map[string]any `json:"config"`
	}
	if e := c.Call(ctx, "config/read", map[string]any{"includeLayers": false, "cwd": cwd}, &conf); e != nil {
		return catalog, e
	}
	catalog.Config = conf.Config
	if f, ok := conf.Config["features"].(map[string]any); ok {
		for k, v := range f {
			if b, ok := v.(bool); ok {
				catalog.Features[k] = b
			}
		}
	}
	var req struct {
		Requirements *struct {
			Features map[string]bool `json:"featureRequirements"`
		} `json:"requirements"`
	}
	if e := c.Call(ctx, "configRequirements/read", map[string]any{}, &req); e == nil && req.Requirements != nil {
		catalog.Requirements = req.Requirements.Features
	}
	// 0.162 split ultrafast's flag from fast_mode. Still obey managed requirements.
	catalog.IndependentSpeedModes = true
	return catalog, nil
}
func (c Catalog) Speeds(model Model) []Tier {
	tiers := []Tier{{ID: "default", Name: "Standard", Description: "Standard inference speed"}}
	candidates := model.Tiers
	if len(candidates) == 0 {
		for _, s := range model.AdditionalTiers {
			candidates = append(candidates, Tier{ID: s, Name: map[string]string{"priority": "Fast", "fast": "Fast", "ultrafast": "Ultrafast", "flex": "Flex"}[s]})
		}
	}
	enabled := func(flag string) bool {
		v, ok := c.Features[flag]
		if ok && !v {
			return false
		}
		v, ok = c.Requirements[flag]
		return !ok || v
	}
	for _, t := range candidates {
		if t.ID == "default" {
			continue
		}
		if t.ID == "ultrafast" {
			if !enabled("ultrafast_mode") || (!c.IndependentSpeedModes && !enabled("fast_mode")) {
				continue
			}
		} else if t.ID != "flex" && !enabled("fast_mode") {
			continue
		}
		if t.Name == "" {
			t.Name = t.ID
		}
		tiers = append(tiers, t)
	}
	return tiers
}
func (c Catalog) Find(slug string) (Model, bool) {
	for _, m := range c.Models {
		if m.Model == slug || m.ID == slug {
			return m, true
		}
	}
	return Model{}, false
}
func (c Catalog) DefaultModel() string {
	if s, ok := c.Config["model"].(string); ok && s != "" {
		return s
	}
	for _, m := range c.Models {
		if m.Default {
			return m.Model
		}
	}
	if len(c.Models) > 0 {
		return c.Models[0].Model
	}
	return ""
}
func (c Catalog) SpeedWarning(slug, tier string) string {
	m, ok := c.Find(slug)
	if !ok {
		return "The installed Codex CLI does not advertise this model. Refresh the catalog or update Codex."
	}
	for _, t := range c.Speeds(m) {
		if t.ID == tier {
			return ""
		}
	}
	return "This Codex CLI/provider does not advertise " + tier + " for " + slug + ". Update Codex and check its provider configuration, or choose Standard."
}
func TextInput(text string) []map[string]any {
	return []map[string]any{{"type": "text", "text": text, "text_elements": []any{}}}
}
func Decode(data json.RawMessage) map[string]any {
	var v map[string]any
	d := json.NewDecoder(bytes.NewReader(data))
	_ = d.Decode(&v)
	return v
}
