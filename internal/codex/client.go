// Package codex speaks newline-delimited JSON-RPC to the installed Codex CLI.
package codex

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"github.com/allquixotic/fastrock/internal/buildinfo"
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
	Origin   *Client         `json:"-"`
	Sequence uint64          `json:"fastrockSequence,omitempty"`
	ID       json.RawMessage `json:"id,omitempty"`
	Method   string          `json:"method,omitempty"`
	Params   json.RawMessage `json:"params,omitempty"`
	Result   json.RawMessage `json:"result,omitempty"`
	Error    *RPCError       `json:"error,omitempty"`
}
type Client struct {
	cmd            *exec.Cmd
	input          io.WriteCloser
	ctx            context.Context
	cancel         context.CancelFunc
	seq            atomic.Uint64
	mu             sync.Mutex
	writer         chan struct{}
	writerOnce     sync.Once
	broker         bool
	pending        map[string]chan Message
	Events         chan Message
	done           chan struct{}
	closeOnce      sync.Once
	Version        string
	stderr         tailBuffer
	cleanupProcess func()
}

var versionPattern = regexp.MustCompile(`(?:codex-cli\s+)?(\d+)\.(\d+)\.(\d+)([^\s]*)`)

func CheckVersion(output string) error {
	return checkMinimumVersion(output, SupportedVersion)
}

func checkMinimumVersion(output, minimum string) error {
	m := versionPattern.FindStringSubmatch(output)
	if m == nil {
		return errors.New("cannot determine Codex CLI version; install the latest Codex CLI")
	}
	want := strings.Split(minimum, ".")
	if len(want) != 3 {
		return fmt.Errorf("invalid supported Codex version %q", minimum)
	}
	comparison := 0
	for i := range 3 {
		got, err := strconv.ParseUint(m[i+1], 10, 64)
		if err != nil {
			return errors.New("cannot determine Codex CLI version; install the latest Codex CLI")
		}
		required, err := strconv.ParseUint(want[i], 10, 64)
		if err != nil {
			return fmt.Errorf("invalid supported Codex version %q", minimum)
		}
		if comparison == 0 {
			if got < required {
				comparison = -1
			} else if got > required {
				comparison = 1
			}
		}
	}
	if comparison < 0 {
		return fmt.Errorf("Codex %s is too old; Fastrock supports Codex %s or newer. Update the installed CLI", strings.Join(m[1:4], "."), minimum)
	}
	return nil
}

// Start never downloads Codex or changes CODEX_HOME/config.toml.
func Start(ctx context.Context) (*Client, error) {
	return StartWithStartup(ctx, ctx)
}

// StartWithStartup keeps the shared process alive independently of the window
// that initiates it, while letting that window cancel a probe or initialization.
func StartWithStartup(parent, startup context.Context) (*Client, error) {
	if err := startup.Err(); err != nil {
		return nil, err
	}
	binary, e := lookPath()
	if e != nil {
		return nil, errors.New("Codex CLI was not found on PATH. Install the latest Codex CLI, then retry")
	}
	probeCtx, cancel := context.WithTimeout(startup, 10*time.Second)
	defer cancel()
	probe := command(probeCtx, binary, "--version")
	out, e := probe.Output()
	if e != nil {
		return nil, fmt.Errorf("cannot run installed Codex CLI: %w", e)
	}
	if e = CheckVersion(string(out)); e != nil {
		return nil, e
	}
	if err := startup.Err(); err != nil {
		return nil, err
	}
	c, e := StartCommand(parent, binary, []string{"app-server"})
	if e != nil {
		return nil, e
	}
	c.Version = strings.TrimSpace(string(out))
	initCtx, stop := context.WithTimeout(startup, 30*time.Second)
	defer stop()
	if e = c.initialize(initCtx); e != nil {
		return nil, e
	}
	return c, nil
}

// Initialization can fail because of local state, permissions or transport
// errors even when the installed CLI passed the supported-version check.
func (c *Client) initialize(ctx context.Context) error {
	var result map[string]any
	e := c.Call(ctx, "initialize", map[string]any{"clientInfo": map[string]any{"name": "fastrock", "title": "Fastrock", "version": buildinfo.Version}, "capabilities": map[string]any{"experimentalApi": true}}, &result)
	if e == nil {
		e = c.Notify("initialized", map[string]any{})
	}
	if e != nil {
		c.Close()
		return fmt.Errorf("Codex app-server initialization failed: %w", e)
	}
	return nil
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
	c.cleanupProcess, e = containProcess(cmd)
	if e != nil {
		cancel()
		_ = cmd.Process.Kill()
		_ = cmd.Wait()
		return nil, fmt.Errorf("contain Codex process tree: %w", e)
	}
	go func() { _, _ = io.Copy(&c.stderr, stderr) }()
	go c.read(output)
	return c, nil
}
func (c *Client) read(output io.Reader) {
	defer func() {
		if c.cleanupProcess != nil {
			c.cleanupProcess()
		}
	}()
	defer close(c.done)
	defer close(c.Events)
	defer c.cancel()
	reader := bufio.NewReaderSize(output, 64<<10)
	var readErr error
readLoop:
	for {
		frame, header, err := readFrameHeader(reader, maxRPCFrame)
		if errors.Is(err, errFrameTooLarge) {
			c.mu.Lock()
			if ch := c.pending[string(header.ID)]; ch != nil {
				ch <- Message{Error: &RPCError{Code: -32000, Message: err.Error()}}
				delete(c.pending, string(header.ID))
			}
			c.mu.Unlock()
			if header.Method != "" && len(header.ID) > 0 {
				_ = c.Reject(header.ID, err.Error())
			}
			select {
			case c.Events <- Message{Method: "fastrock/frameError", Params: json.RawMessage(`{"message":"Codex returned a message exceeding 32 MiB. The operation outcome may be uncertain; narrow or reload the conversation."}`)}:
			case <-c.ctx.Done():
				break readLoop
			}
			continue
		}
		if err != nil {
			readErr = err
			break
		}
		var m Message
		if e := json.Unmarshal(frame, &m); e != nil {
			readErr = fmt.Errorf("invalid JSON from Codex app-server: %w", e)
			break
		}
		if m.Method != "" {
			select {
			case c.Events <- m:
			case <-c.ctx.Done():
				break readLoop
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
		readErr = io.EOF
	}
	c.mu.Lock()
	for id, ch := range c.pending {
		ch <- Message{Error: &RPCError{Code: CodeDisconnected, Message: "app-server disconnected: " + readErr.Error() + c.stderr.message()}}
		delete(c.pending, id)
	}
	c.mu.Unlock()
	_ = c.input.Close()
	if c.cmd != nil {
		// EOF is normally the server's graceful shutdown. Allow it to reap
		// its children before cancelling CommandContext.
		_ = c.cmd.Wait()
	}
}
func (c *Client) write(v any) error { return c.writeContext(c.ctx, v) }
func (c *Client) writeContext(ctx context.Context, v any) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	data, err := json.Marshal(v)
	if err != nil {
		return err
	}
	c.writerOnce.Do(func() { c.writer = make(chan struct{}, 1) })
	select {
	case <-ctx.Done():
		return ctx.Err()
	case c.writer <- struct{}{}:
	}
	defer func() { <-c.writer }()
	if err := ctx.Err(); err != nil {
		return err
	}
	if socket, ok := c.input.(interface{ SetWriteDeadline(time.Time) error }); ok {
		deadline := time.Now().Add(5 * time.Second)
		if d, ok := ctx.Deadline(); ok && d.Before(deadline) {
			deadline = d
		}
		_ = socket.SetWriteDeadline(deadline)
	}
	_, err = c.input.Write(append(data, '\n'))
	return err
}
func (c *Client) Call(ctx context.Context, method string, params, out any) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	id := c.seq.Add(1)
	key := strconv.FormatUint(id, 10)
	ch := make(chan Message, 1)
	c.mu.Lock()
	c.pending[key] = ch
	c.mu.Unlock()
	defer func() { c.mu.Lock(); delete(c.pending, key); c.mu.Unlock() }()
	if e := c.writeContext(ctx, map[string]any{"id": id, "method": method, "params": params}); e != nil {
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
		if c.broker {
			go func() { _ = c.Notify("fastrock/cancelRequest", map[string]string{"id": key}) }()
		}
		return ctx.Err()
	case <-c.ctx.Done():
		return ErrServerStopped
	}
}
func (c *Client) Notify(method string, params any) error {
	return c.write(map[string]any{"method": method, "params": params})
}
func (c *Client) Respond(id json.RawMessage, result any) error {
	if c.broker {
		ctx, cancel := context.WithTimeout(c.ctx, 10*time.Second)
		defer cancel()
		return c.Call(ctx, "fastrock/respond", Message{ID: id, Result: raw(result)}, nil)
	}
	return c.write(map[string]any{"id": id, "result": result})
}
func (c *Client) Reject(id json.RawMessage, message string) error {
	return c.RejectCode(id, CodeRequestRejected, message)
}
func (c *Client) RejectCode(id json.RawMessage, code int, message string) error {
	if c.broker {
		ctx, cancel := context.WithTimeout(c.ctx, 10*time.Second)
		defer cancel()
		return c.Call(ctx, "fastrock/respond", Message{ID: id, Error: &RPCError{Code: code, Message: message}}, nil)
	}
	return c.write(map[string]any{"id": id, "error": RPCError{Code: code, Message: message}})
}
func (c *Client) Close() {
	c.closeOnce.Do(func() {
		_ = c.input.Close()
		select {
		case <-c.done:
		case <-time.After(3 * time.Second):
			c.cancel()
			if c.cmd != nil && c.cmd.Process != nil {
				_ = c.cmd.Process.Kill()
			}
		}
		c.cancel()
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
	Policy                ConfigRequirements
	PolicyLoaded          bool
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
		Requirements *ConfigRequirements `json:"requirements"`
	}
	if e := c.Call(ctx, "configRequirements/read", map[string]any{}, &req); e != nil {
		return catalog, fmt.Errorf("configuration requirements: %w", e)
	} else if req.Requirements != nil {
		catalog.Policy = *req.Requirements
		catalog.Requirements = req.Requirements.Features
	}
	catalog.PolicyLoaded = true
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
	_ = json.Unmarshal(data, &v)
	return v
}
