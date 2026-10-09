package platform

import (
	"context"
	"encoding/base64"
	"encoding/binary"
	"fmt"
	"os/exec"
	"strings"
	"syscall"
	"time"
	"unicode/utf16"
)

func ChoosePath(save, dir bool) (string, error) {
	typ := "OpenFileDialog"
	if save {
		typ = "SaveFileDialog"
	}
	if dir {
		typ = "FolderBrowserDialog"
	}
	property := "FileName"
	if dir {
		property = "SelectedPath"
	}
	script := `Add-Type -AssemblyName System.Windows.Forms; $d = New-Object System.Windows.Forms.` + typ + `; if ($d.ShowDialog() -eq 'OK') { [Console]::OutputEncoding = [Text.Encoding]::UTF8; [Console]::Write($d.` + property + `) }`
	units := utf16.Encode([]rune(script))
	b := make([]byte, 2*len(units))
	for i, u := range units {
		binary.LittleEndian.PutUint16(b[2*i:], u)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Minute)
	defer cancel()
	c := exec.CommandContext(ctx, "powershell.exe", "-NoProfile", "-STA", "-EncodedCommand", base64.StdEncoding.EncodeToString(b))
	c.SysProcAttr = &syscall.SysProcAttr{HideWindow: true, CreationFlags: 0x08000000}
	out, e := c.Output()
	if e != nil {
		return "", fmt.Errorf("file dialog failed: %w", e)
	}
	return strings.TrimSpace(string(out)), nil
}
