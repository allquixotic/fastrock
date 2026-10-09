package platform

import "os/exec"

func ShowError(message string) {
	_ = exec.Command("osascript", "-e", `on run argv
        display alert "Fastrock" message (item 1 of argv) as critical
    end run`, message).Run()
}
