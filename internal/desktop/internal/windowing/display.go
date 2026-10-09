package windowing

import "image"

// Display is the native surface contract used by Fastrock's custom controls.
// Production implements it with go-fltk; headless tests never open a display.
type Display interface {
	NewWindow(*NewWindowOptions) (Window, error)
	NewBuffer(image.Point) (Buffer, error)
}

type NewWindowOptions struct {
	Width, Height int
	Title         string
}

type Buffer interface {
	Release()
	Size() image.Point
	RGBA() *image.RGBA
}

type Window interface {
	NextEvent() any
	Send(any)
	Release()
	Upload(image.Point, Buffer, image.Rectangle)
	Publish()
	SetTitle(string)
	SetIcon(image.Image)
	Controls([]Control)
}
