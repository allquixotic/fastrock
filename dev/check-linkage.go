//go:build ignore

// go run dev/check-linkage.go path/to/fastrock[.exe]
package main

import (
	"debug/macho"
	"debug/pe"
	"fmt"
	"os"
	"strings"
)

func main() {
	if len(os.Args) != 2 {
		panic("usage: check-linkage executable")
	}
	path := os.Args[1]
	var libraries []string
	if strings.HasSuffix(strings.ToLower(path), ".exe") {
		f, err := pe.Open(path)
		must(err)
		defer f.Close()
		// debug/pe.ImportedLibraries is unimplemented; use the import table.
		symbols, err := f.ImportedSymbols()
		must(err)
		seen := map[string]bool{}
		for _, symbol := range symbols {
			if i := strings.LastIndex(symbol, ":"); i >= 0 && !seen[symbol[i+1:]] {
				library := symbol[i+1:]
				seen[library] = true
				libraries = append(libraries, library)
			}
		}
	} else {
		f, err := macho.Open(path)
		must(err)
		defer f.Close()
		libraries, err = f.ImportedLibraries()
		must(err)
	}
	if len(libraries) == 0 {
		panic("could not read executable import table")
	}
	for _, library := range libraries {
		name := strings.ToLower(library)
		if strings.Contains(name, "fltk") || strings.Contains(name, "libgcc") || strings.Contains(name, "libstdc++") || strings.Contains(name, "libwinpthread") {
			panic("unexpected dynamic dependency: " + library)
		}
	}
	fmt.Printf("Static FLTK linkage verified: %s (%d system imports)\n", path, len(libraries))
}
func must(err error) {
	if err != nil {
		panic(err)
	}
}
