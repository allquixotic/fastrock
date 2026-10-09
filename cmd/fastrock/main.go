package main

import (
	"context"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"os/signal"
	"path/filepath"
	"runtime/debug"
	"time"

	"github.com/allquixotic/fastrock/internal/buildinfo"
	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/platform"
	"github.com/allquixotic/fastrock/internal/settings"
	"github.com/allquixotic/fastrock/internal/ui"
	"github.com/allquixotic/fastrock/internal/update"
)

func main() { os.Exit(run()) }
func run() int {
	doctor := flag.Bool("doctor", false, "Check installed Codex and its protocol without opening a window")
	version := flag.Bool("version", false, "Print Fastrock version")
	popout := flag.String("popout", "", "Internal window transfer ticket")
	applyUpdate := flag.String("apply-update", "", "Internal deferred update job")
	flag.Parse()
	if *applyUpdate != "" {
		if e := update.Apply(*applyUpdate); e != nil {
			fmt.Fprintln(os.Stderr, e)
			return 1
		}
		return 0
	}
	if *popout != "" {
		if decoded, err := hex.DecodeString(*popout); err != nil || len(decoded) != 32 {
			platform.ShowError("Invalid window transfer ticket")
			return 1
		}
	}
	if *version {
		fmt.Println("Fastrock " + buildinfo.Version)
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
	startupNotice := ""
	if *popout == "" && update.IsRelease(buildinfo.Version) {
		exe, err := os.Executable()
		if err == nil {
			relaunched, installErr := update.PrepareInstallation(exe)
			if installErr != nil {
				startupNotice = "Per-user installation: " + installErr.Error()
			}
			if relaunched && installErr == nil {
				return 0
			}
		}
	}
	store, e := settings.Open()
	if e != nil {
		platform.ShowError(e.Error())
		return 1
	}
	if data, err := os.ReadFile(filepath.Join(store.Dir, "updates", "last-update.json")); err == nil {
		var last update.Status
		if json.Unmarshal(data, &last) == nil && last.State == "error" {
			startupNotice = last.Message
		}
	}
	prefs, e := store.Load()
	if e != nil {
		platform.ShowError(e.Error())
		return 1
	}
	address, token := os.Getenv("FASTROCK_BROKER"), os.Getenv("FASTROCK_BROKER_TOKEN")
	var broker *codex.Broker
	var updater *update.Manager
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
		exe, _ := os.Executable()
		updater = update.New(buildinfo.Version, filepath.Join(store.Dir, "updates"), exe, func(s update.Status) { broker.UpdateStatus(s) })
		broker.SetUpdateHandler(func(check bool) any {
			if check {
				updater.Check(ctx)
			}
			return updater.Status()
		})
		updater.Check(ctx)
	}
	client, err := codex.Dial(ctx, address, token)
	if err != nil {
		if broker != nil {
			broker.Close()
		}
		platform.ShowError(err.Error())
		return 1
	}
	code := ui.Run(ctx, store, prefs, ui.Connection{Client: client, Address: address, Token: token, Ticket: *popout, Notice: startupNotice})
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
		updater.Close()
		if job := updater.Pending(); job != "" {
			if err := update.LaunchHelper(job); err != nil {
				platform.ShowError("The update is downloaded but could not start: " + err.Error())
			}
		}
	}
	return code
}
