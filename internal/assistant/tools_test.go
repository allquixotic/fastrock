package assistant

import (
	"context"
	"encoding/json"
	"github.com/allquixotic/fastrock/internal/mockrally"
	"github.com/allquixotic/fastrock/internal/rally"
	"net/http/httptest"
	"testing"
)

func setup(t *testing.T) (*rally.Client, *mockrally.Server) {
	t.Helper()
	m := mockrally.New()
	s := httptest.NewServer(m)
	t.Cleanup(s.Close)
	c, e := rally.New(s.URL, "mock-token", nil)
	if e != nil {
		t.Fatal(e)
	}
	return c, m
}
func TestProposalNeverWritesUntilApplied(t *testing.T) {
	c, _ := setup(t)
	var plan Plan
	tools := Tools{Client: c, Propose: func(p Plan) error { plan = p; return nil }}
	ref := rally.WSAPI + "hierarchicalrequirement/100"
	args := json.RawMessage(`{"summary":"Rename story","changes":[{"operation":"update","kind":"story","ref":"` + ref + `","fields":{"Name":"Renamed"}}]}`)
	if _, e := tools.Execute(context.Background(), "rally_propose", args); e != nil {
		t.Fatal(e)
	}
	before, _ := c.Get(context.Background(), ref)
	if before.String("Name") == "Renamed" {
		t.Fatal("proposal executed write")
	}
	if n, e := Apply(context.Background(), c, plan); e != nil || n != 1 {
		t.Fatal(n, e)
	}
	after, _ := c.Get(context.Background(), ref)
	if after.String("Name") != "Renamed" {
		t.Fatal("apply failed")
	}
}
func TestPlanDetectsStaleRevisionAndStops(t *testing.T) {
	c, _ := setup(t)
	ref := rally.WSAPI + "hierarchicalrequirement/100"
	before, _ := c.Get(context.Background(), ref)
	_, _ = c.Update(context.Background(), ref, "story", rally.Object{"Name": "Someone else's change"})
	plan := Plan{Changes: []Change{{Operation: "update", Kind: "story", Ref: ref, Before: before, Fields: rally.Object{"Name": "Overwrite"}}, {Operation: "create", Kind: "story", Fields: rally.Object{"Name": "Should not exist"}}}}
	if n, e := Apply(context.Background(), c, plan); n != 0 || e == nil {
		t.Fatal(n, e)
	}
}
func TestToolsRejectOutOfScopeAndIdentityEdits(t *testing.T) {
	c, _ := setup(t)
	tools := Tools{Client: c, Scope: rally.Query{Project: rally.WSAPI + "project/11"}, Propose: func(Plan) error { t.Fatal("invalid proposal accepted"); return nil }}
	for _, input := range []string{`{"summary":"wrong project","changes":[{"operation":"update","kind":"story","ref":"` + rally.WSAPI + `hierarchicalrequirement/100","fields":{"Name":"wrong"}}]}`, `{"summary":"rewrite identity","changes":[{"operation":"create","kind":"story","fields":{"ObjectID":1,"Name":"bad"}}]}`, `{"summary":"user edit","changes":[{"operation":"create","kind":"User","fields":{"Name":"bad"}}]}`} {
		if _, e := tools.Execute(context.Background(), "rally_propose", json.RawMessage(input)); e == nil {
			t.Fatalf("accepted %s", input)
		}
	}
}
func TestQueryAndShowView(t *testing.T) {
	c, _ := setup(t)
	shown := false
	tools := Tools{Client: c, Show: func(v View) error { shown = v.Page == "teamboard"; return nil }}
	result, e := tools.Execute(context.Background(), "rally_query", json.RawMessage(`{"kind":"story","query":"(Blocked = true)"}`))
	if e != nil || len(result.(rally.Page).Results) != 1 {
		t.Fatal(result, e)
	}
	_, e = tools.Execute(context.Background(), "rally_show_view", json.RawMessage(`{"page":"teamboard","query":"(Blocked = true)","group":"Owner"}`))
	if e != nil || !shown {
		t.Fatal(e)
	}
	if _, e = tools.Execute(context.Background(), "arbitrary_shell", json.RawMessage(`{}`)); e == nil {
		t.Fatal("unknown tool accepted")
	}
}
