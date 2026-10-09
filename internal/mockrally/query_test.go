package mockrally

import (
	"github.com/allquixotic/fastrock/internal/rally"
	"testing"
)

func TestNestedBooleanSearch(t *testing.T) {
	o := rally.Object{"FormattedID": "US1001", "Name": "Story (A OR B)", "Description": "Hello", "Blocked": true}
	for _, q := range []string{`(((Name contains "US1001") OR (Description contains "US1001")) OR (FormattedID contains "US1001"))`, `((Blocked = true) AND ((FormattedID = "US1001") OR (Name = "absent")))`, `(Name = "Story (A OR B)")`, `(Owner = null)`} {
		if !matches(o, q) {
			t.Fatal(q)
		}
	}
	if matches(o, `((Name = "absent") OR (Blocked = false))`) {
		t.Fatal("incorrect match")
	}
}
