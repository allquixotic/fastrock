package ui

import (
	"context"
	"fmt"
	"net/url"
	"slices"
	"strings"

	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/settings"
)

type rallyTypeMetadata struct {
	Fields   []rally.Field
	Workflow []rally.Object
}

func rallyTypeMatches(kind string, filter settings.RallyFilter) (bool, error) {
	if filter.Operator == "contains" {
		value := strings.ToLower(filter.Value)
		return strings.Contains(strings.ToLower(rallyKindLabel(kind)), value) || strings.Contains(strings.ToLower(kind), value), nil
	}
	canonical, valid := rally.CanonicalKind(filter.Value)
	if !valid {
		return false, fmt.Errorf("Choose a supported work item type.")
	}
	match := canonical == kind
	if filter.Operator == "is not" {
		match = !match
	}
	return match, nil
}

func (v *rallyView) queryTypes() []string {
	types := v.Spec.ArtifactTypes()
	for _, filter := range v.StructuredFilters {
		if filter.Field != "Type" {
			continue
		}
		types = slices.DeleteFunc(types, func(kind string) bool {
			match, err := rallyTypeMatches(kind, filter)
			return err != nil || !match
		})
	}
	return types
}

func (v *rallyView) objectKind(o rally.Object) string {
	if kind, valid := rally.CanonicalKind(o.String("_type")); valid {
		return kind
	}
	if u, err := url.Parse(o.String("_ref")); err == nil {
		path := strings.TrimPrefix(u.Path, rally.WSAPI)
		if i := strings.LastIndexByte(path, '/'); i > 0 {
			if kind, valid := rally.CanonicalKind(path[:i]); valid {
				return kind
			}
		}
	}
	return v.Spec.Kind
}

func (v *rallyView) metadataFor(o rally.Object) rallyTypeMetadata {
	if metadata, ok := v.TypeMetadata[v.objectKind(o)]; ok {
		return metadata
	}
	// A mixed view must not borrow another type's schema after an incomplete load.
	if len(v.TypeMetadata) > 0 {
		return rallyTypeMetadata{}
	}
	return rallyTypeMetadata{Fields: v.Fields, Workflow: v.Workflow}
}

func (v *rallyView) objectStateValue(o rally.Object, name string) (any, bool) {
	copy := *v
	copy.Spec.Kind = v.objectKind(o)
	metadata := v.metadataFor(o)
	if len(v.TypeMetadata) > 0 {
		index := slices.IndexFunc(metadata.Fields, func(f rally.Field) bool { return f.Name == v.stateField() })
		if index < 0 || metadata.Fields[index].ReadOnly || name == "Unspecified" && metadata.Fields[index].Required {
			return nil, false
		}
	}
	copy.Fields, copy.Workflow = metadata.Fields, metadata.Workflow
	return copy.stateValue(name)
}

func (v *rallyView) stateField() string {
	if len(v.Spec.ArtifactTypes()) > 0 {
		return "ScheduleState"
	}
	return rally.StateField(v.Spec.Kind)
}

func scheduleMetadata(all map[string]rallyTypeMetadata) {
	for kind, metadata := range all {
		metadata.Workflow = nil
		for _, field := range metadata.Fields {
			if field.Name == "ScheduleState" {
				for _, value := range field.AllowedValues {
					metadata.Workflow = append(metadata.Workflow, rally.Object{"Name": value})
				}
			}
		}
		all[kind] = metadata
	}
}

// Only fields writable for every selected type may drive shared controls.
// Allowed choices intersect; required/read-only restrictions accumulate.
func commonRallyMetadata(all []rallyTypeMetadata) rallyTypeMetadata {
	if len(all) == 0 {
		return rallyTypeMetadata{}
	}
	out := rallyTypeMetadata{Fields: cloneTransferFields(all[0].Fields), Workflow: cloneObjects(all[0].Workflow)}
	for _, metadata := range all[1:] {
		fields := out.Fields[:0]
		for _, field := range out.Fields {
			i := slices.IndexFunc(metadata.Fields, func(f rally.Field) bool {
				return f.Name == field.Name && f.AttributeType == field.AttributeType && f.ReferenceType == field.ReferenceType
			})
			if i < 0 {
				continue
			}
			next := metadata.Fields[i]
			field.Required = field.Required || next.Required
			field.ReadOnly = field.ReadOnly || next.ReadOnly
			field.AllowedValues = slices.DeleteFunc(field.AllowedValues, func(s string) bool { return !slices.Contains(next.AllowedValues, s) })
			fields = append(fields, field)
		}
		out.Fields = fields
		out.Workflow = slices.DeleteFunc(out.Workflow, func(o rally.Object) bool {
			return !slices.ContainsFunc(metadata.Workflow, func(next rally.Object) bool { return next.String("Name") == o.String("Name") })
		})
	}
	return out
}

func loadRallyTypeMetadata(ctx context.Context, c *rally.Client, kinds []string, workspace string) (map[string]rallyTypeMetadata, error) {
	out := make(map[string]rallyTypeMetadata, len(kinds))
	for _, kind := range kinds {
		fields, err := c.Fields(ctx, kind, workspace)
		if err != nil {
			return nil, fmt.Errorf("%s fields: %w", rallyKindLabel(kind), err)
		}
		states, err := c.Workflow(ctx, kind, workspace, fields)
		if err != nil {
			return nil, fmt.Errorf("%s states: %w", rallyKindLabel(kind), err)
		}
		out[kind] = rallyTypeMetadata{fields, states}
	}
	return out, nil
}

func metadataList(all map[string]rallyTypeMetadata, kinds []string) []rallyTypeMetadata {
	out := make([]rallyTypeMetadata, 0, len(kinds))
	for _, kind := range kinds {
		out = append(out, all[kind])
	}
	return out
}

func artifactBadge(kind string) string {
	switch kind {
	case "HierarchicalRequirement":
		return "US"
	case "Defect":
		return "D"
	case "TestSet":
		return "TS"
	case "DefectSuite":
		return "DS"
	default:
		return "•"
	}
}
