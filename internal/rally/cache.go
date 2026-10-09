package rally

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"sync"
	"time"
)

type cachedPage struct {
	Bytes         int
	Page          Page
	Used, Expires time.Time
}
type pageFlight struct {
	done    chan struct{}
	page    Page
	err     error
	cancel  context.CancelFunc
	waiters int
	serial  uint64
}
type pageCache struct {
	epoch   uint64
	mu      sync.Mutex
	pages   map[string]cachedPage
	flights map[string]*pageFlight
	bytes   int
	serial  uint64
	latest  map[string]uint64
}

func newPageCache() *pageCache {
	return &pageCache{pages: map[string]cachedPage{}, flights: map[string]*pageFlight{}, latest: map[string]uint64{}}
}

// Results are immutable: mutation callers replace objects rather than editing
// shared maps. Cache keys include every scope, filter and ordering argument.
// ErrStaleQuery means a mutation or explicit invalidation superseded this read.
var ErrStaleQuery = errors.New("Rally query was invalidated; refresh the view")

func (c *Client) CachedQuery(ctx context.Context, kind string, q Query, fresh bool) (Page, error) {
	if err := ctx.Err(); err != nil {
		return Page{}, err
	}
	cache := c.cache
	keyBytes, _ := json.Marshal(struct {
		Kind  string
		Query Query
	}{kind, q})
	key := string(keyBytes)
	cache.mu.Lock()
	now := time.Now()
	if !fresh {
		if hit, ok := cache.pages[key]; ok && now.Before(hit.Expires) {
			hit.Used = now
			cache.pages[key] = hit
			cache.mu.Unlock()
			return hit.Page, nil
		}
	}
	epoch := cache.epoch
	flightKey := fmt.Sprintf("%d:%s", epoch, key)
	// An explicit refresh must not inherit a read that started before it.
	if fresh {
		flightKey += ":fresh"
	}
	flight := cache.flights[flightKey]
	if flight == nil {
		if len(cache.flights) >= 64 {
			cache.mu.Unlock()
			return Page{}, errors.New("too many pending Rally reads; try again shortly")
		}
		work, cancel := context.WithTimeout(context.WithoutCancel(ctx), 30*time.Second)
		cache.serial++
		flight = &pageFlight{done: make(chan struct{}), cancel: cancel, serial: cache.serial}
		cache.latest[key] = flight.serial
		cache.flights[flightKey] = flight
		go func() {
			defer cancel()
			page, err := c.Query(work, kind, q)
			if err == nil && page.Start != 0 && page.Start != max(q.Start, 1) {
				err = fmt.Errorf("Rally returned page %d, expected %d", page.Start, max(q.Start, 1))
			}
			if err == nil && q.Fetch != "" && q.Fetch != "true" {
				for _, o := range page.Results {
					trimFetch(o, q.Fetch)
				}
			}
			cache.mu.Lock()
			defer cache.mu.Unlock()
			if err == nil && (cache.epoch != epoch || cache.latest[key] != flight.serial || work.Err() != nil) {
				err = ErrStaleQuery
			}
			flight.page, flight.err = page, err
			if err == nil {
				size := pageBytes(page)
				if old, ok := cache.pages[key]; ok {
					cache.bytes -= old.Bytes
					delete(cache.pages, key)
				}
				for len(cache.pages) > 0 && (len(cache.pages) >= 32 || cache.bytes+size > 24<<20) {
					oldestKey := ""
					oldest := time.Now().Add(time.Hour)
					for k, p := range cache.pages {
						if p.Used.Before(oldest) {
							oldest, oldestKey = p.Used, k
						}
					}
					cache.bytes -= cache.pages[oldestKey].Bytes
					delete(cache.pages, oldestKey)
				}
				if size <= 24<<20 {
					cache.pages[key] = cachedPage{Bytes: size, Page: page, Used: time.Now(), Expires: time.Now().Add(45 * time.Second)}
					cache.bytes += size
				}
			}
			if cache.flights[flightKey] == flight {
				delete(cache.flights, flightKey)
			}
			if cache.latest[key] == flight.serial {
				delete(cache.latest, key)
			}
			close(flight.done)
		}()
	}
	flight.waiters++
	cache.mu.Unlock()
	defer func() {
		cache.mu.Lock()
		defer cache.mu.Unlock()
		flight.waiters--
		if flight.waiters == 0 {
			flight.cancel()
			if cache.flights[flightKey] == flight {
				delete(cache.flights, flightKey)
			}
		}
	}()
	select {
	case <-ctx.Done():
		return Page{}, ctx.Err()
	case <-flight.done:
		return flight.page, flight.err
	}
}
func (c *Client) PurgeCache() {
	c.cache.mu.Lock()
	clear(c.cache.pages)
	clear(c.cache.latest)
	c.cache.epoch++
	c.cache.bytes = 0
	c.cache.mu.Unlock()
}

func trimFetch(o Object, fetch string) {
	for k := range o {
		if strings.HasPrefix(k, "_") || k == "ObjectID" {
			continue
		}
		found := false
		for field := range strings.SplitSeq(fetch, ",") {
			if strings.EqualFold(k, field) {
				found = true
				break
			}
		}
		if !found {
			delete(o, k)
		}
	}
}
func pageBytes(p Page) int {
	n := 128
	for _, o := range p.Results {
		n += valueBytes(map[string]any(o))
	}
	return n
}
func valueBytes(v any) int {
	switch x := v.(type) {
	case string:
		return len(x) + 16
	case map[string]any:
		n := 64
		for k, v := range x {
			n += len(k) + 64 + valueBytes(v)
		}
		return n
	case []any:
		n := 24
		for _, v := range x {
			n += valueBytes(v) + 16
		}
		return n
	default:
		return 16
	}
}
