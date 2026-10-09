package ui

import (
	"context"
	"encoding/json"
	"strings"
	"sync"

	"github.com/allquixotic/fastrock/internal/codex"
)

type decodedEvent struct {
	message      codex.Message
	params       map[string]any
	client       *codex.Client
	disconnected bool
	delta        strings.Builder
	bytes        int
}

// A separate bounded inbox keeps streaming traffic out of the UI callback
// queue. Adjacent deltas coalesce, but requests and lifecycle events retain
// their exact order. Backpressure applies only at the explicit memory bound.
type eventInbox struct {
	mu     sync.Mutex
	events []*decodedEvent
	bytes  int
	space  chan struct{}
}

func deltaEvent(m codex.Message) bool {
	if len(m.ID) != 0 {
		return false
	}
	switch m.Method {
	case "item/agentMessage/delta", "item/commandExecution/outputDelta", "item/reasoning/summaryTextDelta", "item/reasoning/textDelta":
		return true
	}
	return false
}

func (q *eventInbox) push(ctx context.Context, event decodedEvent) bool {
	for {
		q.mu.Lock()
		if q.space == nil {
			q.space = make(chan struct{}, 1)
		}
		if deltaEvent(event.message) && len(q.events) > 0 {
			last := q.events[len(q.events)-1]
			length := last.delta.Len()
			if length == 0 {
				length = len(str(last.params, "delta"))
			}
			incoming := len(str(event.params, "delta"))
			if last.client == event.client && last.message.Method == event.message.Method && str(last.params, "threadId") == str(event.params, "threadId") && str(last.params, "itemId") == str(event.params, "itemId") && integer(last.params, "summaryIndex") == integer(event.params, "summaryIndex") && integer(last.params, "contentIndex") == integer(event.params, "contentIndex") && length+incoming <= 64<<10 && q.bytes+incoming <= 4<<20 {
				if last.delta.Len() == 0 {
					last.delta.WriteString(str(last.params, "delta"))
				}
				delta := str(event.params, "delta")
				last.delta.WriteString(delta)
				last.bytes += len(delta)
				q.bytes += len(delta)
				last.message.Sequence = event.message.Sequence
				q.mu.Unlock()
				return true
			}
		}
		if len(q.events) < 256 && (q.bytes+len(event.message.Params) <= 4<<20 || len(q.events) == 0) {
			event.bytes = len(event.message.Params)
			q.bytes += event.bytes
			q.events = append(q.events, &event)
			q.mu.Unlock()
			return true
		}
		space := q.space
		q.mu.Unlock()
		select {
		case <-space:
		case <-ctx.Done():
			return false
		}
	}
}

func (q *eventInbox) take() *decodedEvent {
	q.mu.Lock()
	defer q.mu.Unlock()
	if len(q.events) == 0 {
		return nil
	}
	e := q.events[0]
	q.events[0] = nil
	q.events = q.events[1:]
	q.bytes -= e.bytes
	if e.delta.Len() > 0 {
		e.params["delta"] = e.delta.String()
		// Transfer replay consumes raw Params; keep its representation consistent.
		e.message.Params, _ = json.Marshal(e.params)
	}
	select {
	case q.space <- struct{}{}:
	default:
	}
	return e
}

func (a *App) consumeEvent(client *codex.Client, message codex.Message) bool {
	message.Origin = client
	if !a.events.push(a.ctx, decodedEvent{message: message, params: codex.Decode(message.Params), client: client}) {
		return false
	}
	if a.window != nil {
		a.window.Changed()
	}
	return true
}
