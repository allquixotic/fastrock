package ui

import (
	"context"
	"errors"
	"fmt"
	"time"

	"github.com/aarzilli/nucular"
	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/workspace"
)

const recapTimeout = 30 * time.Second

type recapRun struct {
	ctx        context.Context
	cancel     context.CancelFunc
	client     *codex.Client
	generation uint64
	threadID   string // UI-owned; registered before turn/start can emit events.
	events     chan map[string]any
}

func (a *App) startRecap(c *workspace.Conversation) {
	if a.client == nil || a.serverPaused {
		a.toast = "Reconnect to Codex before generating a recap"
		return
	}
	if a.recaps[c.ID] != nil {
		a.toast = "A recap is already being generated"
		return
	}
	if len(a.recaps) >= 4 {
		a.toast = "Wait for a running recap to finish"
		return
	}
	history := recapHistory(c.Blocks)
	if history == "" {
		a.toast = "There is no conversation to recap yet"
		return
	}
	if a.recaps == nil {
		a.recaps = map[string]*recapRun{}
	}
	if a.recapCreates == nil {
		a.recapCreates = make(chan struct{}, 4)
	}
	ctx, cancel := context.WithCancel(a.ctx)
	r := &recapRun{ctx: ctx, cancel: cancel, client: a.client, generation: a.serverGeneration, events: make(chan map[string]any, 32)}
	a.recaps[c.ID] = r
	a.toast = "Generating a recap…"
	// Snapshot settings on the UI owner; background work never reads live chat state.
	options := &workspace.Conversation{Cwd: c.Cwd, Model: c.Model, Settings: c.Settings}
	lifetime, slots := a.ctx, a.recapCreates
	a.work(func() {
		result, err := generateRecap(r, lifetime, slots, options, history, recapTimeout, func(id string) bool {
			attached := make(chan bool, 1)
			a.post(func() {
				ok := a.recaps[c.ID] == r && a.state.Chats[c.ID] == c && a.client == r.client && a.serverGeneration == r.generation && r.ctx.Err() == nil
				if ok {
					r.threadID = id
				}
				attached <- ok
			})
			select {
			case ok := <-attached:
				return ok
			case <-r.ctx.Done():
				return false
			}
		})
		a.post(func() {
			if a.recaps[c.ID] != r {
				return
			}
			delete(a.recaps, c.ID)
			defer cancel()
			if a.state.Chats[c.ID] != c || a.client != r.client || a.serverGeneration != r.generation || r.ctx.Err() != nil {
				return
			}
			if err != nil {
				c.Append(workspace.NewID("recap-error"), "notice", "notice", "Could not generate a recap: "+err.Error()+". Please try again.")
				return
			}
			c.Append(workspace.NewID("recap"), "recap", "recap", result.markdown())
		})
	}, func() {
		cancel()
		if a.recaps[c.ID] == r {
			delete(a.recaps, c.ID)
		}
	})
}

func (a *App) cancelRecap(thread string) {
	if r := a.recaps[thread]; r != nil {
		r.cancel()
		delete(a.recaps, thread)
	}
}

func (a *App) cancelRecaps() {
	for thread := range a.recaps {
		a.cancelRecap(thread)
	}
}

func (a *App) drawRecapStatus(w *nucular.Window, c *workspace.Conversation) {
	if a.recaps[c.ID] == nil {
		return
	}
	w.Row(28).Ratio(.75, .25)
	w.Label("Generating conversation recap…", "LC")
	if w.ButtonText("Cancel recap") {
		a.cancelRecap(c.ID)
	}
}

// Temporary-thread events never reach the visible conversation or approval queue.
func (a *App) recapEvent(m codex.Message, p map[string]any) bool {
	id := str(p, "threadId")
	if id == "" {
		return false
	}
	for _, r := range a.recaps {
		if r.threadID != id || (m.Origin != nil && m.Origin != r.client) {
			continue
		}
		if len(m.ID) > 0 {
			client := r.client
			a.controlWork(func() { _ = client.Reject(m.ID, "Recap threads cannot use tools or ask for input") })
			r.cancel()
			return true
		}
		var event map[string]any
		switch m.Method {
		case "item/completed":
			item, _ := p["item"].(map[string]any)
			if str(item, "type") != "agentMessage" {
				return true
			}
			text := str(item, "text")
			if len(text) > recapAnswerBytes {
				event = map[string]any{"method": "recap/tooLarge"}
			} else {
				event = map[string]any{"method": m.Method, "turnId": str(p, "turnId"), "item": map[string]any{"type": "agentMessage", "text": text}}
			}
		case "turn/completed":
			turn, _ := p["turn"].(map[string]any)
			event = map[string]any{"method": m.Method, "turn": map[string]any{"id": str(turn, "id"), "status": str(turn, "status")}}
		case "error":
			retry, _ := p["willRetry"].(bool)
			event = map[string]any{"method": m.Method, "willRetry": retry}
		}
		if event != nil {
			select {
			case r.events <- event:
			default:
				r.cancel()
			}
		}
		return true
	}
	return false
}

func recapCleanup(client *codex.Client, thread, turn string) {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if turn != "" {
		_ = client.Call(ctx, "turn/interrupt", map[string]any{"threadId": thread, "turnId": turn}, nil)
	}
	_ = client.Call(ctx, "thread/unsubscribe", map[string]any{"threadId": thread}, nil)
}

// Creation RPCs keep listening after a local cancellation/timeout so late-created
// threads/turns can be released. Four slots bound these detached listeners.
func recapCreate(ctx, lifetime context.Context, slots chan struct{}, client *codex.Client, method string, params any, late func(map[string]any)) (map[string]any, error) {
	select {
	case slots <- struct{}{}:
	case <-ctx.Done():
		return nil, ctx.Err()
	}
	type result struct {
		value map[string]any
		err   error
	}
	ready := make(chan result)
	go func() {
		defer func() { <-slots }()
		var value map[string]any
		err := client.Call(lifetime, method, params, &value)
		select {
		case ready <- result{value, err}:
		case <-ctx.Done():
			if err == nil {
				late(value)
			}
		}
	}()
	select {
	case result := <-ready:
		return result.value, result.err
	case <-ctx.Done():
		return nil, ctx.Err()
	}
}

func generateRecap(r *recapRun, lifetime context.Context, slots chan struct{}, options *workspace.Conversation, history string, timeout time.Duration, attach func(string) bool) (generatedRecap, error) {
	var empty generatedRecap
	ctx, cancel := context.WithTimeout(r.ctx, timeout)
	var config struct {
		Config map[string]any `json:"config"`
	}
	err := r.client.Call(ctx, "config/read", map[string]any{"cwd": options.Cwd, "includeLayers": false}, &config)
	cancel()
	if err != nil {
		return empty, err
	}
	if config.Config == nil {
		return empty, errors.New("the effective configuration was unavailable")
	}
	params, err := recapThreadParams(options, config.Config)
	if err != nil {
		return empty, err
	}
	ctx, cancel = context.WithTimeout(r.ctx, timeout)
	started, err := recapCreate(ctx, lifetime, slots, r.client, "thread/start", params, func(late map[string]any) {
		if thread, _ := late["thread"].(map[string]any); str(thread, "id") != "" {
			recapCleanup(r.client, str(thread, "id"), "")
		}
	})
	cancel()
	if err != nil {
		return empty, err
	}
	thread, _ := started["thread"].(map[string]any)
	id := str(thread, "id")
	if id == "" {
		return empty, errors.New("Codex did not create the temporary recap thread")
	}
	turnID := ""
	completed := false
	defer func() {
		if completed {
			turnID = ""
		}
		recapCleanup(r.client, id, turnID)
	}()
	sandbox, _ := started["sandbox"].(map[string]any)
	if str(sandbox, "type") != "read-only" {
		return empty, errors.New("the temporary thread did not start read-only")
	}
	if !attach(id) {
		return empty, context.Canceled
	}
	ctx, cancel = context.WithTimeout(r.ctx, timeout)
	defer cancel()
	started, err = recapCreate(ctx, lifetime, slots, r.client, "turn/start", map[string]any{"threadId": id, "input": codex.TextInput(recapPrompt + history), "outputSchema": recapSchema()}, func(late map[string]any) {
		turn, _ := late["turn"].(map[string]any)
		recapCleanup(r.client, id, str(turn, "id"))
	})
	if err != nil {
		return empty, err
	}
	turn, _ := started["turn"].(map[string]any)
	turnID = str(turn, "id")
	if turnID == "" {
		return empty, errors.New("Codex did not start the recap turn")
	}
	answer := ""
	for {
		select {
		case <-ctx.Done():
			return empty, ctx.Err()
		case event := <-r.events:
			switch str(event, "method") {
			case "recap/tooLarge":
				return empty, errors.New("the recap answer was too large")
			case "item/completed":
				if str(event, "turnId") != turnID {
					continue
				}
				item, _ := event["item"].(map[string]any)
				if str(item, "type") == "agentMessage" {
					answer = str(item, "text")
					if len(answer) > recapAnswerBytes {
						return empty, errors.New("the recap answer was too large")
					}
				}
			case "turn/completed":
				turn, _ := event["turn"].(map[string]any)
				if str(turn, "id") != turnID {
					continue
				}
				completed = true
				if status := str(turn, "status"); status != "completed" {
					return empty, fmt.Errorf("the recap turn ended as %s", status)
				}
				return parseRecap(answer)
			case "error":
				if retry, _ := event["willRetry"].(bool); !retry {
					return empty, errors.New("the recap turn failed")
				}
			}
		}
	}
}
