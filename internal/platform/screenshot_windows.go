package platform

import (
	"encoding/base64"
	"encoding/binary"
	"os"
	"os/exec"
	"strconv"
	"syscall"
	"unicode/utf16"
)

func Screenshot(path string) error {
	script := `Add-Type -AssemblyName System.Drawing; Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public class ShotWin { [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h,out R r); [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h); [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags); public struct R { public int L,T,Rt,B; } }'; $p=Get-Process -Id $env:FASTROCK_SHOT_PID; $h=$p.MainWindowHandle; [ShotWin]::SetWindowPos($h,[IntPtr](-1),0,0,0,0,3) | Out-Null; [ShotWin]::SetForegroundWindow($h) | Out-Null; Start-Sleep -Milliseconds 350; $r=New-Object ShotWin+R; [ShotWin]::GetWindowRect($h,[ref]$r) | Out-Null; $b=New-Object Drawing.Bitmap(($r.Rt-$r.L),($r.B-$r.T)); $g=[Drawing.Graphics]::FromImage($b); $g.CopyFromScreen($r.L,$r.T,0,0,$b.Size); $b.Save($env:FASTROCK_SHOT_PATH,[Drawing.Imaging.ImageFormat]::Png); $g.Dispose(); $b.Dispose(); [ShotWin]::SetWindowPos($h,[IntPtr](-2),0,0,0,0,3) | Out-Null`
	units := utf16.Encode([]rune(script))
	b := make([]byte, 2*len(units))
	for i, u := range units {
		binary.LittleEndian.PutUint16(b[2*i:], u)
	}
	c := exec.Command("powershell.exe", "-NoProfile", "-EncodedCommand", base64.StdEncoding.EncodeToString(b))
	c.Env = append(os.Environ(), "FASTROCK_SHOT_PID="+strconv.Itoa(os.Getpid()), "FASTROCK_SHOT_PATH="+path)
	c.SysProcAttr = &syscall.SysProcAttr{HideWindow: true, CreationFlags: 0x08000000}
	return c.Run()
}
