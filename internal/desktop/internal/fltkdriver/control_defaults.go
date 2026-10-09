//go:build !fltk_headless

package fltkdriver

// FLTK exports this default, but go-fltk does not yet wrap it. Set it before
// constructing an input; all accesses happen on the FLTK event thread.

// extern int FL_NORMAL_SIZE;
import "C"

func setInputFontSize(size int) { C.FL_NORMAL_SIZE = C.int(size) }
