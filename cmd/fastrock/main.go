package main

import (
	"context"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"os/signal"
	"runtime/debug"
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
	popout := flag.String("popout", "", "Internal window transfer ticket")
	flag.Parse()
	if *popout != "" {
		if decoded, err := hex.DecodeString(*popout); err != nil || len(decoded) != 32 {
			platform.ShowError("Invalid window transfer ticket")
			return 1
		}
	}
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
	address, token := os.Getenv("FASTROCK_BROKER"), os.Getenv("FASTROCK_BROKER_TOKEN")
	var broker *codex.Broker
	if *popout == "" {
		client, err := codex.Start(ctx)
		if err != nil {
			platform.ShowError(err.Error())
			return 1
		}
		broker, err = codex.NewBroker(client)
		if err != nil {
			client.Close()
			platform.ShowError(err.Error())
			return 1
		}
		broker.SetStarter(func() (*codex.Client, error) { return codex.Start(ctx) })
		address, token = broker.Address(), broker.Token()
	}
	client, err := codex.Dial(ctx, address, token)
	if err != nil {
		if broker != nil {
			broker.Close()
		}
		platform.ShowError(err.Error())
		return 1
	}
	code := ui.Run(ctx, store, prefs, ui.Connection{Client: client, Address: address, Token: token, Ticket: *popout})
	if broker != nil {
		debug.FreeOSMemory()
		broker.ReportServiceMemory(platform.ResidentMemory())
		go func() {
			ticker := time.NewTicker(10 * time.Second)
			defer ticker.Stop()
			for {
				select {
				case <-broker.Done():
					return
				case <-ticker.C:
					broker.ReportServiceMemory(platform.ResidentMemory())
				}
			}
		}()
		broker.Wait()
	}
	return code
}
