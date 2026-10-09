//go:build fltk_headless

package ui

import (
	"context"
	"encoding/json"
	"fmt"
	"image"
	"image/color"
	"net/http"
	"net/http/httptest"
	"slices"
	"strings"
	"testing"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/command"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/workspace"
	"golang.org/x/mobile/event/mouse"
)

func TestV61RejectsForeignAndWrongCollectionReferences(t *testing.T) {
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		t.Error("invalid selection reached Rally")
		w.WriteHeader(500)
	}))
	defer s.Close()
	a := presetApp(t)
	a.rallyClient, _ = rally.New(s.URL, "test", nil)
	f := rally.Field{Name: "Tags", AttributeType: "COLLECTION"}
	for _, ref := range []string{"https://foreign.example" + rally.WSAPI + "tag/1", s.URL + rally.WSAPI + "project/1"} {
		d := makeDetail(rally.Object{"_ref": s.URL + rally.WSAPI + "hierarchicalrequirement/1", "Name": "Work"}, "HierarchicalRequirement", false)
		mergeSchemaEditors(d, []rally.Field{f})
		setText(d.Editors["Tags"], encodeCollectionRefs([]string{ref}))
		v := newRallyView(rally.FindPage("teamboard"))
		v.Detail = d
		a.saveDetail(v)
		if d.Saving || d.Error == "" {
			t.Fatal("invalid reference was accepted for mutation", ref)
		}
		if _, err := readPropertyCollection(context.Background(), a.rallyClient, rally.Object{"Tags": []any{map[string]any{"_ref": ref}}}, f); err == nil {
			t.Fatal("hydrated selection bypassed reference validation", ref)
		}
	}
}

func TestV61CollectionDraftPayloadPickerAndTransfer(t *testing.T) {
	a := presetApp(t)
	a.rallyClient, _ = rally.New("https://rally.test", "test", nil)
	id := a.state.Open(workspace.Rally, "Board", "", "teamboard")
	v := newRallyView(rally.FindPage("teamboard"))
	a.rallyViews[id] = v
	oldRef, newRef := "https://rally.test"+rally.WSAPI+"tag/1", "https://rally.test"+rally.WSAPI+"tag/2"
	d := makeDetail(rally.Object{"Name": "Work", "Tags": []any{map[string]any{"_ref": oldRef, "Name": "Existing tag"}}}, "HierarchicalRequirement", false)
	f := rally.Field{Name: "Tags", AttributeType: "COLLECTION"}
	mergeSchemaEditors(d, []rally.Field{f})
	v.Detail = d
	if d.dirty() {
		t.Fatal("hydrated tags dirtied the form")
	}
	p := &referencePicker{detail: d, field: f, editor: d.Editors["Tags"], editorRevision: d.Editors["Tags"].TextRevision(), client: a.rallyClient, kinds: []string{"Tag"}, items: []rally.Object{{"_ref": newRef, "Name": "New tag"}}, search: textEditor("tag", false)}
	q := referenceQuery(p, 1)
	if strings.Contains(q.Expression, "FormattedID") || strings.Contains(q.Fetch, "FormattedID") {
		t.Fatal("tag search queries a field not supported by Tags", q)
	}
	d.referencePicker = p
	if !a.selectReference(p, p.items[0]) {
		t.Fatal("collection picker failed")
	}
	refs, err := decodeCollectionRefs(text(d.Editors["Tags"]))
	if err != nil || !slices.Equal(refs, []string{oldRef, newRef}) || d.referenceLabels["Tags\x00"+oldRef] != "Existing tag" {
		t.Fatal("adding replaced existing tags or labels", refs, err)
	}
	changes, err := d.changes()
	if err != nil {
		t.Fatal(err)
	}
	objects, ok := changes["Tags"].([]any)
	if !ok || len(objects) != 2 || rally.Object(objects[1].(map[string]any)).String("_ref") != newRef {
		t.Fatal("tags payload is not an array of reference objects", changes)
	}
	b := presetApp(t)
	if err := b.installTransfer(transferJSON(t, a.tabSnapshot(*a.state.Current()))); err != nil {
		t.Fatal(err)
	}
	restored := b.rallyViews[b.state.Active].Detail
	if text(restored.Editors["Tags"]) != text(d.Editors["Tags"]) || !restored.dirty() {
		t.Fatal("transferred collection lost its edits")
	}
	submitted := d.values()
	d.acceptSave(d.savedSnapshot(d.Original, rally.Object{"Name": "Work", "Tags": map[string]any{"Count": float64(2), "_ref": "https://rally.test" + rally.WSAPI + "hierarchicalrequirement/1/Tags"}}, changes), submitted)
	if d.dirty() || d.referenceLabels["Tags\x00"+newRef] != "New tag" {
		t.Fatal("sparse acknowledgment lost saved collection state")
	}
	d.acceptSave(d.savedSnapshot(d.Original, rally.Object{"Name": "Work", "Tags": map[string]any{"Count": float64(3), "_ref": "https://rally.test" + rally.WSAPI + "hierarchicalrequirement/1/Tags"}}, nil), d.values())
	if !d.Conflict || !d.dirty() {
		t.Fatal("unexpected server membership was silently accepted")
	}
}

func TestV61CollectionLoadingIsCompleteOrKeepsBaseline(t *testing.T) {
	for _, outcome := range []string{"complete", "short", "duplicate", "foreign", "large", "replaced"} {
		t.Run(outcome, func(t *testing.T) {
			var base string
			s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if strings.Contains(r.URL.Query().Get("fetch"), "FormattedID") {
					t.Error("unsupported Tag field requested")
				}
				start, total := 1, 3
				if r.URL.Query().Get("start") == "3" {
					start = 3
				}
				if outcome == "large" {
					total = maxPropertyReferences + 1
				}
				rows := []rally.Object{{"_ref": base + rally.WSAPI + "tag/1", "Name": "One"}, {"_ref": base + rally.WSAPI + "tag/2", "Name": "Two"}}
				if start == 3 {
					rows = []rally.Object{{"_ref": base + rally.WSAPI + "tag/3", "Name": "Three"}}
				}
				if start == 3 && outcome == "short" {
					rows = nil
				}
				if start == 3 && outcome == "duplicate" {
					rows[0]["_ref"] = base + rally.WSAPI + "tag/1"
				}
				if outcome == "foreign" {
					rows[0]["_ref"] = "https://elsewhere.test" + rally.WSAPI + "tag/1"
				}
				json.NewEncoder(w).Encode(map[string]any{"QueryResult": map[string]any{"Results": rows, "StartIndex": start, "PageSize": 200, "TotalResultCount": total}})
			}))
			defer s.Close()
			base = s.URL
			a := presetApp(t)
			a.rallyClient, _ = rally.New(s.URL, "test", nil)
			d := makeDetail(rally.Object{"Name": "Work", "Tags": map[string]any{"Count": float64(3), "_ref": base + rally.WSAPI + "hierarchicalrequirement/1/Tags"}}, "HierarchicalRequirement", false)
			f := rally.Field{Name: "Tags", AttributeType: "COLLECTION"}
			mergeSchemaEditors(d, []rally.Field{f})
			if d.Editors["Tags"] != nil {
				t.Fatal("unloaded collection was writable")
			}
			a.loadPropertyCollection(d, f)
			if outcome == "replaced" {
				a.rallyClient, _ = rally.New(s.URL, "new", nil)
			}
			drain(t, a, func() bool { return !d.collectionsLoading() })
			if outcome == "complete" {
				refs, err := decodeCollectionRefs(text(d.Editors["Tags"]))
				if err != nil || len(refs) != 3 || d.dirty() {
					t.Fatal("full collection was not installed cleanly", refs, err)
				}
			} else if d.Editors["Tags"] != nil || d.Original.Count("Tags") != 3 || d.collectionEditors["Tags"].err == "" {
				t.Fatal("unsafe partial collection replaced the baseline", d.Original)
			}
		})
	}
}

func TestV61CollectionReloadComparesCompleteServerSelection(t *testing.T) {
	var base string
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if strings.HasSuffix(r.URL.Path, "/Tags") {
			json.NewEncoder(w).Encode(map[string]any{"QueryResult": map[string]any{"Results": []rally.Object{{"_ref": base + rally.WSAPI + "tag/1", "Name": "One"}, {"_ref": base + rally.WSAPI + "tag/3", "Name": "Three"}}, "TotalResultCount": 2, "StartIndex": 1}})
			return
		}
		fmt.Fprintf(w, `{"HierarchicalRequirement":{"_ref":%q,"ObjectID":1,"Name":"Work","LastUpdateDate":"two","Tags":{"_ref":%q,"Count":2}}}`, base+rally.WSAPI+"hierarchicalrequirement/1", base+rally.WSAPI+"hierarchicalrequirement/1/Tags")
	}))
	defer s.Close()
	base = s.URL
	a := presetApp(t)
	a.rallyClient, _ = rally.New(base, "test", nil)
	d := makeDetail(rally.Object{"_ref": base + rally.WSAPI + "hierarchicalrequirement/1", "Name": "Work", "LastUpdateDate": "one", "Tags": []any{map[string]any{"_ref": base + rally.WSAPI + "tag/1"}, map[string]any{"_ref": base + rally.WSAPI + "tag/2"}}}, "HierarchicalRequirement", false)
	mergeSchemaEditors(d, []rally.Field{{Name: "Tags", AttributeType: "COLLECTION"}})
	setText(d.Editors["Tags"], encodeCollectionRefs([]string{base + rally.WSAPI + "tag/1"}))
	v := newRallyView(rally.FindPage("teamboard"))
	v.Detail = d
	a.reloadDetail(v)
	drain(t, a, func() bool { return !d.Saving })
	if len(d.fieldConflicts) != 1 || d.fieldConflicts[0].Name != "Tags" {
		t.Fatal("collection changed without review", d.Error, d.fieldConflicts)
	}
	d.resolveFieldConflict("Tags", true)
	refs, err := decodeCollectionRefs(text(d.Editors["Tags"]))
	if err != nil || len(refs) != 2 || !strings.HasSuffix(refs[1], "tag/3") || d.dirty() {
		t.Fatal("server collection choice was incomplete", refs, err)
	}
}

func TestV61ColorAndCollectionControlsAtDisplayScales(t *testing.T) {
	for _, scale := range []float64{1, 1.5, 2} {
		a := presetApp(t)
		d := makeDetail(rally.Object{"Name": "Work", "Tags": []any{map[string]any{"_ref": rally.WSAPI + "tag/1", "Name": "Selected tag"}}}, "HierarchicalRequirement", false)
		colorField := rally.Field{Name: "DisplayColor", AttributeType: "STRING"}
		tagField := rally.Field{Name: "Tags", AttributeType: "COLLECTION"}
		mergeSchemaEditors(d, []rally.Field{colorField, tagField})
		var click image.Point
		h := desktop.NewHeadlessHarness(0, image.Pt(int(340*scale), int(600*scale)), func(w *desktop.Window) {
			if click != (image.Point{}) {
				m := &w.Input().Mouse
				m.Pos = click
				m.Buttons[mouse.ButtonLeft].Clicked = true
				m.Buttons[mouse.ButtonLeft].ClickedPos = click
			}
			a.detailProperty(w, d, colorField)
			a.detailProperty(w, d, tagField)
		})
		style := makeStyle(a.p, 13)
		style.Scale(scale)
		h.Master().SetStyle(style)
		h.Frame(false)
		for _, c := range h.Commands() {
			if c.Kind == command.RectFilledCmd && c.RectFilled.Color == (color.RGBA{R: 0x10, G: 0x7c, B: 0x1e, A: 255}) {
				click = image.Pt(c.Rect.X+2, c.Rect.Y+2)
			}
		}
		if click == (image.Point{}) {
			t.Fatal("missing Green swatch", scale)
		}
		h.Frame(false)
		if text(d.Editors["DisplayColor"]) != "#107c1e" {
			t.Fatal("color swatch did not select its WSAPI value", scale)
		}
		click = image.Point{}
		h.Frame(false)
		for _, c := range h.Commands() {
			if c.Kind == command.TextCmd && c.Text.String == "×" {
				click = image.Pt(c.Rect.X+2, c.Rect.Y+2)
			}
		}
		if click == (image.Point{}) {
			t.Fatal("missing collection remove action", scale)
		}
		h.Frame(false)
		changes, err := d.changes()
		if err != nil || len(changes["Tags"].([]any)) != 0 {
			t.Fatal("remove did not encode an empty collection", err, changes)
		}
		setText(d.Editors["DisplayColor"], "")
		d.Original["DisplayColor"] = "#107c1e"
		changes, err = d.changes()
		if err != nil || changes["DisplayColor"] != nil {
			t.Fatal("default color must clear to null", changes, err)
		}
	}
}
