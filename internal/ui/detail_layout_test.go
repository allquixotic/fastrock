//go:build nucular_headless

package ui

import (
	"fmt"
	"image"
	"slices"
	"strings"
	"testing"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/command"
	"github.com/allquixotic/fastrock/internal/rally"
)

func TestV57DetailPaneSizing(t *testing.T) {
	for _, scale := range []float64{1, 1.5, 2} {
		for _, width := range []int{400, 700, 1100, 1600} {
			content, sidebar, stacked := detailPaneWidths(int(float64(width)*scale), scale, int(8*scale))
			if sidebar < int(300*scale) || stacked != (content < int(360*scale)) {
				t.Fatal(width, scale, content, sidebar, stacked)
			}
			if !stacked && width >= 1100 {
				ratio := float64(content) / float64(content+sidebar)
				if ratio < .70 || ratio > .73 {
					t.Fatal("content split is not approximately 72/28", width, scale, ratio)
				}
			}
		}
	}
}

func TestV57DetailSchemaOrderAndInlineRichFields(t *testing.T) {
	d := makeDetail(rally.Object{"Name": "Defect", "Description": "<p>Description</p>", "c_CustomHTML": "<p>Custom</p>", "StepsToReproduce": "<p>Reproduce</p>", "TaskStatus": "INPROGRESS", "Tasks": map[string]any{"Count": 3}}, "Defect", false)
	mergeSchemaEditors(d, []rally.Field{
		{Name: "c_CustomHTML", DisplayName: "Custom explanation", AttributeType: "TEXT"},
		{Name: "StepsToReproduce", DisplayName: "Steps to Reproduce", AttributeType: "TEXT"},
		{Name: "Release", DisplayName: "FY Quarter", AttributeType: "OBJECT"},
		{Name: "c_Choice", DisplayName: "Business priority", AttributeType: "RATING", AllowedValues: []string{"High", "Low"}},
		{Name: "Project", DisplayName: "Team", AttributeType: "OBJECT"},
		{Name: "Owner", DisplayName: "Accountable", AttributeType: "OBJECT"},
		{Name: "State", DisplayName: "Defect State", AttributeType: "RATING"},
		{Name: "PlanEstimate", AttributeType: "QUANTITY"},
		{Name: "Priority", AttributeType: "RATING"},
		{Name: "Severity", AttributeType: "RATING"},
		{Name: "Parent", AttributeType: "OBJECT"},
		{Name: "CreationDate", AttributeType: "DATE", ReadOnly: true},
	})
	var properties, rich []string
	for _, f := range d.propertyFields() {
		properties = append(properties, f.Name)
	}
	want := []string{"Owner", "Project", "State", "PlanEstimate", "Priority", "TaskRollup", "Parent", "Severity", "Release", "c_Choice", "CreationDate"}
	if !slices.Equal(properties, want) {
		t.Fatal("schema/reference property order", properties)
	}
	for _, f := range d.richFields() {
		rich = append(rich, f.Name)
	}
	if !slices.Equal(rich, []string{"Description", "StepsToReproduce", "Notes", "c_CustomHTML"}) {
		t.Fatal("rich fields remain hidden under More fields", rich)
	}
	if f, _ := d.schemaField("Project"); detailCaption(f) != "Team" {
		t.Fatal("workspace display name ignored")
	}
}

func TestV57DetailRichLayoutAtEveryScale(t *testing.T) {
	for _, scale := range []float64{1, 1.5, 2} {
		for _, width := range []int{550, 1300} {
			t.Run(fmt.Sprintf("%g/%d", scale, width), func(t *testing.T) {
				a := presetApp(t)
				d := makeDetail(rally.Object{"Name": "Story", "Description": "<p>Body</p>", "c_Inline": "<p>Inline body</p>"}, "HierarchicalRequirement", false)
				mergeSchemaEditors(d, []rally.Field{{Name: "c_Inline", DisplayName: "Inline custom", AttributeType: "TEXT"}})
				h := nucular.NewHeadlessHarness(0, image.Pt(int(float64(width)*scale), int(2200*scale)), func(w *nucular.Window) { a.detailFields(w, d) })
				style := makeStyle(a.p, 13)
				style.Scale(scale)
				h.Master().SetStyle(style)
				h.Frame(false)
				inline, description := false, false
				for _, c := range h.Commands() {
					if c.Kind == command.RectFilledCmd && c.Rect.H >= int(360*scale)-2 && c.Rect.H <= int(360*scale)+2 {
						description = true
					}
					if c.Kind == command.TextCmd && c.Text.String == "Inline custom" {
						inline = true
					}
				}
				if !inline {
					t.Fatal("custom rich field did not render inline")
				}
				if !description {
					t.Fatal("description editor is shorter than 360 logical pixels")
				}
			})
		}
	}
}

func TestV57CompactRichControlsStayVisible(t *testing.T) {
	for _, scale := range []float64{1, 1.5, 2} {
		a := presetApp(t)
		r := newRichEditor("<p>Text</p>")
		h := nucular.NewHeadlessHarness(0, image.Pt(int(360*scale), int(700*scale)), func(w *nucular.Window) { a.richField(w, "Description", r, 360) })
		style := makeStyle(a.p, 13)
		style.Scale(scale)
		h.Master().SetStyle(style)
		h.Frame(false)
		labels := map[string]bool{}
		for _, c := range h.Commands() {
			if c.Kind == command.TextCmd && strings.Contains(" Edit Preview HTML Paragraph Link ", " "+c.Text.String+" ") {
				labels[c.Text.String] = true
				if c.Rect.X < 0 || c.Rect.X+c.Rect.W > int(360*scale) {
					t.Fatal("rich action overflowed the narrow content column", scale, c.Text.String, c.Rect)
				}
			}
		}
		for _, name := range []string{"Edit", "Preview", "HTML", "Link", "Paragraph"} {
			if !labels[name] {
				t.Fatal("missing rich action", scale, name, labels)
			}
		}
	}
}
