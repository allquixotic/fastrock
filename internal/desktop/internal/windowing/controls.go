package windowing

import (
	"image"
	"image/color"
)

type ControlKind uint8

const (
	ButtonControl ControlKind = iota
	InputControl
)

// Control is a value-only snapshot: the native thread never touches editor
// state or closures owned by the layout thread.
type Control struct {
	ID                                uint64
	Kind                              ControlKind
	Bounds                            image.Rectangle
	Text, Placeholder                 string
	Foreground, Background, Selection color.RGBA
	Border                            color.RGBA
	BorderWidth                       int
	FontSize                          int
	Sequence                          uint64
	Cursor, Mark                      int // UTF-8 byte offsets, matching FLTK
	Focused                           bool
}
type ControlEvent struct {
	ID, Sequence       uint64
	Kind               ControlKind
	Text, Base         string
	Cursor, Mark       int
	Focused, Committed bool
}

// ControlFocusEvent clears focus from canvas editors before a native widget
// can forward keyboard shortcuts into the application.
type ControlFocusEvent struct{ ID uint64 }
