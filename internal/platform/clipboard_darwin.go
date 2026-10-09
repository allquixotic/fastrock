package platform

import (
	"fmt"
	"github.com/ebitengine/purego"
	"github.com/ebitengine/purego/objc"
	"runtime"
	"unsafe"
)

func ClipboardPNG() ([]byte, error) {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	if _, err := purego.Dlopen("/System/Library/Frameworks/AppKit.framework/AppKit", purego.RTLD_LAZY|purego.RTLD_GLOBAL); err != nil {
		return nil, err
	}
	pool := objc.ID(objc.GetClass("NSAutoreleasePool")).Send(objc.RegisterName("alloc")).Send(objc.RegisterName("init"))
	defer pool.Send(objc.RegisterName("drain"))
	stringClass := objc.ID(objc.GetClass("NSString"))
	makeString := objc.RegisterName("stringWithUTF8String:")
	pb := objc.ID(objc.GetClass("NSPasteboard")).Send(objc.RegisterName("generalPasteboard"))
	data := pb.Send(objc.RegisterName("dataForType:"), stringClass.Send(makeString, "public.png"))
	if data == 0 {
		tiff := pb.Send(objc.RegisterName("dataForType:"), stringClass.Send(makeString, "public.tiff"))
		if tiff == 0 {
			return nil, nil
		}
		rep := objc.ID(objc.GetClass("NSBitmapImageRep")).Send(objc.RegisterName("imageRepWithData:"), tiff)
		if rep == 0 {
			return nil, fmt.Errorf("unsupported clipboard image")
		}
		data = rep.Send(objc.RegisterName("representationUsingType:properties:"), uintptr(4), objc.ID(objc.GetClass("NSDictionary")).Send(objc.RegisterName("dictionary")))
	}
	if data == 0 {
		return nil, nil
	}
	n := int(data.Send(objc.RegisterName("length")))
	if n <= 0 || n > 32<<20 {
		return nil, fmt.Errorf("clipboard PNG exceeds 32 MiB")
	}
	ptr := objc.Send[unsafe.Pointer](data, objc.RegisterName("bytes"))
	return append([]byte(nil), unsafe.Slice((*byte)(ptr), n)...), nil
}
