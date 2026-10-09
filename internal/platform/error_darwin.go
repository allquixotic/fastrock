package platform

func ShowError(message string) {
	_ = Command("osascript", "-e", `on run argv
        display alert "Fastrock" message (item 1 of argv) as critical
    end run`, message).Run()
}
