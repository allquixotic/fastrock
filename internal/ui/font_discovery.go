package ui

import (
	"context"
	"encoding/binary"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/allquixotic/fastrock/internal/desktop/font"
	"golang.org/x/image/font/sfnt"
)

// Candidate discovery runs once before starting the UI. Linux fontconfig is
// bounded and optional; embedded Go faces cover missing/unsupported fonts.
func fontPaths(goos, windowsRoot string, v fontVariant) []string {
	if goos == "windows" {
		if windowsRoot == "" {
			windowsRoot = `C:\Windows`
		}
		names := []string{"segoeui.ttf", "segoeuib.ttf", "segoeuii.ttf", "segoeuiz.ttf", "consola.ttf", "consolab.ttf", "consolai.ttf", "consolaz.ttf"}
		return []string{filepath.Join(windowsRoot, "Fonts", names[v])}
	}
	if goos == "darwin" {
		if v >= monoFont {
			names := []string{"Courier New.ttf", "Courier New Bold.ttf", "Courier New Italic.ttf", "Courier New Bold Italic.ttf"}
			return []string{"/System/Library/Fonts/Menlo.ttc", filepath.Join("/System/Library/Fonts/Supplemental", names[v%4])}
		}
		names := []string{"Arial.ttf", "Arial Bold.ttf", "Arial Italic.ttf", "Arial Bold Italic.ttf"}
		return []string{filepath.Join("/System/Library/Fonts/Supplemental", names[v])}
	}
	family := "sans-serif"
	if v >= monoFont {
		family = "monospace"
	}
	styles := []string{"Regular", "Bold", "Italic", "Bold Italic"}
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	data, err := exec.CommandContext(ctx, "fc-match", "-f", "%{file}", family+":style="+styles[v%4]).Output()
	if err == nil && strings.TrimSpace(string(data)) != "" {
		return []string{strings.TrimSpace(string(data))}
	}
	return nil
}

func loadFontFile(path string, v fontVariant) ([]byte, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	if len(data) > 64<<20 {
		return nil, errors.New("font exceeds size limit")
	}
	if len(data) >= 4 && string(data[:4]) == "ttcf" {
		data, err = collectionFont(data, v)
		if err != nil {
			return nil, err
		}
	}
	parsed, err := sfnt.Parse(data)
	if err != nil {
		return nil, err
	}
	name, err := parsed.Name(nil, sfnt.NameIDSubfamily)
	if err != nil || !fontStyleMatches(name, v) {
		return nil, errors.New("font has no matching weight or slant")
	}
	face, err := font.NewFace(data, 13)
	if err != nil {
		return nil, err
	}
	_ = face.Face.Close()
	return data, nil
}

// FreeType's Go parser cannot select a TTC member. Copy that member's tables
// into a standalone TrueType face, retaining its real metrics and glyphs.
func collectionFont(data []byte, v fontVariant) ([]byte, error) {
	collection, err := sfnt.ParseCollection(data)
	if err != nil {
		return nil, err
	}
	index := -1
	for i := 0; i < collection.NumFonts(); i++ {
		f, e := collection.Font(i)
		if e != nil {
			continue
		}
		name, e := f.Name(nil, sfnt.NameIDSubfamily)
		if e != nil {
			continue
		}
		if fontStyleMatches(name, v) {
			index = i
			break
		}
	}
	if index < 0 || 12+4*(index+1) > len(data) {
		return nil, errors.New("font collection has no matching style")
	}
	at := int(binary.BigEndian.Uint32(data[12+4*index:]))
	if at < 0 || at > len(data)-12 || binary.BigEndian.Uint32(data[at:]) != 0x00010000 {
		return nil, errors.New("unsupported collection face")
	}
	n := int(binary.BigEndian.Uint16(data[at+4:]))
	if n > 4096 || 12+16*n > len(data)-at {
		return nil, errors.New("invalid font table directory")
	}
	out := append([]byte(nil), data[at:at+12+16*n]...)
	for i := 0; i < n; i++ {
		record := out[12+16*i : 12+16*(i+1)]
		offset, size := int(binary.BigEndian.Uint32(record[8:])), int(binary.BigEndian.Uint32(record[12:]))
		if offset < 0 || size < 0 || offset > len(data)-size {
			return nil, errors.New("invalid font table range")
		}
		binary.BigEndian.PutUint32(record[8:], uint32(len(out)))
		if size > (64<<20)-len(out)-3 {
			return nil, errors.New("font tables exceed size limit")
		}
		out = append(out, data[offset:offset+size]...)
		for len(out)%4 != 0 {
			out = append(out, 0)
		}
	}
	return out, nil
}

func fontStyleMatches(name string, v fontVariant) bool {
	name = strings.ToLower(name)
	bold := strings.Contains(name, "bold") || strings.Contains(name, "heavy") || strings.Contains(name, "demi")
	italic := strings.Contains(name, "italic") || strings.Contains(name, "oblique")
	return bold == (v%4 == boldFont || v%4 == boldItalicFont) && italic == (v%4 == italicFont || v%4 == boldItalicFont)
}
