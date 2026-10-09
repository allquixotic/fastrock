package codex

import (
	"context"
	"fmt"
	"sync"
	"testing"
	"time"
)

func TestV27TransferCancellationCommitRace(t *testing.T) {
	b, err := NewBroker(helper(t))
	if err != nil {
		t.Fatal(err)
	}
	defer b.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	source, err := Dial(ctx, b.Address(), b.Token())
	if err != nil {
		t.Fatal(err)
	}
	defer source.Close()
	dest, err := Dial(ctx, b.Address(), b.Token())
	if err != nil {
		t.Fatal(err)
	}
	defer dest.Close()
	stranger, err := Dial(ctx, b.Address(), b.Token())
	if err != nil {
		t.Fatal(err)
	}
	defer stranger.Close()
	for range 32 {
		var offer struct{ Ticket string }
		if err := source.Call(ctx, "fastrock/offer", map[string]any{"data": map[string]string{"draft": "retained"}}, &offer); err != nil {
			t.Fatal(err)
		}
		args := map[string]string{"ticket": offer.Ticket}
		if err := dest.Call(ctx, "fastrock/claim", args, nil); err != nil {
			t.Fatal(err)
		}
		if err := source.Call(ctx, "fastrock/finalize", map[string]any{"ticket": offer.Ticket, "data": map[string]string{"draft": "latest"}}, nil); err != nil {
			t.Fatal(err)
		}
		var result TransferResult
		var appliedErr, cancelErr error
		var wg sync.WaitGroup
		wg.Go(func() { appliedErr = dest.Call(ctx, "fastrock/applied", args, nil) })
		wg.Go(func() { cancelErr = source.Call(ctx, "fastrock/cancel", args, &result) })
		wg.Wait()
		if cancelErr != nil {
			t.Fatal(cancelErr)
		}
		switch result.State {
		case TransferCancelled:
			if appliedErr == nil {
				t.Fatal("cancelled transfer committed")
			}
		case TransferCommitted:
			if appliedErr != nil {
				t.Fatal(appliedErr)
			}
			if err := dest.Call(ctx, "fastrock/applied", args, nil); err != nil {
				t.Fatal("commit retry not idempotent", err)
			}
		default:
			t.Fatalf("unknown outcome: %q", result.State)
		}
		var retry TransferResult
		if err := dest.Call(ctx, "fastrock/cancel", args, &retry); err != nil || retry != result {
			t.Fatalf("lost terminal outcome: %+v %v", retry, err)
		}
		if err := stranger.Call(ctx, "fastrock/cancel", args, nil); err == nil {
			t.Fatal("unrelated peer accessed receipt")
		}
	}
}

func TestV27TransferReceiptsBounded(t *testing.T) {
	b := &Broker{}
	owner, dest := &peer{id: "source"}, &peer{id: "dest"}
	for i := range 1000 {
		b.recordTransfer(fmt.Sprint(i), transfer{Owner: owner, Claimed: dest}, TransferCommitted)
	}
	if len(b.transferReceipts) != transferReceiptLimit {
		t.Fatal(len(b.transferReceipts))
	}
	if b.transferOutcome("999", dest) != TransferCommitted {
		t.Fatal("latest receipt missing")
	}
	r := b.transferReceipts["999"]
	r.expires = time.Now().Add(-time.Second)
	b.transferReceipts["999"] = r
	if b.transferOutcome("999", owner) != "" {
		t.Fatal("expired receipt accepted")
	}
}
