package codex

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"testing"
)

func TestBrokerPreservesProtocolErrorCategories(t *testing.T) {
	for _, test := range []struct {
		err  error
		code int
	}{
		{fmt.Errorf("wrapped: %w", &RPCError{Code: CodeMethodNotFound, Message: "unsupported"}), CodeMethodNotFound},
		{ErrDisconnected, CodeDisconnected}, {context.Canceled, CodeRequestCanceled}, {errors.New("rejected"), CodeRequestRejected},
	} {
		if got := protocolError(test.err).Code; got != test.code {
			t.Fatalf("%v: %d", test.err, got)
		}
	}
	p := &peer{ctx: context.Background(), out: make(chan []byte, 1)}
	b := &Broker{}
	b.request(context.Background(), p, Message{ID: json.RawMessage("1"), Method: "fastrock/windows", Params: json.RawMessage(`"invalid"`)})
	var response Message
	if err := json.Unmarshal(<-p.out, &response); err != nil {
		t.Fatal(err)
	}
	if response.Error == nil || response.Error.Code != CodeInvalidParams {
		t.Fatalf("%+v", response)
	}
}
