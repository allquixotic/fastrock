//go:build fltk_headless

package ui

import (
	"fmt"
	"image"
	"net/http"
	"net/http/httptest"
	"reflect"
	"testing"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/command"
	"github.com/allquixotic/fastrock/internal/desktop/font"
	"github.com/allquixotic/fastrock/internal/rally"
	ifont "golang.org/x/image/font"
	"golang.org/x/mobile/event/mouse"
)

// Windows UI fonts can report a line advance smaller than ascent + descent.
type shortLineFace struct{ ifont.Face }

func (f shortLineFace) Metrics() ifont.Metrics {
	m := f.Face.Metrics()
	m.Height = m.Ascent / 2
	return m
}

func TestV80SingleClickChoicesAndReferenceLabels(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		fmt.Fprint(w, `{"QueryResult":{"Results":[{"_ref":"/slm/webservice/v2.0/user/2","DisplayName":"Casey Rivera"}],"TotalResultCount":1,"StartIndex":1}}`)
	}))
	defer server.Close()
	for _, field := range []rally.Field{
		{Name: "ScheduleState", AttributeType: "STATE", Required: true, AllowedValues: []string{"Defined", "Accepted"}},
		{Name: "Owner", AttributeType: "OBJECT", ReferenceType: "User"},
	} {
		t.Run(field.Name, func(t *testing.T) {
			a, v, o := inlineFixture(t, server.URL)
			v.Fields = []rally.Field{field}
			o["ScheduleState"] = "Defined"
			o["Owner"] = map[string]any{"_ref": "/slm/webservice/v2.0/user/1", "_refObjectName": "Alex Morgan"}
			var click image.Point
			h := desktop.NewHeadlessHarness(0, image.Pt(650, 450), func(w *desktop.Window) {
				m := &w.Master().Input().Mouse
				m.Pos, m.Buttons[mouse.ButtonLeft].ClickedPos = click, click
				m.Buttons[mouse.ButtonLeft].Clicked = click != (image.Point{})
				click = image.Point{}
				a.drawInlineStatus(w, v)
				w.Row(36).Static(140)
				a.drawInlineCell(w, v, o, field.Name)
			})
			h.Master().SetStyle(makeStyle(a.p, 13))
			h.Frame(false)
			click = image.Pt(60, 18)
			h.Frame(false)
			choice := "Accepted"
			if field.Name == "Owner" {
				if v.Detail == nil || v.Detail.referencePicker == nil {
					t.Fatal("one click did not open reference choices")
				}
				drain(t, a, func() bool { return !v.Detail.referencePicker.loading })
				choice = "Casey Rivera"
			}
			h.Frame(false)
			h.Frame(false)
			for _, c := range h.Commands() {
				if c.Kind == command.TextCmd && c.Text.String == choice {
					click = image.Pt(c.Rect.X+2, c.Rect.Y+c.Rect.H/2)
				}
			}
			if click == (image.Point{}) {
				t.Fatal("choice not rendered", choice)
			}
			h.Frame(false)
			if v.Detail == nil || v.Detail.inlineField != field.Name {
				t.Fatal("choice did not start cell edit")
			}
			want := "Accepted"
			if field.Name == "Owner" {
				want = rally.WSAPI + "user/2"
			}
			if got := v.Detail.fieldValue(field.Name); got != want {
				t.Fatal("choice lost reference/value", got, want)
			}
		})
	}
}

func TestV79TableTextFitsFontAndRankIsOrdinal(t *testing.T) {
	for _, scale := range []float64{1, 1.25, 1.5, 2} {
		t.Run(fmt.Sprint(scale), func(t *testing.T) {
			a, v, o := inlineFixture(t, "https://rally.test")
			v.Columns = []string{"Rank", "Name", "FormattedID"}
			v.Page, v.PageSize, v.Start, v.Total = 2, 25, 26, 26
			o["Name"] = "Long wrapping story title with descenders gyjp and another full line"
			h := desktop.NewHeadlessHarness(0, image.Pt(int(600*scale), int(450*scale)), func(w *desktop.Window) { a.table(w, v, v.Items) })
			s := makeStyle(a.p, 13)
			s.Scale(scale)
			s.Font = font.Face{Face: shortLineFace{s.Font.Face}}
			h.Master().SetStyle(s)
			h.Frame(false)
			ordinal := false
			for _, c := range h.Commands() {
				if c.Kind != command.TextCmd {
					continue
				}
				if c.Text.String == "ABCD" {
					t.Fatal("opaque rank token shown")
				}
				ordinal = ordinal || c.Text.String == "26"
				if c.Text.Foreground == a.p.Accent && c.Rect.H < desktop.FontHeight(c.Text.Face) {
					t.Fatalf("glyphs clipped: %q height %d need %d", c.Text.String, c.Rect.H, desktop.FontHeight(c.Text.Face))
				}
			}
			if !ordinal {
				t.Fatal("missing page-aware rank position")
			}
		})
	}
}

func TestV80InlineSchemaPayloads(t *testing.T) {
	for _, tc := range []struct {
		field  rally.Field
		before any
		input  string
		want   any
		bad    string
	}{
		{rally.Field{Name: "Name", AttributeType: "STRING", Required: true}, "Story", "New title", "New title", ""},
		{rally.Field{Name: "ScheduleState", AttributeType: "STATE", AllowedValues: []string{"Defined", "Accepted"}}, "Defined", "Accepted", "Accepted", "Invented"},
		{rally.Field{Name: "Owner", AttributeType: "OBJECT", ReferenceType: "User"}, map[string]any{"_ref": "https://rally.test/slm/webservice/v2.0/user/1", "_refObjectName": "Alex"}, "https://rally.test/slm/webservice/v2.0/user/2", "https://rally.test/slm/webservice/v2.0/user/2", "Sam"},
		{rally.Field{Name: "c_Count", AttributeType: "INTEGER"}, float64(1), "3", float64(3), "3.5"},
		{rally.Field{Name: "Estimate", AttributeType: "QUANTITY"}, float64(1), "2.5", float64(2.5), "-1"},
		{rally.Field{Name: "Ready", AttributeType: "BOOLEAN"}, false, "true", true, "bad"},
	} {
		t.Run(tc.field.Name, func(t *testing.T) {
			a, v, o := inlineFixture(t, "https://rally.test")
			v.Fields = []rally.Field{tc.field, {Name: "c_Unfetched", Required: true}}
			o[tc.field.Name] = tc.before
			a.startInline(v, o, tc.field.Name, nil)
			d := v.Detail
			if d == nil {
				t.Fatal("editable schema field refused")
			}
			if d.dirty() {
				t.Fatal("new inline editor is dirty")
			}
			setText(d.Editors[tc.field.Name], tc.input)
			got, err := a.inlineChanges(d)
			if err != nil || !reflect.DeepEqual(got, rally.Object{tc.field.Name: tc.want}) {
				t.Fatal(got, err)
			}
			setText(d.Editors[tc.field.Name], tc.bad)
			if _, err := a.inlineChanges(d); err == nil {
				t.Fatal("invalid value accepted", tc.bad)
			}
		})
	}
}
