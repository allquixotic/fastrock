package rally

import (
	"context"
	"fmt"
	"net/http"
	"net/http/httptest"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

func TestPageCacheDeduplicatesScopesAndRefreshes(t *testing.T) {
	var calls atomic.Int64
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		time.Sleep(5 * time.Millisecond)
		fmt.Fprintf(w, `{"QueryResult":{"Results":[{"_ref":"/1","Name":"Card","Description":"large description"}],"StartIndex":%s,"TotalResultCount":10000,"PageSize":128}}`, r.URL.Query().Get("start"))
	}))
	defer server.Close()
	c, err := New(server.URL, "test", nil)
	if err != nil {
		t.Fatal(err)
	}
	q := Query{PageSize: 128, Fetch: "Name", Project: "/team/1"}
	var wg sync.WaitGroup
	for range 12 {
		wg.Go(func() {
			p, e := c.CachedQuery(context.Background(), "HierarchicalRequirement", q, false)
			if e != nil || len(p.Results) != 1 {
				t.Errorf("query: %v %v", p, e)
			} else if _, ok := p.Results[0]["Description"]; ok {
				t.Error("retained non-card fields")
			}
		})
	}
	wg.Wait()
	if n := calls.Load(); n != 1 {
		t.Fatalf("%d requests for identical concurrent query", n)
	}
	_, _ = c.CachedQuery(context.Background(), "HierarchicalRequirement", q, false)
	if calls.Load() != 1 {
		t.Fatal("cache miss")
	}
	_, _ = c.CachedQuery(context.Background(), "HierarchicalRequirement", q, true)
	q.Project = "/team/2"
	_, _ = c.CachedQuery(context.Background(), "HierarchicalRequirement", q, false)
	if calls.Load() != 3 {
		t.Fatal("manual refresh or scope isolation failed")
	}
	for i := 1; i < 40; i++ {
		q.Start = 1 + i*128
		_, _ = c.CachedQuery(context.Background(), "HierarchicalRequirement", q, false)
	}
	if len(c.cache.pages) > 32 || c.cache.bytes > 24<<20 {
		t.Fatal("unbounded page cache")
	}
	c.PurgeCache()
	if len(c.cache.pages) != 0 || c.cache.bytes != 0 {
		t.Fatal("cache retains pages")
	}
}

func TestPurgePreventsStaleInflightCache(t *testing.T) {
	started, release := make(chan struct{}), make(chan struct{})
	var calls atomic.Int64
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		n := calls.Add(1)
		if n == 1 {
			close(started)
			<-release
		}
		fmt.Fprintf(w, `{"QueryResult":{"Results":[{"Name":"version-%d"}],"StartIndex":1,"TotalResultCount":1}}`, n)
	}))
	defer server.Close()
	c, _ := New(server.URL, "test", nil)
	q := Query{Fetch: "Name"}
	done := make(chan struct{})
	go func() {
		defer close(done)
		_, _ = c.CachedQuery(context.Background(), "HierarchicalRequirement", q, false)
	}()
	<-started
	c.PurgeCache()
	page, err := c.CachedQuery(context.Background(), "HierarchicalRequirement", q, false)
	close(release)
	<-done
	if err != nil || page.Results[0].String("Name") != "version-2" {
		t.Fatal("joined stale pre-mutation query", err)
	}
	page, err = c.CachedQuery(context.Background(), "HierarchicalRequirement", q, false)
	if err != nil || page.Results[0].String("Name") != "version-2" || calls.Load() != 2 {
		t.Fatal("old flight repopulated cache")
	}
}

func TestSharedReadSurvivesFirstWaiterCancellation(t *testing.T) {
	started, release := make(chan struct{}), make(chan struct{})
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		close(started)
		select {
		case <-release:
			fmt.Fprint(w, `{"QueryResult":{"Results":[{"Name":"shared"}],"StartIndex":1,"TotalResultCount":1}}`)
		case <-r.Context().Done():
		}
	}))
	defer server.Close()
	c, _ := New(server.URL, "test", nil)
	first, cancel := context.WithCancel(context.Background())
	done1 := make(chan error, 1)
	done2 := make(chan error, 1)
	go func() { _, e := c.CachedQuery(first, "story", Query{}, false); done1 <- e }()
	<-started
	go func() { _, e := c.CachedQuery(context.Background(), "story", Query{}, false); done2 <- e }()
	deadline := time.Now().Add(time.Second)
	for {
		c.cache.mu.Lock()
		n := 0
		for _, f := range c.cache.flights {
			n = f.waiters
		}
		c.cache.mu.Unlock()
		if n == 2 {
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("second waiter did not join")
		}
		time.Sleep(time.Millisecond)
	}
	cancel()
	if e := <-done1; e != context.Canceled {
		t.Fatal(e)
	}
	close(release)
	if e := <-done2; e != nil {
		t.Fatalf("first caller cancelled shared read: %v", e)
	}
}

func TestAllRequestPathsShareConcurrencyLimit(t *testing.T) {
	var live, peak atomic.Int64
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		n := live.Add(1)
		defer live.Add(-1)
		for p := peak.Load(); n > p && !peak.CompareAndSwap(p, n); p = peak.Load() {
		}
		time.Sleep(10 * time.Millisecond)
		fmt.Fprint(w, `{"Thing":{"ObjectID":1,"Name":"ok"}}`)
	}))
	defer s.Close()
	c, _ := New(s.URL, "test", nil)
	var wg sync.WaitGroup
	for range 16 {
		wg.Go(func() {
			if _, e := c.Get(context.Background(), "thing/1"); e != nil {
				t.Error(e)
			}
		})
	}
	wg.Wait()
	if peak.Load() > 4 {
		t.Fatalf("%d simultaneous requests", peak.Load())
	}
}
