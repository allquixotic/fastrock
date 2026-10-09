package rally

import (
	"context"
	"encoding/json"
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
	done chan struct{}
	page Page
	err  error
}
type pageCache struct {
	epoch   uint64
	mu      sync.Mutex
	pages   map[string]cachedPage
	flights map[string]*pageFlight
	slots   chan struct{}
	bytes   int
}

func newPageCache() *pageCache {
	return &pageCache{pages: map[string]cachedPage{}, flights: map[string]*pageFlight{}, slots: make(chan struct{}, 4)}
}

// Results are immutable: mutation callers replace objects rather than editing
// shared maps. Cache keys include every scope, filter and ordering argument.
func (c *Client) CachedQuery(ctx context.Context, kind string, q Query, fresh bool) (Page, error) {
	cache := c.cache
	keyBytes, _ := json.Marshal(struct {
		Kind  string
		Query Query
	}{kind, q})
	key := string(keyBytes)
	now := time.Now()
	cache.mu.Lock()
	epoch := cache.epoch
	if !fresh {
		if hit, ok := cache.pages[key]; ok && now.Before(hit.Expires) {
			hit.Used = now
			cache.pages[key] = hit
			cache.mu.Unlock()
			return hit.Page, nil
		}
	}
	if flight := cache.flights[key]; flight != nil {
		cache.mu.Unlock()
		select {
		case <-ctx.Done():
			return Page{}, ctx.Err()
		case <-flight.done:
			return flight.page, flight.err
		}
	}
	flight := &pageFlight{done: make(chan struct{})}
	cache.flights[key] = flight
	cache.mu.Unlock()
	select {
	case cache.slots <- struct{}{}:
		flight.page, flight.err = c.Query(ctx, kind, q)
		<-cache.slots
	case <-ctx.Done():
		flight.err = ctx.Err()
	}
	if flight.err == nil && flight.page.Start != 0 && flight.page.Start != max(q.Start, 1) {
		flight.err = fmt.Errorf("Rally returned page %d, expected %d", flight.page.Start, max(q.Start, 1))
	}
	if flight.err == nil && q.Fetch != "" && q.Fetch != "true" {
		for _, o := range flight.page.Results {
			trimFetch(o, q.Fetch)
		}
	}
	cache.mu.Lock()
	if flight.err == nil && cache.epoch == epoch {
		size := pageBytes(flight.page)
		if old, ok := cache.pages[key]; ok {
			cache.bytes -= old.Bytes
			delete(cache.pages, key)
		}
		for len(cache.pages) > 0 && (len(cache.pages) >= 32 || cache.bytes+size > 24<<20) {
			oldKey := ""
			oldest := time.Now().Add(time.Hour)
			for k, p := range cache.pages {
				if p.Used.Before(oldest) {
					oldest = p.Used
					oldKey = k
				}
			}
			cache.bytes -= cache.pages[oldKey].Bytes
			delete(cache.pages, oldKey)
		}
		if size <= 24<<20 {
			cache.pages[key] = cachedPage{Bytes: size, Page: flight.page, Used: now, Expires: now.Add(45 * time.Second)}
			cache.bytes += size
		}

	}
	if cache.flights[key] == flight {
		delete(cache.flights, key)
	}
	close(flight.done)
	cache.mu.Unlock()
	return flight.page, flight.err
}
func (c *Client) PurgeCache() {
	c.cache.mu.Lock()
	clear(c.cache.pages)
	clear(c.cache.flights)
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
