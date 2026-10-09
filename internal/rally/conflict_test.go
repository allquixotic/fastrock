package rally

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"testing"
)

func TestV60RevisionConflictIsTypedAndDoesNotWrite(t *testing.T) {
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodGet {
			t.Error("stale edit reached write endpoint")
		}
		fmt.Fprint(w, `{"Task":{"ObjectID":1,"LastUpdateDate":"two"}}`)
	}))
	defer s.Close()
	c, _ := New(s.URL, "test", nil)
	_, err := c.UpdateIfUnchanged(context.Background(), Object{"_ref": s.URL + WSAPI + "task/1", "LastUpdateDate": "one"}, "Task", Object{"Name": "Updated"})
	var conflict *ConflictError
	if !errors.As(err, &conflict) {
		t.Fatal("conflict is not actionable", err)
	}
	if !c.SameReference(s.URL+WSAPI+"task/1", WSAPI+"task/1") || c.SameReference(s.URL+WSAPI+"task/1", "https://elsewhere.example"+WSAPI+"task/1") || c.SameReference(WSAPI+"task/1", WSAPI+"defect/1") || c.SameReference(WSAPI+"task/1", WSAPI+"task/2") {
		t.Fatal("reference identity comparison is unsafe")
	}
}
