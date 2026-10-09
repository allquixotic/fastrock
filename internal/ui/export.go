package ui

import (
	"context"
	"encoding/csv"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"github.com/allquixotic/fastrock/internal/workspace"
)

// Commit a complete export atomically; cancellation or a page failure leaves
// any existing destination intact, never a misleading partial report.
func exportFile(path string, write func(*os.File) error) error {
	f, err := os.CreateTemp(filepath.Dir(path), ".fastrock-export-*")
	if err != nil {
		return err
	}
	defer os.Remove(f.Name())
	err = write(f)
	if err == nil {
		err = f.Sync()
	}
	if closeErr := f.Close(); err == nil {
		err = closeErr
	}
	if err == nil {
		err = os.Rename(f.Name(), path)
	}
	return err
}
func (a *App) exportChat(c *workspace.Conversation) {
	client := a.client
	if client == nil {
		a.toast = "Reconnect Codex before exporting full conversation history"
		return
	}
	id, title := c.ID, c.Title
	a.choosePath(true, false, func(path string) {
		a.work(func() {
			ctx, cancel := context.WithTimeout(a.ctx, 5*time.Minute)
			defer cancel()
			err := exportFile(path, func(f *os.File) error {
				return writeChatHistory(ctx, client, id, title, f, 64<<20)
			})
			a.post(func() {
				if err != nil {
					a.report(err)
				} else {
					a.toast = "Exported full conversation history"
				}
			})
		})
	})
}
func (a *App) exportRally(v *rallyView) {
	if a.rallyClient == nil || v.Exporting {
		return
	}
	if _, err := a.structuredFilterExpression(v); err != nil {
		v.Error = err.Error()
		return
	}
	q, kind, c := a.rallyQuery(v), v.Spec.QueryKind(), a.rallyClient
	columns := append([]string(nil), v.Columns...)
	q.Fetch = strings.Join(columns, ",") + ",ObjectID"
	q.PageSize = 200
	q.Start = 1
	a.choosePath(true, false, func(path string) {
		if v.Closed || v.Exporting {
			return
		}
		ctx, cancel := context.WithTimeout(a.ctx, 5*time.Minute)
		v.ExportCancel = cancel
		v.Exporting = true
		a.work(func() {
			defer cancel()
			count := 0
			err := exportFile(path, func(f *os.File) error {
				writer := csv.NewWriter(f)
				if err := writer.Write(columns); err != nil {
					return err
				}
				defer writer.Flush()
				for q.Start <= 100000 {
					page, err := c.Query(ctx, kind, q)
					if err != nil {
						return err
					}
					for _, o := range page.Results {
						row := make([]string, len(columns))
						for i, k := range columns {
							cell := o.String(k)
							if _, ok := o[k].(map[string]any); ok {
								if k == "Tasks" || k == "Discussion" {
									cell = strconv.Itoa(o.Count(k))
								}
							}
							if len(cell) > 0 && strings.ContainsAny(cell[:1], "=+-@\t\r") {
								cell = "'" + cell
							}
							row[i] = cell
						}
						if err = writer.Write(row); err != nil {
							return err
						}
						count++
					}
					writer.Flush()
					if err = writer.Error(); err != nil {
						return err
					}
					q.Start += len(page.Results)
					if q.Start > page.Total {
						return nil
					}
					if len(page.Results) == 0 {
						return fmt.Errorf("Rally returned an empty page before the export completed")
					}
				}
				return fmt.Errorf("export exceeds 100,000 items; narrow the query")
			})
			a.post(func() {
				v.Exporting = false
				v.ExportCancel = nil
				if err != nil {
					a.report(err)
				} else {
					a.toast = fmt.Sprintf("Exported all %d matching work items", count)
				}
			})
		}, func() { v.Exporting = false; cancel() })
	})
}
