package ui

import (
	"context"
	"errors"
	"fmt"
)

type settingsCaller interface {
	Call(context.Context, string, any, any) error
}

// Bound pagination and reject repeated cursors instead of silently showing only
// the first page or looping forever against a misbehaving server.
func settingsPayload(ctx context.Context, c settingsCaller, page, method string, params any, cwd string) (any, error) {
	if page != "Features" && page != "MCP servers" {
		var data any
		err := c.Call(ctx, method, params, &data)
		return data, err
	}
	p := map[string]any{}
	for k, v := range object(params) {
		p[k] = v
	}
	p["limit"] = 100
	if page == "MCP servers" {
		p["detail"] = "toolsAndAuthOnly"
	}
	data := map[string]any{"data": []any{}}
	seen := map[string]bool{}
	var listErr error
	for count := 0; count < 100; count++ {
		var response map[string]any
		if err := c.Call(ctx, method, p, &response); err != nil {
			listErr = err
			break
		}
		values, _ := response["data"].([]any)
		data["data"] = append(data["data"].([]any), values...)
		cursor := str(response, "nextCursor")
		if cursor == "" {
			break
		}
		if seen[cursor] {
			listErr = fmt.Errorf("%s returned a repeated pagination cursor", method)
			break
		}
		seen[cursor] = true
		p["cursor"] = cursor
		if count == 99 {
			listErr = fmt.Errorf("%s exceeded the 100-page limit; showing the loaded entries", method)
		}
	}
	if page == "MCP servers" {
		var config map[string]any
		err := c.Call(ctx, "config/read", map[string]any{"includeLayers": true, "cwd": cwd}, &config)
		data["_configuration"] = config
		listErr = errors.Join(listErr, err)
	}
	return data, listErr
}
