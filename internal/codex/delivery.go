package codex

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"time"
	"unicode/utf8"
)

const (
	MaxSendsPerTurn     = 5
	MaxPendingPerTarget = 10
	MaxUnattendedHops   = 3
	DeliveryTTL         = time.Hour
)

type DeliveryStart struct {
	ID, From, Target, Text, TurnID, CallID string
	Wait                                   bool
}
type DeliveryRequest struct {
	ID              string
	Revision        uint64
	From, To        OpenThread
	Text, AlwaysAsk string
	Wait            bool
	Hop             int
}
type DeliveryAnswer struct {
	ID, Choice string
	Revision   uint64
}
type DeliveryResult struct {
	ID, Error, Wrapped string
	Success            bool
	Session            bool
	Request            DeliveryRequest
}
type deliveryTurn struct{ Thread, Turn string }
type deliveryCount struct {
	Count   int
	Expires time.Time
}
type deliveryPair struct{ From, To string }
type deliveryPermissionPair struct {
	From, To       PermissionScope
	FromCwd, ToCwd string
}
type deliveryHop struct {
	Turn    string
	Hop     int
	Expires time.Time
}
type pendingDelivery struct {
	request                         DeliveryRequest
	source, target                  *peer
	client                          *Client
	ctx                             context.Context
	cancel                          context.CancelFunc
	timer                           *time.Timer
	wrapped                         string
	sourceTurn, callID              string
	submitting, delivered, consumed bool
}

func permissionPair(from, to OpenThread) deliveryPermissionPair {
	return deliveryPermissionPair{from.Permissions, to.Permissions, from.Cwd, to.Cwd}
}

// Caller holds b.mu. A registry row alone cannot override a known owner.
func (b *Broker) deliveryThread(id string) (OpenThread, *peer) {
	if owner := b.owners[id]; owner != nil {
		for _, row := range owner.threads {
			if row.ID == id {
				return row, owner
			}
		}
		return OpenThread{}, nil
	}
	for _, owner := range b.peers {
		for _, row := range owner.threads {
			if row.ID == id {
				return row, owner
			}
		}
	}
	return OpenThread{}, nil
}
func deliveryReason(from, to OpenThread, hop int) string {
	reason := DeliveryEscalation(from, to)
	if hop > MaxUnattendedHops {
		if reason != "" {
			reason += "; "
		}
		reason += fmt.Sprintf("this message is hop %d of an agent chain; chains longer than %d always ask", hop, MaxUnattendedHops)
	}
	return reason
}
func (b *Broker) requestDelivery(p *peer, input json.RawMessage) error {
	var in DeliveryStart
	if json.Unmarshal(input, &in) != nil || in.ID == "" || len(in.ID) > 128 || in.From == "" || in.Target == "" || in.TurnID == "" || !utf8.ValidString(in.Text) || len(in.Text) == 0 || len(in.Text) > 8192 {
		return errors.New("invalid cross-tab message; include a source turn and 1–8192 UTF-8 bytes")
	}
	b.mu.Lock()
	if b.deliveries == nil {
		b.deliveries = map[string]*pendingDelivery{}
		b.deliverySends = map[deliveryTurn]deliveryCount{}
		b.deliveryGrants = map[deliveryPair]deliveryPermissionPair{}
		b.deliveryHops = map[string]deliveryHop{}
	}
	if b.deliveries[in.ID] != nil || len(b.deliveries) >= 1024 || b.client == nil || b.closed {
		b.mu.Unlock()
		return errors.New("message request is duplicate, disconnected, or the delivery queue is full")
	}
	from, owner := b.deliveryThread(in.From)
	if owner != p || !from.AcceptsMessages {
		b.mu.Unlock()
		return errors.New("the sending conversation is closed or has messaging turned off")
	}
	to, target := b.deliveryThread(in.Target)
	if target == nil {
		seen := map[string]bool{}
		for _, window := range b.peers {
			for _, row := range window.threads {
				if row.Title == in.Target {
					seen[row.ID] = true
				}
			}
		}
		if len(seen) == 1 {
			for id := range seen {
				to, target = b.deliveryThread(id)
			}
		}
		if len(seen) > 1 {
			b.mu.Unlock()
			return errors.New("ambiguous title; use the thread ID")
		}
	}
	if target == nil || !to.AcceptsMessages || to.ID == from.ID {
		b.mu.Unlock()
		return errors.New("target is closed, is the sending tab, or has messaging turned off")
	}
	now := time.Now()
	for key, value := range b.deliverySends {
		if now.After(value.Expires) {
			delete(b.deliverySends, key)
		}
	}
	for key, value := range b.deliveryHops {
		if now.After(value.Expires) {
			delete(b.deliveryHops, key)
		}
	}
	key := deliveryTurn{from.ID, in.TurnID}
	count := b.deliverySends[key]
	if count.Count >= MaxSendsPerTurn || len(b.deliverySends) >= 4096 && count.Count == 0 {
		b.mu.Unlock()
		return errors.New("this turn has reached the cross-tab send limit of 5")
	}
	count.Count++
	count.Expires = now.Add(DeliveryTTL)
	b.deliverySends[key] = count
	waiting := 0
	for _, d := range b.deliveries {
		if d.request.To.ID == to.ID {
			waiting++
		}
	}
	if waiting >= MaxPendingPerTarget {
		b.mu.Unlock()
		return errors.New("the target already has 10 messages awaiting consent or consumption")
	}
	hop := 1
	if previous := b.deliveryHops[from.ID]; previous.Turn == in.TurnID {
		hop = previous.Hop + 1
	}
	request := DeliveryRequest{ID: in.ID, Revision: 1, From: from, To: to, Text: in.Text, Wait: in.Wait, Hop: hop, AlwaysAsk: deliveryReason(from, to, hop)}
	ctx, cancel := context.WithCancel(p.ctx)
	d := &pendingDelivery{request: request, source: p, target: target, client: b.client, ctx: ctx, cancel: cancel, wrapped: WrapAgentMessage(from.ID, from.Title, in.Text, in.Wait, in.ID), sourceTurn: in.TurnID, callID: in.CallID}
	b.deliveries[in.ID] = d
	d.timer = time.AfterFunc(DeliveryTTL, func() { b.expireDelivery(in.ID) })
	grant, allowed := b.deliveryGrants[deliveryPair{from.ID, to.ID}]
	auto := allowed && grant == permissionPair(from, to) && request.AlwaysAsk == ""
	// These are bounded in-memory outbox writes. Keep card publication ordered
	// with cancellation so a late request cannot follow its resolved notice.
	pendingErr := p.send(Message{Method: "fastrock/deliveryPending", Params: raw(request)})
	var targetErr error
	if pendingErr == nil && !auto {
		targetErr = target.send(Message{Method: "fastrock/deliveryRequest", Params: raw(request)})
	}
	b.mu.Unlock()
	if pendingErr != nil {
		b.completeDelivery(d, false, "the sending window disconnected", false)
		return nil
	}
	if auto {
		if err := b.answerDelivery(target, DeliveryAnswer{ID: in.ID, Revision: request.Revision, Choice: "deliver"}); err != nil {
			b.cancelDeliveryRequest(d, err.Error())
		}
		return nil
	}
	if targetErr != nil {
		b.completeDelivery(d, false, "the recipient window disconnected before consent", false)
	}
	return nil
}

func (b *Broker) cancelDelivery(p *peer, id string) {
	b.mu.Lock()
	d := b.deliveries[id]
	allowed := d != nil && (d.source == p || d.target == p) && !d.delivered
	if allowed {
		d.cancel()
	}
	b.mu.Unlock()
	if allowed {
		b.cancelDeliveryRequest(d, "message delivery was cancelled")
	}
}
func (b *Broker) cancelDeliveryRequest(d *pendingDelivery, reason string) {
	b.mu.Lock()
	uncertain := d.submitting
	d.cancel()
	b.mu.Unlock()
	if uncertain {
		reason += ". Delivery may already be queued; check the target history before resending."
	}
	b.completeDelivery(d, false, reason, false)
}
func (b *Broker) expireDelivery(id string) {
	b.mu.Lock()
	d := b.deliveries[id]
	if d != nil && d.delivered {
		delete(b.deliveries, id)
		d.cancel()
		b.mu.Unlock()
		return
	}
	b.mu.Unlock()
	if d != nil {
		b.cancelDeliveryRequest(d, "cross-tab message expired after one hour")
	}
}

func (b *Broker) answerDelivery(p *peer, answer DeliveryAnswer) error {
	b.mu.Lock()
	d := b.deliveries[answer.ID]
	if d == nil || d.delivered || d.submitting || d.target != p || d.request.Revision != answer.Revision {
		b.mu.Unlock()
		return errors.New("this delivery is no longer pending in this window")
	}
	from, source := b.deliveryThread(d.request.From.ID)
	to, target := b.deliveryThread(d.request.To.ID)
	if source != d.source || target != d.target || !from.AcceptsMessages || !to.AcceptsMessages || d.client != b.client {
		b.mu.Unlock()
		b.completeDelivery(d, false, "a conversation closed, moved, disconnected, or turned off messaging", false)
		return nil
	}
	if permissionPair(from, to) != permissionPair(d.request.From, d.request.To) {
		b.mu.Unlock()
		b.refreshDeliveries()
		return errors.New("permissions changed; review the updated delivery card")
	}
	if answer.Choice == "decline" {
		b.mu.Unlock()
		b.completeDelivery(d, false, "the user declined delivery", false)
		return nil
	}
	if answer.Choice != "deliver" && answer.Choice != "session" || answer.Choice == "session" && d.request.AlwaysAsk != "" {
		b.mu.Unlock()
		return errors.New("that delivery choice is not available")
	}
	if answer.Choice == "session" && len(b.deliveryGrants) >= 4096 {
		b.mu.Unlock()
		return errors.New("the session allowance limit was reached; choose Deliver for this message")
	}
	d.submitting = true
	b.mu.Unlock()
	ctx, cancel := context.WithTimeout(d.ctx, 30*time.Second)
	defer cancel()
	err := d.client.Call(ctx, "thread/queue/add", map[string]any{"threadId": to.ID, "input": TextInput(d.wrapped), "clientUserMessageId": d.request.ID}, nil)
	if err != nil {
		b.completeDelivery(d, false, "delivery could not be confirmed: "+err.Error()+". Check the target history before resending.", false)
		return nil
	}
	b.completeDelivery(d, true, "", answer.Choice == "session")
	if to.Status != "running" && to.Status != "inProgress" {
		// The acknowledged queue entry remains recoverable if starting fails.
		_ = d.client.Call(ctx, "thread/queue/start", map[string]any{"threadId": to.ID}, nil)
	}
	return nil
}
func (b *Broker) completeDelivery(d *pendingDelivery, success bool, reason string, grant bool) {
	b.mu.Lock()
	if b.deliveries[d.request.ID] != d || d.delivered {
		b.mu.Unlock()
		return
	}
	d.submitting = false
	d.delivered = success
	request := d.request
	if success && grant && request.AlwaysAsk == "" && len(b.deliveryGrants) < 4096 {
		b.deliveryGrants[deliveryPair{request.From.ID, request.To.ID}] = permissionPair(request.From, request.To)
	}
	if !success || d.consumed {
		delete(b.deliveries, request.ID)
		d.timer.Stop()
		if !success {
			d.cancel()
		}
	}
	b.mu.Unlock()
	result := DeliveryResult{ID: request.ID, Error: reason, Success: success, Session: success && grant, Request: request, Wrapped: d.wrapped}
	_ = d.source.send(Message{Method: "fastrock/deliveryResult", Params: raw(result)})
	_ = d.target.send(Message{Method: "fastrock/deliveryResolved", Params: raw(result)})
}

// Recheck registry changes before a pending card can grant broader permissions.
func (b *Broker) refreshDeliveries() {
	var cancel []*pendingDelivery
	b.mu.Lock()
	for pair, scope := range b.deliveryGrants {
		from, source := b.deliveryThread(pair.From)
		to, target := b.deliveryThread(pair.To)
		if source == nil || target == nil || !from.AcceptsMessages || !to.AcceptsMessages || permissionPair(from, to) != scope {
			delete(b.deliveryGrants, pair)
		}
	}
	for _, d := range b.deliveries {
		if d.delivered {
			continue
		}
		from, source := b.deliveryThread(d.request.From.ID)
		to, target := b.deliveryThread(d.request.To.ID)
		if source != d.source || target != d.target || !from.AcceptsMessages || !to.AcceptsMessages || d.client != b.client {
			cancel = append(cancel, d)
			d.cancel()
			continue
		}
		if permissionPair(from, to) != permissionPair(d.request.From, d.request.To) {
			if d.submitting {
				d.cancel()
				continue
			}
			d.request.From, d.request.To = from, to
			d.request.Revision++
			d.request.AlwaysAsk = deliveryReason(from, to, d.request.Hop)
			if target.send(Message{Method: "fastrock/deliveryRequest", Params: raw(d.request)}) != nil {
				cancel = append(cancel, d)
			}
		}
	}
	b.mu.Unlock()
	for _, d := range cancel {
		b.cancelDeliveryRequest(d, "a conversation closed, moved, disconnected, or turned off messaging")
	}
}
func (b *Broker) deliveryEvent(m Message) {
	var p struct {
		ThreadID, TurnID string
		RequestID        json.RawMessage
		Turn             struct{ ID string }
		Item             struct {
			Type    string
			Content []struct{ Text string }
		}
	}
	if json.Unmarshal(m.Params, &p) != nil {
		return
	}
	if m.Method == "thread/settings/updated" {
		// Invalidate allowances before this settings event reaches any window.
		// The owner will publish its newly decoded effective scope afterward.
		b.mu.Lock()
		for _, peer := range b.peers {
			for i := range peer.threads {
				if peer.threads[i].ID == p.ThreadID {
					peer.threads[i].Permissions = PermissionScope{}
				}
			}
		}
		b.mu.Unlock()
		b.refreshDeliveries()
		return
	}
	if m.Method == "turn/completed" || m.Method == "serverRequest/resolved" {
		var cancelled []*pendingDelivery
		b.mu.Lock()
		for _, d := range b.deliveries {
			if d.delivered {
				continue
			}
			turnEnded := m.Method == "turn/completed" && d.request.From.ID == p.ThreadID && d.sourceTurn == p.Turn.ID
			resolved := m.Method == "serverRequest/resolved" && d.callID != "" && d.callID == string(p.RequestID)
			if turnEnded || resolved {
				cancelled = append(cancelled, d)
			}
		}
		b.mu.Unlock()
		for _, d := range cancelled {
			b.cancelDeliveryRequest(d, "the sending request ended before delivery")
		}
	}
	b.mu.Lock()
	defer b.mu.Unlock()
	if m.Method == "turn/completed" {
		delete(b.deliverySends, deliveryTurn{p.ThreadID, p.Turn.ID})
		if h := b.deliveryHops[p.ThreadID]; h.Turn == p.Turn.ID {
			delete(b.deliveryHops, p.ThreadID)
		}
	}
	if (m.Method == "item/started" || m.Method == "item/completed") && p.Item.Type == "userMessage" {
		for _, text := range p.Item.Content {
			for id, d := range b.deliveries {
				if d.request.To.ID == p.ThreadID && d.wrapped == text.Text && p.TurnID != "" {
					d.consumed = true
					b.deliveryHops[p.ThreadID] = deliveryHop{Turn: p.TurnID, Hop: d.request.Hop, Expires: time.Now().Add(DeliveryTTL)}
					if d.delivered {
						delete(b.deliveries, id)
						d.timer.Stop()
						d.cancel()
					}
				}
			}
		}
	}
}
func (b *Broker) clearDeliveries(reason string) {
	b.mu.Lock()
	var pending []*pendingDelivery
	for _, d := range b.deliveries {
		d.cancel()
		d.timer.Stop()
		if !d.delivered {
			pending = append(pending, d)
		}
	}
	clear(b.deliveryGrants)
	clear(b.deliverySends)
	clear(b.deliveryHops)
	b.mu.Unlock()
	for _, d := range pending {
		b.cancelDeliveryRequest(d, reason)
	}
	b.mu.Lock()
	clear(b.deliveries)
	b.mu.Unlock()
}

func deliveryChoice(input json.RawMessage) (DeliveryAnswer, error) {
	var answer DeliveryAnswer
	if json.Unmarshal(input, &answer) != nil || strings.TrimSpace(answer.ID) == "" {
		return answer, errors.New("invalid delivery answer")
	}
	return answer, nil
}
