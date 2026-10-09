//go:build nucular_headless

package ui

import (
	"errors"
	"os"
	"os/exec"
	"strings"
	"testing"
	"time"

	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/workspace"
)

type transferProcessFixture struct{ killed chan struct{} }

func (p *transferProcessFixture) Kill() error { p.killed <- struct{}{}; return nil }

func TestV27TransferChildLateStartAndCommit(t *testing.T) {
	for _, stop := range []bool{true, false} {
		child := &transferChild{}
		p := &transferProcessFixture{make(chan struct{}, 2)}
		child.resolve(stop)
		child.attach(p)
		child.resolve(!stop) // A late event cannot reverse the terminal decision.
		select {
		case <-p.killed:
			if !stop {
				t.Fatal("committed child killed")
			}
		default:
			if stop {
				t.Fatal("late child escaped cancellation")
			}
		}
		select {
		case <-p.killed:
			t.Fatal("child killed twice")
		default:
		}
	}
}

// This fixture runs only a sleeping Go test process; it never starts a GUI.
func TestV27PopoutProcessFixture(t *testing.T) {
	if len(os.Args) > 0 && os.Args[len(os.Args)-1] == "fastrock-idle-popout" {
		time.Sleep(time.Minute)
		os.Exit(0)
	}
}
func TestV27TransferStopsActualChild(t *testing.T) {
	binary, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	cmd := exec.Command(binary, "-test.run=^TestV27PopoutProcessFixture$", "--", "fastrock-idle-popout")
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = cmd.Process.Kill() })
	child := &transferChild{}
	child.attach(cmd.Process)
	done := make(chan error, 1)
	go func() { done <- cmd.Wait(); child.exited() }()
	child.resolve(true)
	select {
	case err := <-done:
		if err == nil {
			t.Fatal("child exited normally instead of being terminated")
		}
	case <-time.After(3 * time.Second):
		t.Fatal("cancelled child leaked")
	}
}

func TestV27TransferTimeoutResolvesBeforeKilling(t *testing.T) {
	for _, committed := range []bool{false, true} {
		t.Run(map[bool]string{false: "cancelled", true: "committed"}[committed], func(t *testing.T) {
			a := reviewFixture(t)
			b, err := codex.NewBroker(a.client)
			if err != nil {
				t.Fatal(err)
			}
			t.Cleanup(b.Close)
			source, err := codex.Dial(a.ctx, b.Address(), b.Token())
			if err != nil {
				t.Fatal(err)
			}
			t.Cleanup(source.Close)
			dest, err := codex.Dial(a.ctx, b.Address(), b.Token())
			if err != nil {
				t.Fatal(err)
			}
			t.Cleanup(dest.Close)
			a.client = source
			id := a.state.Open(workspace.File, "retained", "fixture", "")
			var offer struct{ Ticket string }
			if err := source.Call(a.ctx, "fastrock/offer", map[string]any{"data": map[string]string{"draft": "retained"}}, &offer); err != nil {
				t.Fatal(err)
			}
			args := map[string]string{"ticket": offer.Ticket}
			if committed {
				if err := dest.Call(a.ctx, "fastrock/claim", args, nil); err != nil {
					t.Fatal(err)
				}
				if err := source.Call(a.ctx, "fastrock/finalize", map[string]any{"ticket": offer.Ticket, "data": map[string]string{}}, nil); err != nil {
					t.Fatal(err)
				}
				if err := dest.Call(a.ctx, "fastrock/applied", args, nil); err != nil {
					t.Fatal(err)
				}
			}
			p := &transferProcessFixture{make(chan struct{}, 2)}
			child := &transferChild{}
			child.attach(p)
			a.transfers = map[string]string{offer.Ticket: id}
			a.popping = map[string]bool{id: true}
			a.transferChildren = map[string]*transferChild{offer.Ticket: child}
			a.watchTransferAfter(id, offer.Ticket, time.Millisecond)
			t.Cleanup(a.stopTransferTimers)
			drain(t, a, func() bool { return a.transfers[offer.Ticket] == "" })
			if (len(a.state.Tabs) == 0) != committed || a.popping[id] || len(a.transferTimers) > 0 {
				t.Fatal("wrong final source state")
			}
			if committed {
				select {
				case <-p.killed:
					t.Fatal("committed destination killed")
				default:
				}
			} else {
				select {
				case <-p.killed:
				case <-time.After(time.Second):
					t.Fatal("cancelled child not killed")
				}
			}
		})
	}
}

func TestV27UnknownTransferPreservesDocuments(t *testing.T) {
	a := reviewFixture(t)
	a.client = nil
	id := a.state.Open(workspace.File, "retained", "fixture", "")
	a.transfers = map[string]string{"ticket": id}
	a.popping = map[string]bool{id: true}
	p := &transferProcessFixture{make(chan struct{}, 2)}
	child := &transferChild{}
	child.attach(p)
	a.transferChildren = map[string]*transferChild{"ticket": child}
	a.abortTransfer(id, "ticket", errors.New("timeout"))
	if !a.popping[id] || len(a.state.Tabs) != 1 || !strings.Contains(a.toast, "unconfirmed") {
		t.Fatal("uncertain source was discarded or unfrozen")
	}
	select {
	case <-p.killed:
		t.Fatal("uncertain child killed")
	default:
	}
	// Broker loss leaves the received copy available even if applied's reply
	// was lost after the source had closed its copy.
	a.transferPending, a.incomingTicket, a.incomingTab = true, "ticket", id
	a.resolveIncoming("ticket", errors.New("lost reply"))
	if len(a.state.Tabs) != 1 || !a.transferPending {
		t.Fatal("uncertain incoming document discarded")
	}
}

func TestV27BrokerDisconnectRetainsPendingDocument(t *testing.T) {
	a := reviewFixture(t)
	client := a.client
	id := a.state.Open(workspace.File, "retained", "fixture", "")
	a.transferPending, a.incomingTicket, a.incomingTab = true, "ticket", id
	a.disconnected(client)
	if a.client != nil || len(a.state.Tabs) != 1 || !a.transferPending || !strings.Contains(a.status, "disconnected") {
		t.Fatal("broker death was not surfaced with pending document retained")
	}
}
