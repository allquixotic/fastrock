package ui

import (
	"context"
	"github.com/allquixotic/fastrock/internal/rally"
	"strings"
	"time"
)

func (a *App) loadViewMetadata(v *rallyView) {
	c := a.rallyClient
	if c == nil {
		return
	}
	workspace := a.prefs.RallyWorkspace
	mixed := len(v.Spec.ArtifactTypes()) > 0
	kinds := v.Spec.ArtifactTypes()
	if len(kinds) == 0 {
		kinds = []string{v.Spec.Kind}
	}
	signature := v.signature
	v.metadataSignature = signature
	a.work(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 45*time.Second)
		defer cancel()
		metadata, err := loadRallyTypeMetadata(ctx, c, kinds, workspace)
		if mixed {
			scheduleMetadata(metadata)
		}
		common := commonRallyMetadata(metadataList(metadata, kinds))
		var states []rally.Object
		for _, kind := range kinds {
			for _, state := range metadata[kind].Workflow {
				found := false
				for _, old := range states {
					found = found || old.String("Name") == state.String("Name")
				}
				if !found {
					states = append(states, state)
				}
			}
		}
		a.post(func() {
			if v.Closed || v.metadataSignature != signature || a.rallyClient != c {
				return
			}
			if err != nil {
				v.WorkflowError = err.Error()
				return
			}
			v.WorkflowError = ""
			v.Fields = common.Fields
			v.TypeMetadata = metadata
			v.filterValid = false
			v.Workflow = states
			v.cardSource = nil
			v.boardPrepared = false
		})
	}, func() { v.WorkflowError = errWorkQueueFull.Error() })
}

func (a *App) rehydrateDetail(v *rallyView) {
	d, c := v.Detail, a.rallyClient
	if d == nil || c == nil {
		return
	}
	if d.selection != nil {
		a.rehydrateSelection(v, d)
		return
	}
	d.SchemaLoading = true
	workspace := a.prefs.RallyWorkspace
	a.work(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 45*time.Second)
		defer cancel()
		fields, err := c.Fields(ctx, d.Kind, workspace)
		var states []rally.Object
		if err == nil {
			states, err = c.Workflow(ctx, d.Kind, workspace, fields)
		}
		a.post(func() {
			if v.Closed || v.Detail != d {
				return
			}
			d.SchemaLoading = false
			if a.rallyClient != c {
				d.SchemaError = "Rally connection changed. Reopen the editor to load field metadata."
				return
			}
			if err != nil {
				d.SchemaError = err.Error()
				return
			}
			mergeSchemaEditors(d, fields)
			d.setStates(states)
			d.SchemaError = ""
		})
	}, func() { d.SchemaLoading = false; d.SchemaError = errWorkQueueFull.Error() })
}

func rallyFetch(v *rallyView) string {
	fields := strings.Split(cardFields, ",")
	switch v.Spec.Kind {
	case "User":
		fields = []string{"ObjectID", "DisplayName", "UserName"}
	case "Project", "Workspace":
		fields = []string{"ObjectID", "Name", "State", "Owner"}
	case "Iteration", "Release":
		fields = []string{"ObjectID", "Name", "StartDate", "EndDate", "Project", "State"}
	case "TestCase":
		fields = []string{"ObjectID", "FormattedID", "Name", "LastVerdict", "Method", "Owner", "Project", "LastUpdateDate"}
	}
	fields = append(fields, v.Columns...)
	fields = append(fields, v.Sort, v.Group)
	if v.Spec.Kind != "User" && v.Spec.Kind != "Project" && v.Spec.Kind != "Workspace" && v.Spec.Kind != "Iteration" && v.Spec.Kind != "Release" {
		fields = append(fields, "VersionId", "Discussion", "DisplayColor")
		if strings.HasPrefix(v.Spec.Kind, "PortfolioItem/") {
			fields = append(fields, "PlannedStartDate", "PlannedEndDate")
		}
	}
	seen := map[string]bool{}
	out := make([]string, 0, len(fields))
	for _, field := range fields {
		if field == "Rank" {
			field = "DragAndDropRank"
		}
		if field != "" && field != "None" && !seen[field] {
			seen[field] = true
			out = append(out, field)
		}
	}
	return strings.Join(out, ",")
}

func (v *rallyView) selectItem(o rally.Object, selected bool) {
	v.selectionRevision++
	ref := o.String("_ref")
	if !selected {
		delete(v.Selected, ref)
		delete(v.SelectedItems, ref)
		return
	}
	if v.Selected == nil {
		v.Selected = map[string]bool{}
	}
	if v.SelectedItems == nil {
		v.SelectedItems = map[string]rally.Object{}
	}
	v.Selected[ref] = true
	v.SelectedItems[ref] = o.Clone()
}
func (v *rallyView) stateNames() []string {
	names := make([]string, 0, len(v.Workflow))
	for _, o := range v.Workflow {
		names = append(names, o.String("Name"))
	}
	return names
}
func (v *rallyView) stateValue(name string) (any, bool) {
	if name == "Unspecified" {
		return nil, true
	}
	for _, o := range v.Workflow {
		if o.String("Name") == name {
			if strings.HasPrefix(strings.ToLower(v.Spec.Kind), "portfolioitem") {
				return o.String("_ref"), o.String("_ref") != ""
			}
			return name, true
		}
	}
	return nil, false
}
