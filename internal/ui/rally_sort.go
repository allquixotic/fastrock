package ui

import (
	"cmp"
	"github.com/allquixotic/fastrock/internal/rally"
	"strconv"
	"strings"
	"time"
)

func compareRally(a, b rally.Object, key string, fields []rally.Field) int {
	if key == "Rank" || key == "DragAndDropRank" {
		return strings.Compare(a.String("DragAndDropRank"), b.String("DragAndDropRank"))
	}
	if a[key] == nil || b[key] == nil {
		if a[key] == nil && b[key] == nil {
			return 0
		}
		if a[key] == nil {
			return -1
		}
		return 1
	}
	kind := ""
	for _, field := range fields {
		if field.Name == key {
			kind = field.AttributeType
			break
		}
	}
	if kind == "INTEGER" || kind == "DECIMAL" || kind == "QUANTITY" || strings.Contains(",PlanEstimate,Estimate,ToDo,Actuals,ObjectID,", ","+key+",") {
		x, ex := strconv.ParseFloat(a.String(key), 64)
		y, ey := strconv.ParseFloat(b.String(key), 64)
		if ex == nil && ey == nil {
			return cmp.Compare(x, y)
		}
	}
	if kind == "DATE" || strings.HasSuffix(key, "Date") {
		x, ex := time.Parse(time.RFC3339, a.String(key))
		y, ey := time.Parse(time.RFC3339, b.String(key))
		if ex == nil && ey == nil {
			return x.Compare(y)
		}
	}
	return strings.Compare(strings.ToLower(a.String(key)), strings.ToLower(b.String(key)))
}
