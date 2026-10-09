package codex

// The renderer has one native window per process. A loopback broker keeps all
// windows on one app-server; credentials are inherited through the environment,
// never put on the command line or written to disk.
import (
	"bufio"
	"context"
	"crypto/rand"
	"crypto/subtle"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"sync"
	"sync/atomic"
	"time"

	"github.com/allquixotic/fastrock/internal/settings"
)

type transfer struct {
	Finalized bool
	Owner     *peer
	Claimed   *peer
	Data      json.RawMessage
	Expires   time.Time
}
type peer struct {
	conn        net.Conn
	out         chan []byte
	ctx         context.Context
	cancel      context.CancelFunc
	calls       sync.Map
	queuedBytes atomic.Int64
	id          string
	memory      uint64
	name        string
	threads     []OpenThread
}

const peerQueueBytes = 16 << 20

func (p *peer) send(m Message) error {
	data, err := json.Marshal(m)
	if err != nil {
		return err
	}
	data = append(data, '\n')
	if p.queuedBytes.Add(int64(len(data))) > peerQueueBytes {
		p.queuedBytes.Add(-int64(len(data)))
		p.cancel()
		p.conn.Close()
		return errors.New("window fell behind; reconnect to resynchronize")
	}
	select {
	case <-p.ctx.Done():
		p.queuedBytes.Add(-int64(len(data)))
		return p.ctx.Err()
	case p.out <- data:
		return nil
	default:
		p.queuedBytes.Add(-int64(len(data)))
		p.cancel()
		p.conn.Close()
		return errors.New("window fell behind; reconnect to resynchronize")
	}
}
func (p *peer) writeLoop() {
	defer p.cancel()
	defer p.conn.Close()
	for {
		select {
		case <-p.ctx.Done():
			return
		case data := <-p.out:
			_ = p.conn.SetWriteDeadline(time.Now().Add(5 * time.Second))
			_, err := p.conn.Write(data)
			p.queuedBytes.Add(-int64(len(data)))
			if err != nil {
				return
			}
		}
	}
}

type Broker struct {
	preferencesMu       sync.Mutex
	preferences         settings.Preferences
	preferencesRevision uint64
	savePreferences     func(settings.Preferences) error
	updateHandler       func(bool) any
	restartMu           sync.Mutex
	starter             func() (*Client, error)
	sequence            atomic.Uint64
	serviceMemory       atomic.Uint64
	client              *Client
	listener            net.Listener
	token               string
	mu                  sync.Mutex
	peers               map[string]*peer
	owners              map[string]*peer
	approvals           map[string]*peer
	tickets             map[string]transfer
	transferReceipts    map[string]transferReceipt
	requests            map[string]Message
	done                chan struct{}
	once                sync.Once
	connected           bool
	closed              bool
	deliveries          map[string]*pendingDelivery
	deliverySends       map[deliveryTurn]deliveryCount
	deliveryGrants      map[deliveryPair]deliveryPermissionPair
	deliveryHops        map[string]deliveryHop
}

func secret() string {
	var b [32]byte
	if _, err := rand.Read(b[:]); err != nil {
		panic(err)
	}
	return hex.EncodeToString(b[:])
}
func NewBroker(c *Client) (*Broker, error) {
	l, err := net.Listen("tcp4", "127.0.0.1:0")
	if err != nil {
		return nil, err
	}
	b := &Broker{client: c, listener: l, token: secret(), peers: map[string]*peer{}, owners: map[string]*peer{}, approvals: map[string]*peer{}, tickets: map[string]transfer{}, requests: map[string]Message{}, done: make(chan struct{})}
	go b.accept()
	go b.events(c)
	return b, nil
}
func (b *Broker) Address() string { return b.listener.Addr().String() }
func (b *Broker) Token() string   { return b.token }
func (b *Broker) Wait()           { <-b.done }
func (b *Broker) Close() {
	b.once.Do(func() {
		b.mu.Lock()
		b.closed = true
		b.mu.Unlock()
		b.clearDeliveries("the shared Codex connection closed")
		b.listener.Close()
		b.mu.Lock()
		b.closed = true
		for _, p := range b.peers {
			p.conn.Close()
		}
		b.mu.Unlock()
		if c := b.currentClient(); c != nil {
			c.Close()
		}
		close(b.done)
	})
}
func (b *Broker) accept() {
	for {
		c, e := b.listener.Accept()
		if e != nil {
			return
		}
		go b.serve(c)
	}
}
func (b *Broker) serve(conn net.Conn) {
	defer conn.Close()
	_ = conn.SetReadDeadline(time.Now().Add(10 * time.Second))
	s := bufio.NewScanner(conn)
	s.Buffer(make([]byte, 64<<10), 32<<20)
	if !s.Scan() {
		return
	}
	var hello Message
	if json.Unmarshal(s.Bytes(), &hello) != nil || hello.Method != "fastrock/hello" {
		return
	}
	var auth struct{ Token string }
	_ = json.Unmarshal(hello.Params, &auth)
	if subtle.ConstantTimeCompare([]byte(auth.Token), []byte(b.token)) != 1 {
		return
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	p := &peer{conn: conn, id: secret(), ctx: ctx, cancel: cancel, out: make(chan []byte, 128)}
	go p.writeLoop()
	b.mu.Lock()
	b.peers[p.id] = p
	b.connected = true
	b.mu.Unlock()
	defer func() {
		b.mu.Lock()
		delete(b.peers, p.id)
		var cancelled []struct {
			owner  *peer
			ticket string
		}
		for key, t := range b.tickets {
			if t.Claimed == p || t.Owner == p {
				delete(b.tickets, key)
				b.recordTransfer(key, t, TransferCancelled)
				target := t.Owner
				if target == p {
					target = t.Claimed
				}
				if target != nil {
					cancelled = append(cancelled, struct {
						owner  *peer
						ticket string
					}{target, key})
				}
			}
		}
		for id, owner := range b.owners {
			if owner == p {
				delete(b.owners, id)
			}
		}
		for id, owner := range b.approvals {
			if owner == p {
				request := b.requests[id]
				delete(b.requests, id)
				if request.Origin != nil {
					go request.Origin.Reject(request.ID, "Owning Fastrock window disconnected before answering")
				}
				delete(b.approvals, id)
			}
		}
		empty := b.connected && len(b.peers) == 0
		b.mu.Unlock()
		b.refreshDeliveries()
		for _, c := range cancelled {
			_ = c.owner.send(Message{Method: "fastrock/tabCancelled", Params: raw(map[string]string{"ticket": c.ticket})})
		}
		if empty {
			go b.Close()
		}
	}()
	_ = p.send(Message{ID: hello.ID, Result: raw(map[string]string{"version": b.version(), "window": p.id})})
	_ = conn.SetReadDeadline(time.Time{})
	// Bound requests per window even when a client is misbehaving.
	slots := make(chan struct{}, 32)
	for s.Scan() {
		var m Message
		if json.Unmarshal(s.Bytes(), &m) != nil {
			return
		}
		if m.Method == "fastrock/cancelRequest" {
			var args struct{ ID string }
			if json.Unmarshal(m.Params, &args) == nil {
				if value, ok := p.calls.Load(args.ID); ok {
					value.(context.CancelFunc)()
				}
			}
			continue
		}
		if m.Method == "" {
			b.mu.Lock()
			owner := b.approvals[string(m.ID)]
			request := b.requests[string(m.ID)]
			if owner == p {
				delete(b.approvals, string(m.ID))
				delete(b.requests, string(m.ID))
			}
			b.mu.Unlock()
			if owner == p {
				if c := request.Origin; c != nil && c == b.currentClient() {
					_ = c.write(m)
				}
			}
			continue
		}
		// Approval delivery has its own synchronous, deadline-bounded path so it
		// remains available when ordinary request slots are occupied.
		if m.Method == "fastrock/respond" {
			work, stop := context.WithTimeout(p.ctx, 10*time.Second)
			b.request(work, p, m)
			stop()
			continue
		}
		select {
		case slots <- struct{}{}:
			work, stop := context.WithTimeout(p.ctx, 2*time.Minute)
			p.calls.Store(string(m.ID), context.CancelFunc(stop))
			go func() { defer func() { <-slots; p.calls.Delete(string(m.ID)); stop() }(); b.request(work, p, m) }()
		default:
			// Never stop reading the socket: approval responses must still pass
			// through when all normal request slots are occupied.
			if len(m.ID) > 0 {
				_ = p.send(Message{ID: m.ID, Error: &RPCError{Code: CodeRequestRejected, Message: "Too many pending requests in this window; try again shortly"}})
			}
		}
	}
}
func raw(v any) json.RawMessage { r, _ := json.Marshal(v); return r }
func (b *Broker) request(requestCtx context.Context, p *peer, m Message) {
	defer func() {
		if err := recover(); err != nil {
			_ = p.send(Message{ID: m.ID, Error: &RPCError{Code: CodeInternalError, Message: fmt.Sprintf("Internal broker error: %v", err)}})
		}
	}()
	var result json.RawMessage
	var err error
	var args struct {
		ThreadID, Ticket, Window, Name string
		Check                          bool
		Bytes                          uint64
		Data                           json.RawMessage
	}
	if len(m.Params) > 0 {
		if err := json.Unmarshal(m.Params, &args); err != nil {
			if len(m.ID) > 0 {
				_ = p.send(Message{ID: m.ID, Error: &RPCError{Code: CodeInvalidParams, Message: "Invalid request parameters"}})
			}
			return
		}
	}
	switch m.Method {
	case "fastrock/requestDelivery":
		err = b.requestDelivery(p, args.Data)
		result = raw(nil)
	case "fastrock/answerDelivery":
		var answer DeliveryAnswer
		answer, err = deliveryChoice(args.Data)
		if err == nil {
			err = b.answerDelivery(p, answer)
		}
		result = raw(nil)
	case "fastrock/cancelDelivery":
		var request struct{ ID string }
		err = json.Unmarshal(args.Data, &request)
		if err == nil {
			b.cancelDelivery(p, request.ID)
		}
		result = raw(nil)
	case "fastrock/respond":
		var response Message
		if err = json.Unmarshal(m.Params, &response); err != nil {
			break
		}
		b.mu.Lock()
		owner, request := b.approvals[string(response.ID)], b.requests[string(response.ID)]
		b.mu.Unlock()
		if owner != p || request.Origin == nil || request.Origin != b.currentClient() {
			err = errors.New("approval no longer belongs to this window and server connection")
			break
		}
		response.Method, response.Params = "", nil
		if err = request.Origin.writeContext(requestCtx, response); err == nil {
			b.mu.Lock()
			if b.requests[string(response.ID)].Origin == request.Origin {
				delete(b.requests, string(response.ID))
				delete(b.approvals, string(response.ID))
			}
			b.mu.Unlock()
		}
		result = raw(nil)
	case "fastrock/update":
		b.mu.Lock()
		handler := b.updateHandler
		b.mu.Unlock()
		if handler == nil {
			err = errors.New("updates are unavailable in this process")
		} else {
			result = raw(handler(args.Check))
		}

	case "fastrock/publishThreads":
		var threads []OpenThread
		err = json.Unmarshal(args.Data, &threads)
		if len(threads) > 512 {
			err = errors.New("too many open threads")
		}
		if err == nil {
			var disabled []string
			b.mu.Lock()
			previous := map[string]bool{}
			for _, thread := range p.threads {
				previous[thread.ID] = thread.AcceptsMessages
			}
			for _, thread := range threads {
				if previous[thread.ID] && !thread.AcceptsMessages {
					disabled = append(disabled, thread.ID)
				}
			}
			p.threads = threads
			b.mu.Unlock()
			b.refreshDeliveries()
			if len(disabled) > 0 {
				b.broadcast("fastrock/messagingDisabled", map[string]any{"threadIds": disabled})
			}
		}
		result = raw(nil)
	case "fastrock/threads":
		b.mu.Lock()
		rows := []OpenThread{}
		seen := map[string]bool{}
		for _, window := range b.peers {
			for _, thread := range window.threads {
				if !seen[thread.ID] {
					thread.Window = window.id
					rows = append(rows, thread)
					seen[thread.ID] = true
				}
			}
		}
		b.mu.Unlock()
		result = raw(rows)
	case "fastrock/delivery":
		b.mu.Lock()
		target := b.owners[args.ThreadID]
		if target == nil {
			for _, window := range b.peers {
				for _, thread := range window.threads {
					if thread.ID == args.ThreadID {
						target = window
						break
					}
				}
			}
		}
		b.mu.Unlock()
		if target == nil {
			err = errors.New("recipient window closed")
		} else {
			err = target.send(Message{Method: "fastrock/delivery", Params: args.Data})
		}
		result = raw(nil)
	case "fastrock/restart":
		err = b.restart()
		result = raw(nil)
	case "fastrock/offer":
		b.mu.Lock()
		for k, t := range b.tickets {
			if time.Now().After(t.Expires) {
				delete(b.tickets, k)
				b.recordTransfer(k, t, TransferCancelled)
			}
		}
		ticket := secret()
		b.tickets[ticket] = transfer{Owner: p, Data: args.Data, Expires: time.Now().Add(time.Minute)}
		b.mu.Unlock()
		result = raw(map[string]string{"ticket": ticket})
	case "fastrock/claim":
		b.mu.Lock()
		t, ok := b.tickets[args.Ticket]
		if ok && time.Now().Before(t.Expires) && t.Claimed == nil {
			t.Claimed = p
			b.tickets[args.Ticket] = t
		} else {
			ok = false
		}
		b.mu.Unlock()
		if !ok {
			err = errors.New("window transfer expired or already claimed")
		} else {
			result = t.Data
		}
	case "fastrock/ready":
		b.mu.Lock()
		t, ok := b.tickets[args.Ticket]
		b.mu.Unlock()
		if !ok || t.Claimed != p {
			err = errors.New("window transfer is not owned by this window")
		} else {
			err = t.Owner.send(Message{Method: "fastrock/tabRefresh", Params: raw(map[string]string{"ticket": args.Ticket})})
			result = raw(nil)
		}
	case "fastrock/finalize":
		b.mu.Lock()
		t, ok := b.tickets[args.Ticket]
		if ok && t.Owner == p && t.Claimed != nil {
			t.Finalized = true
			t.Data = args.Data
			b.tickets[args.Ticket] = t
		} else {
			ok = false
		}
		b.mu.Unlock()
		if !ok {
			err = errors.New("window transfer expired")
		} else {
			err = t.Claimed.send(Message{Method: "fastrock/finalized", Params: raw(map[string]any{"ticket": args.Ticket, "data": json.RawMessage(args.Data)})})
		}
		result = raw(nil)
	case "fastrock/applied":
		b.mu.Lock()
		t, ok := b.tickets[args.Ticket]
		if ok && t.Claimed == p && t.Finalized {
			delete(b.tickets, args.Ticket)
			b.recordTransfer(args.Ticket, t, TransferCommitted)
		} else {
			ok = false
		}
		alreadyApplied := b.transferOutcome(args.Ticket, p) == TransferCommitted && b.transferReceipts[args.Ticket].claimed == p.id
		b.mu.Unlock()
		if !ok && !alreadyApplied {
			err = errors.New("window transfer not finalized")
		} else if ok {
			var payload struct{ Chat *struct{ ID string } }
			_ = json.Unmarshal(t.Data, &payload)
			if payload.Chat != nil {
				b.own(p, payload.Chat.ID)
			}
			_ = t.Owner.send(Message{Method: "fastrock/tabClaimed", Params: raw(map[string]string{"ticket": args.Ticket})})
		}
		result = raw(nil)

	case "fastrock/cancel":
		b.mu.Lock()
		t := b.tickets[args.Ticket]
		allowed := t.Owner == p || t.Claimed == p
		if allowed {
			delete(b.tickets, args.Ticket)
			b.recordTransfer(args.Ticket, t, TransferCancelled)
		}
		outcome := b.transferOutcome(args.Ticket, p)
		b.mu.Unlock()
		if allowed {
			for _, target := range []*peer{t.Owner, t.Claimed} {
				if target != nil && target != p {
					_ = target.send(Message{Method: "fastrock/tabCancelled", Params: raw(map[string]string{"ticket": args.Ticket})})
				}
			}
		}
		if outcome == "" {
			err = errors.New("window transfer outcome is unavailable for this window")
		} else {
			result = raw(TransferResult{State: outcome})
		}
	case "fastrock/own":
		b.own(p, args.ThreadID)
		result = raw(nil)
	case "fastrock/name":
		b.mu.Lock()
		p.name = args.Name
		b.mu.Unlock()
		result = raw(nil)
	case "fastrock/preferences":
		var patch settings.Patch
		if err = json.Unmarshal(args.Data, &patch); err != nil {
			break
		}
		b.preferencesMu.Lock()
		next := b.preferences
		if len(patch) > 0 {
			next, err = settings.Apply(next, patch)
			if err == nil && b.savePreferences != nil {
				err = b.savePreferences(next)
			}
			if err == nil {
				b.preferences = next
				b.preferencesRevision++
			}
		}
		state := PreferencesState{Revision: b.preferencesRevision, Data: b.preferences}
		if err == nil {
			result = raw(state)
			if len(patch) > 0 {
				b.broadcast("fastrock/preferences", state)
			}
		}
		b.preferencesMu.Unlock()
	case "fastrock/move":
		b.mu.Lock()
		dest := b.peers[args.Window]
		t, ok := b.tickets[args.Ticket]
		b.mu.Unlock()
		if dest == nil || !ok || t.Owner != p {
			err = errors.New("destination window closed")
		} else {
			err = dest.send(Message{Method: "fastrock/tabAvailable", Params: raw(map[string]string{"ticket": args.Ticket})})
		}
		result = raw(nil)
	case "fastrock/memory":
		b.mu.Lock()
		p.memory = args.Bytes
		total := b.serviceMemory.Load()
		for _, window := range b.peers {
			total += window.memory
		}
		count := len(b.peers)
		b.mu.Unlock()
		result = raw(map[string]any{"bytes": total, "count": count})
	case "fastrock/windows":
		b.mu.Lock()
		windows := []map[string]string{}
		for _, other := range b.peers {
			if other != p {
				windows = append(windows, map[string]string{"id": other.id, "name": other.name})
			}
		}
		b.mu.Unlock()
		result = raw(map[string]any{"windows": windows})

	default:
		if args.ThreadID != "" && (m.Method == "thread/start" || m.Method == "thread/resume" || m.Method == "turn/start" || m.Method == "turn/steer") {
			b.mu.Lock()
			if previous := b.owners[args.ThreadID]; previous == nil || previous == p {
				b.owners[args.ThreadID] = p
			}
			b.mu.Unlock()
		}
		c := b.currentClient()
		if c == nil {
			err = fmt.Errorf("%w; restart it in Settings", ErrDisconnected)
			break
		}
		ctx, cancel := context.WithCancel(requestCtx)
		defer cancel()
		if len(m.ID) == 0 {
			err = c.Notify(m.Method, json.RawMessage(m.Params))
		} else {
			err = c.Call(ctx, m.Method, json.RawMessage(m.Params), &result)
		}
	}
	if len(m.ID) == 0 {
		return
	}
	reply := Message{ID: m.ID, Result: result}
	if err != nil {
		reply.Result = nil
		reply.Error = protocolError(err)
	}
	if e := p.send(reply); e != nil {
		p.conn.Close()
	}
}

// OpenThread shares only presentation metadata between windows, never transcript or draft text.
type OpenThread struct {
	ID              string          `json:"thread_id"`
	Title           string          `json:"title"`
	Cwd             string          `json:"cwd"`
	Status          string          `json:"status"`
	AcceptsMessages bool            `json:"accepts_messages"`
	Window          string          `json:"window"`
	Permissions     PermissionScope `json:"permissions"`
}

func (b *Broker) events(c *Client) {
	for m := range c.Events {
		if b.currentClient() != c {
			return
		}
		m.Sequence = b.sequence.Add(1)
		b.deliveryEvent(m)
		var scope struct {
			ThreadID  string          `json:"threadId"`
			RequestID json.RawMessage `json:"requestId"`
		}
		_ = json.Unmarshal(m.Params, &scope)
		b.mu.Lock()
		targets := make([]*peer, 0, len(b.peers))
		owner := b.owners[scope.ThreadID]
		if m.Method == "serverRequest/resolved" {
			delete(b.approvals, string(scope.RequestID))
			delete(b.requests, string(scope.RequestID))
		}
		if m.Method == "turn/completed" {
			for id, request := range b.requests {
				var pending struct {
					ThreadID string `json:"threadId"`
				}
				_ = json.Unmarshal(request.Params, &pending)
				if pending.ThreadID == scope.ThreadID {
					delete(b.requests, id)
					delete(b.approvals, id)
				}
			}
		}
		if len(m.ID) > 0 {
			if owner == nil {
				for _, p := range b.peers {
					for _, thread := range p.threads {
						if thread.ID == scope.ThreadID {
							owner = p
							break
						}
					}
					if owner != nil {
						break
					}
				}
			}
			if owner != nil {
				targets = append(targets, owner)
				b.approvals[string(m.ID)] = owner
				m.Origin = c
				b.requests[string(m.ID)] = m
			}
		} else {
			for _, p := range b.peers {
				targets = append(targets, p)
			}
		}
		b.mu.Unlock()
		if len(m.ID) > 0 && owner == nil {
			_ = c.Reject(m.ID, "No open Fastrock window owns this thread; open it and retry")
			continue
		}
		for _, p := range targets {
			if err := p.send(m); err != nil {
				p.conn.Close()
			}
		}
	}
	// Serialize shutdown with replacement startup. Clearing delivery state takes
	// b.mu internally and must complete before a new client accepts messages.
	b.restartMu.Lock()
	defer b.restartMu.Unlock()
	b.mu.Lock()
	current := b.client == c
	if current {
		b.client = nil
	}
	b.mu.Unlock()
	if current {
		b.clearDeliveries("Codex app-server stopped before delivery")
		b.broadcast("fastrock/serverStopped", map[string]string{"message": "Codex app-server stopped. Restart it in Settings."})
	}
}
func Dial(ctx context.Context, address, token string) (*Client, error) {
	return DialWithStartup(ctx, ctx, address, token)
}

// DialWithStartup cancels connection setup without tying an established client
// to the short-lived setup context.
func DialWithStartup(parent, startup context.Context, address, token string) (*Client, error) {
	host, _, err := net.SplitHostPort(address)
	if err != nil || host != "127.0.0.1" {
		return nil, errors.New("invalid local broker address")
	}
	conn, err := (&net.Dialer{Timeout: 10 * time.Second}).DialContext(startup, "tcp4", address)
	if err != nil {
		return nil, err
	}
	ctx, cancel := context.WithCancel(parent)
	c := &Client{broker: true, input: conn, ctx: ctx, cancel: cancel, pending: map[string]chan Message{}, Events: make(chan Message, 256), done: make(chan struct{})}
	go c.read(conn)
	var response struct{ Version string }
	authCtx, stop := context.WithTimeout(startup, 10*time.Second)
	defer stop()
	if err = c.Call(authCtx, "fastrock/hello", map[string]string{"token": token}, &response); err != nil {
		c.Close()
		return nil, fmt.Errorf("local Codex connection: %w", err)
	}
	c.Version = response.Version
	return c, nil
}

func (b *Broker) own(p *peer, id string) {
	b.mu.Lock()
	old := b.owners[id]
	if old == p {
		b.mu.Unlock()
		return
	}
	b.owners[id] = p
	var pending []Message
	for key, owner := range b.approvals {
		if owner == old {
			m := b.requests[key]
			var args struct{ ThreadID string }
			_ = json.Unmarshal(m.Params, &args)
			if args.ThreadID == id {
				b.approvals[key] = p
				pending = append(pending, m)
			}
		}
	}
	b.mu.Unlock()
	b.refreshDeliveries()
	if old != nil && old != p {
		_ = old.send(Message{Method: "fastrock/threadMoved", Params: raw(map[string]string{"threadId": id})})
	}
	for _, m := range pending {
		_ = p.send(m)
	}
}

func (b *Broker) ReportServiceMemory(bytes uint64) { b.serviceMemory.Store(bytes) }
func (b *Broker) Done() <-chan struct{}            { return b.done }

func (b *Broker) currentClient() *Client {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.client
}
func (b *Broker) version() string {
	if c := b.currentClient(); c != nil {
		return c.Version
	}
	return "disconnected"
}
func (b *Broker) SetStarter(start func() (*Client, error)) {
	b.restartMu.Lock()
	defer b.restartMu.Unlock()
	b.starter = start
}
func (b *Broker) broadcast(method string, value any) {
	b.mu.Lock()
	targets := make([]*peer, 0, len(b.peers))
	for _, p := range b.peers {
		targets = append(targets, p)
	}
	b.mu.Unlock()
	m := Message{Method: method, Params: raw(value)}
	for _, p := range targets {
		if p.send(m) != nil {
			p.conn.Close()
		}
	}
}
func (b *Broker) restart() error {
	b.restartMu.Lock()
	defer b.restartMu.Unlock()
	if b.starter == nil {
		return errors.New("app-server restart unavailable")
	}
	select {
	case <-b.done:
		return errors.New("Fastrock is closing")
	default:
	}
	b.mu.Lock()
	old := b.client
	b.client = nil
	clear(b.approvals)
	clear(b.requests)
	b.mu.Unlock()
	b.clearDeliveries("Codex app-server restarted before delivery")
	b.broadcast("fastrock/serverStopped", map[string]any{"message": "Restarting Codex app-server…", "restarting": true})
	if old != nil {
		old.Close()
	}
	c, err := b.starter()
	if err != nil {
		b.broadcast("fastrock/serverStopped", map[string]string{"message": err.Error()})
		return err
	}
	b.mu.Lock()
	if b.closed {
		b.mu.Unlock()
		c.Close()
		return errors.New("Fastrock is closing")
	}
	b.client = c
	b.mu.Unlock()
	go b.events(c)
	b.broadcast("fastrock/serverReady", map[string]string{"version": c.Version})
	return nil
}

// SetUpdateHandler connects the one shared updater to every native window.
func (b *Broker) SetUpdateHandler(fn func(bool) any) {
	b.mu.Lock()
	b.updateHandler = fn
	b.mu.Unlock()
}
func (b *Broker) UpdateStatus(value any) { b.broadcast("fastrock/updateStatus", value) }
