package rally_test

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/allquixotic/fastrock/internal/mockrally"
	"github.com/allquixotic/fastrock/internal/rally"
)

func fixture(t *testing.T) (*rally.Client, *mockrally.Server, string) {
	t.Helper()
	mock := mockrally.New()
	s := httptest.NewServer(mock)
	t.Cleanup(s.Close)
	c, e := rally.New(s.URL, "mock-token", nil)
	if e != nil {
		t.Fatal(e)
	}
	return c, mock, s.URL
}
func TestEndpointValidation(t *testing.T) {
	for _, endpoint := range []string{"", "ftp://rally.test", "http://rally.test", "https://user:pass@rally.test", "https://rally.test/?token=secret"} {
		if _, e := rally.New(endpoint, "token", nil); e == nil {
			t.Errorf("accepted %q", endpoint)
		}
	}
	for _, endpoint := range []string{"https://rally.test", "https://rally.test/slm/webservice/v2.0", "http://127.0.0.1:4567"} {
		if _, e := rally.New(endpoint, "token", nil); e != nil {
			t.Error(e)
		}
	}
}
func TestPaginationScopeAndUnknownFields(t *testing.T) {
	c, _, _ := fixture(t)
	q := rally.Query{Project: rally.WSAPI + "project/10", Expression: rally.Eq("Blocked", "true"), PageSize: 2}
	p, e := c.Query(context.Background(), "story", q)
	if e != nil {
		t.Fatal(e)
	}
	if len(p.Results) != 1 || p.Results[0].String("c_TeamNote") != "Ready for review" {
		t.Fatalf("bad query: %+v", p)
	}
	all, e := c.All(context.Background(), "story", rally.Query{})
	if e != nil || len(all) != 20 {
		t.Fatalf("%d %v", len(all), e)
	}
	p, e = c.Query(context.Background(), "story", rally.Query{Start: 3, PageSize: 2})
	if e != nil || p.Start != 3 || len(p.Results) != 2 {
		t.Fatalf("%+v %v", p, e)
	}
}
func TestReadWriteLifecycleAndSchema(t *testing.T) {
	c, _, _ := fixture(t)
	ctx := context.Background()
	o, e := c.Create(ctx, "story", rally.Object{"Name": "Created through native client", "c_Foo": "custom"})
	if e != nil {
		t.Fatal(e)
	}
	o, e = c.Update(ctx, o.String("_ref"), o.Kind(), rally.Object{"Name": "Updated"})
	if e != nil || o.String("Name") != "Updated" {
		t.Fatalf("%v %v", o, e)
	}
	full, e := c.Get(ctx, o.String("_ref"))
	if e != nil || full.String("c_Foo") != "custom" {
		t.Fatalf("custom fields lost: %v %v", full, e)
	}
	fields, e := c.Fields(ctx, "story")
	if e != nil || len(fields) != 4 {
		t.Fatalf("schema: %v %v", fields, e)
	}
	if e = c.Delete(ctx, o.String("_ref")); e != nil {
		t.Fatal(e)
	}
}
func TestTokenNeverSentToOtherOriginOrOutsideWSAPI(t *testing.T) {
	c, _, server := fixture(t)
	for _, ref := range []string{"http://127.0.0.1:1/slm/webservice/v2.0/user/1", server + "/other", rally.WSAPI + "../outside", "//attacker.invalid/stolen"} {
		if _, e := c.Get(context.Background(), ref); e == nil {
			t.Errorf("accepted %q", ref)
		}
	}
	var leaked atomic.Int32
	other := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { leaked.Add(1) }))
	defer other.Close()
	redirect := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { http.Redirect(w, r, other.URL, http.StatusFound) }))
	defer redirect.Close()
	c, _ = rally.New(redirect.URL, "supersecret", nil)
	if _, e := c.Get(context.Background(), "user/1"); e == nil {
		t.Fatal("followed redirect")
	}
	if leaked.Load() != 0 {
		t.Fatal("token request leaked")
	}
}
func TestRetryCancellationAndMutationsNeverRetried(t *testing.T) {
	var calls atomic.Int32
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		w.Header().Set("Retry-After", "10")
		w.WriteHeader(429)
	}))
	defer s.Close()
	c, _ := rally.New(s.URL, "secret", nil)
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Millisecond)
	defer cancel()
	_, e := c.Query(ctx, "story", rally.Query{})
	if !errors.Is(e, context.DeadlineExceeded) {
		t.Fatalf("wanted cancellation: %v", e)
	}
	calls.Store(0)
	_, e = c.Create(context.Background(), "story", rally.Object{"Name": "test"})
	if e == nil || calls.Load() != 1 {
		t.Fatalf("write retried: %d %v", calls.Load(), e)
	}
}
func TestWSErrorsAndAuth(t *testing.T) {
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("ZSESSIONID") != "token" {
			t.Error("missing auth")
		}
		fmt.Fprint(w, `{"OperationResult":{"Errors":["bad token"],"Warnings":[]}}`)
	}))
	defer s.Close()
	c, _ := rally.New(s.URL, "token", nil)
	_, e := c.Get(context.Background(), "user/1")
	if e == nil || strings.Contains(e.Error(), "bad token") {
		t.Fatalf("not redacted: %v", e)
	}
}
func TestQueryEscapingAndScopeValues(t *testing.T) {
	q := rally.Query{Expression: rally.Eq("Name", `a"b\c`), Workspace: "/workspace/1", Project: "/project/2", Parents: true, Children: false, PageSize: 9000}
	v := rally.QueryValues(q)
	if v.Get("pagesize") != "2000" || v.Get("projectScopeUp") != "true" || v.Get("projectScopeDown") != "false" {
		t.Fatal(v)
	}
	encoded, err := url.ParseQuery(v.Encode())
	if err != nil || encoded.Get("query") != q.Expression {
		t.Fatal(err)
	}
	b, _ := json.Marshal(rally.Object{"Name": "x"})
	if len(b) == 0 {
		t.Fatal("marshal")
	}
}

func TestAllRejectsIncompletePagination(t *testing.T) {
	for _, body := range []string{
		`{"QueryResult":{"Results":[],"TotalResultCount":2,"StartIndex":1}}`,
		`{"QueryResult":{"Results":[{"ObjectID":1}],"TotalResultCount":2,"StartIndex":99}}`,
	} {
		server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { fmt.Fprint(w, body) }))
		c, _ := rally.New(server.URL, "test", nil)
		if _, err := c.All(context.Background(), "story", rally.Query{}); err == nil {
			t.Error("accepted invalid pagination")
		}
		server.Close()
	}
}
