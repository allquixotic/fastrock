package rally

import (
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"strings"
	"testing"
)

func TestV5TypedPageEnvelope(t *testing.T) {
	c := &Client{token: "private-token"}
	p, err := c.decodePage([]byte(`{"QueryResult":{"Errors":[],"Results":[{"ObjectID":42,"Name":"example","c_Custom":{"value":"kept"}}],"TotalResultCount":7,"StartIndex":3,"PageSize":2}}`), 200)
	if err != nil || p.Start != 3 || p.Total != 7 || p.PageSize != 2 || len(p.Results) != 1 || p.Results[0]["c_Custom"].(map[string]any)["value"] != "kept" {
		t.Fatalf("%+v %v", p, err)
	}
	for _, tc := range []struct {
		body   string
		status int
		api    bool
	}{
		{`{"QueryResult":{"Errors":["private-token rejected"]}}`, 200, true},
		{`{"OperationResult":{"Errors":["private-token rejected"]}}`, 200, true},
		{`upstream unavailable`, 502, true},
		{`{}`, http.StatusUnauthorized, true},
		{`{"QueryResult":null}`, 200, false},
		{`{"QueryResult":{"Results":"invalid"}}`, 200, false},
		{`{"QueryResult":{}} trailing`, 200, false},
	} {
		_, err := c.decodePage([]byte(tc.body), tc.status)
		var api *APIError
		if err == nil || errors.As(err, &api) != tc.api || strings.Contains(err.Error(), c.token) {
			t.Fatalf("body %s error %v", tc.body, err)
		}
	}
}

func BenchmarkTypedPageDecode(b *testing.B) {
	var raw strings.Builder
	raw.WriteString(`{"QueryResult":{"Results":[`)
	for i := 0; i < 1000; i++ {
		if i > 0 {
			raw.WriteByte(',')
		}
		fmt.Fprintf(&raw, `{"ObjectID":%d,"Name":"A work item","Description":"<p>Example rich content</p>","Owner":{"_ref":"/user/1","_refObjectName":"Example"}}`, i)
	}
	raw.WriteString(`],"TotalResultCount":1000,"StartIndex":1,"PageSize":1000}}`)
	data := []byte(raw.String())
	c := &Client{token: "secret"}
	b.Run("typed", func(b *testing.B) {
		for i := 0; i < b.N; i++ {
			if _, err := c.decodePage(data, 200); err != nil {
				b.Fatal(err)
			}
		}
	})
	b.Run("previous-envelope", func(b *testing.B) {
		for i := 0; i < b.N; i++ {
			var envelope map[string]json.RawMessage
			if err := json.Unmarshal(data, &envelope); err != nil {
				b.Fatal(err)
			}
			for _, value := range envelope {
				var status responseStatus
				if err := json.Unmarshal(value, &status); err != nil {
					b.Fatal(err)
				}
			}
			var page queryResult
			if err := json.Unmarshal(envelope["QueryResult"], &page); err != nil {
				b.Fatal(err)
			}
		}
	})
}
