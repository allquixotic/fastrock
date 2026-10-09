package main

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"os/signal"
	"time"

	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/platform"
	"github.com/allquixotic/fastrock/internal/settings"
	"github.com/allquixotic/fastrock/internal/ui"
)

func main() { os.Exit(run()) }
func run() int {
	doctor := flag.Bool("doctor", false, "Check installed Codex and its protocol without opening a window")
	version := flag.Bool("version", false, "Print Fastrock version")
	flag.Parse()
	if *version {
		fmt.Println("Fastrock 1.0.0")
		return 0
	}
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt)
	defer stop()
	if *doctor {
		ctx, cancel := context.WithTimeout(ctx, 45*time.Second)
		defer cancel()
		c, e := codex.Start(ctx)
		if e != nil {
			fmt.Fprintln(os.Stderr, e)
			return 1
		}
		defer c.Close()
		cwd, _ := os.Getwd()
		catalog, e := c.Catalog(ctx, cwd)
		if e != nil {
			fmt.Fprintln(os.Stderr, e)
			return 1
		}
		result := map[string]any{"codex": c.Version, "model": catalog.DefaultModel(), "models": catalog.Models, "protocol": "compatible"}
		b, _ := json.MarshalIndent(result, "", "  ")
		fmt.Println(string(b))
		return 0
	}
	store, e := settings.Open()
	if e != nil {
		platform.ShowError(e.Error())
		return 1
	}
	prefs, e := store.Load()
	if e != nil {
		platform.ShowError(e.Error())
		return 1
	}
	return ui.Run(ctx, store, prefs)
}
