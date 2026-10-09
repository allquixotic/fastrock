package ui

import (
	"context"
	"encoding/json"
	"time"
)

// Failed writes leave the latest snapshot dirty. New snapshots supersede older
// retries, and only successful writes advance the deduplication baseline.
func persistSessions(ctx context.Context, input <-chan session, write func(session) error, report func(error)) {
	var pending *session
	var last string
	var retry <-chan time.Time
	delay := time.Second
	for {
		select {
		case <-ctx.Done():
			return
		case value := <-input:
			pending = &value
		case <-retry:
		}
		if pending == nil {
			continue
		}
		encoded, err := json.Marshal(pending)
		if err == nil && string(encoded) == last {
			pending = nil
			retry = nil
			continue
		}
		if err == nil {
			err = write(*pending)
		}
		if err != nil {
			report(err)
			retry = time.After(delay)
			delay = min(delay*2, 30*time.Second)
			continue
		}
		last = string(encoded)
		pending = nil
		retry = nil
		delay = time.Second
		report(nil)
	}
}
