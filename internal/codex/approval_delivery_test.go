package codex

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"testing"
)

type approvalWriter struct{ err error }

func (w *approvalWriter) Write(p []byte) (int, error) {
	if w.err != nil {
		return 0, w.err
	}
	return len(p), nil
}
func (*approvalWriter) Close() error { return nil }

func TestBrokerApprovalAcknowledgesServerWrite(t *testing.T) {
	for _, fail := range []bool{false, true} {
		t.Run(map[bool]string{false: "delivered", true: "failed"}[fail], func(t *testing.T) {
			writer := &approvalWriter{}
			if fail {
				writer.err = io.ErrClosedPipe
			}
			client := &Client{ctx: context.Background(), input: writer}
			window := &peer{ctx: context.Background(), out: make(chan []byte, 1)}
			broker := &Broker{client: client, approvals: map[string]*peer{"7": window}, requests: map[string]Message{"7": {ID: json.RawMessage("7"), Origin: client}}}
			broker.request(context.Background(), window, Message{ID: json.RawMessage("1"), Method: "fastrock/respond", Params: raw(Message{ID: json.RawMessage("7"), Result: raw(true)})})
			var reply Message
			if err := json.Unmarshal(<-window.out, &reply); err != nil {
				t.Fatal(err)
			}
			if (reply.Error != nil) != fail {
				t.Fatalf("wrong delivery acknowledgement: %+v", reply)
			}
			_, retained := broker.requests["7"]
			if retained != fail {
				t.Fatal("request ownership was discarded before successful delivery")
			}
		})
	}
}

func TestBrokerRejectsApprovalFromPreviousConnection(t *testing.T) {
	old := &Client{ctx: context.Background(), input: &approvalWriter{err: errors.New("must not write")}}
	current := &Client{}
	window := &peer{ctx: context.Background(), out: make(chan []byte, 1)}
	broker := &Broker{client: current, approvals: map[string]*peer{"7": window}, requests: map[string]Message{"7": {Origin: old}}}
	broker.request(context.Background(), window, Message{ID: json.RawMessage("1"), Method: "fastrock/respond", Params: raw(Message{ID: json.RawMessage("7"), Result: raw(true)})})
	var reply Message
	json.Unmarshal(<-window.out, &reply)
	if reply.Error == nil {
		t.Fatal("answered approval on a replacement connection")
	}
}

func TestV35MessagingOptOutReachesOtherWindows(t *testing.T) {
	source := &peer{id: "source", ctx: context.Background(), out: make(chan []byte, 4)}
	target := &peer{id: "target", ctx: context.Background(), out: make(chan []byte, 4), threads: []OpenThread{{ID: "chat", AcceptsMessages: true}}}
	b := &Broker{peers: map[string]*peer{source.id: source, target.id: target}}
	b.request(context.Background(), target, Message{Method: "fastrock/publishThreads", Params: raw(map[string]any{"data": []OpenThread{{ID: "chat", AcceptsMessages: false}}})})
	select {
	case data := <-source.out:
		var m Message
		if err := json.Unmarshal(data, &m); err != nil {
			t.Fatal(err)
		}
		var params struct {
			ThreadIDs []string `json:"threadIds"`
		}
		if err := json.Unmarshal(m.Params, &params); err != nil {
			t.Fatal(err)
		}
		if m.Method != "fastrock/messagingDisabled" || len(params.ThreadIDs) != 1 || params.ThreadIDs[0] != "chat" {
			t.Fatal(m, params)
		}
	default:
		t.Fatal("other window retained its pending consent")
	}
}
