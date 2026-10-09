//go:build nucular_headless

package ui

import (
	"context"
	"testing"
	"time"
)

func TestV4WorkerAdmissionKeepsControlAvailable(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	a := &App{ctx: ctx, updates: make(chan func(), 128)}
	entered := make(chan struct{}, 6)
	release := make(chan struct{})
	defer close(release)
	for range 6 {
		a.work(func() { entered <- struct{}{}; <-release })
	}
	for range 6 {
		select {
		case <-entered:
		case <-time.After(time.Second):
			t.Fatal("read worker did not start")
		}
	}
	for range 64 {
		if !a.work(func() {}) {
			t.Fatal("read queue filled prematurely")
		}
	}
	cleaned := false
	if a.work(func() { t.Error("rejected task ran") }, func() { cleaned = true }) {
		t.Fatal("unbounded admission")
	}
	control := make(chan struct{})
	if !a.controlWork(func() { close(control) }) {
		t.Fatal("control work rejected with reads full")
	}
	select {
	case <-control:
	case <-time.After(time.Second):
		t.Fatal("control blocked behind reads")
	}
	drain(t, a, func() bool { return cleaned })
}

func TestV4WorkerPanicClearsPendingState(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	a := &App{ctx: ctx, updates: make(chan func(), 128)}
	pending := true
	a.work(func() { panic("fixture failure") }, func() { pending = false })
	drain(t, a, func() bool { return !pending })
	finished := make(chan struct{})
	a.work(func() { close(finished) })
	select {
	case <-finished:
	case <-time.After(time.Second):
		t.Fatal("worker did not recover")
	}
}

func TestV4ReplyTimeoutsDoNotOccupyReadWorkers(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	a := &App{ctx: ctx, updates: make(chan func(), 128)}
	defer a.clearReplyWaits()
	for range 8 {
		waiter := &replyWait{}
		a.replyWaits = append(a.replyWaits, waiter)
		a.armReplyTimeout(waiter, time.Minute)
	}
	a.watchTransfer("tab", "ticket")
	done := make(chan struct{}, 6)
	for range 6 {
		a.work(func() { done <- struct{}{} })
	}
	for range 6 {
		select {
		case <-done:
		case <-time.After(time.Second):
			t.Fatal("reply timer occupied read workers")
		}
	}
}
