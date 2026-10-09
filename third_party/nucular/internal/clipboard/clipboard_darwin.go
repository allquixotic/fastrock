package clipboard

import (
	"errors"
	"runtime"
	"sync"

	"github.com/ebitengine/purego"
	"github.com/ebitengine/purego/objc"
)

var pasteboard struct {
	sync.Once
	ok                                                    bool
	general, utf8, read, clear, write, alloc, init, drain objc.SEL
}

func Start() {}
func prepare() bool {
	pasteboard.Do(func() {
		_, err := purego.Dlopen("/System/Library/Frameworks/AppKit.framework/AppKit", purego.RTLD_LAZY|purego.RTLD_GLOBAL)
		if err != nil {
			return
		}
		pasteboard.general = objc.RegisterName("generalPasteboard")
		pasteboard.utf8 = objc.RegisterName("stringWithUTF8String:")
		pasteboard.read = objc.RegisterName("stringForType:")
		pasteboard.clear = objc.RegisterName("clearContents")
		pasteboard.write = objc.RegisterName("setString:forType:")
		pasteboard.alloc = objc.RegisterName("alloc")
		pasteboard.init = objc.RegisterName("init")
		pasteboard.drain = objc.RegisterName("drain")
		pasteboard.ok = true
	})
	return pasteboard.ok
}
func Read(primary bool) (string, error) {
	if primary {
		return "", errors.New("primary selection is unavailable on macOS")
	}
	if !prepare() {
		return "", errors.New("macOS pasteboard is unavailable")
	}
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	pool := objc.ID(objc.GetClass("NSAutoreleasePool")).Send(pasteboard.alloc).Send(pasteboard.init)
	defer pool.Send(pasteboard.drain)
	pb := objc.ID(objc.GetClass("NSPasteboard")).Send(pasteboard.general)
	typ := objc.ID(objc.GetClass("NSString")).Send(pasteboard.utf8, "public.utf8-plain-text")
	s := pb.Send(pasteboard.read, typ)
	if s == 0 {
		return "", nil
	}
	return objc.Send[string](s, objc.RegisterName("UTF8String")), nil
}
func Get() string        { value, _ := Read(false); return value }
func GetPrimary() string { return Get() }
func Write(text string) error {
	if !prepare() {
		return errors.New("macOS pasteboard is unavailable")
	}
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	pool := objc.ID(objc.GetClass("NSAutoreleasePool")).Send(pasteboard.alloc).Send(pasteboard.init)
	defer pool.Send(pasteboard.drain)
	pb := objc.ID(objc.GetClass("NSPasteboard")).Send(pasteboard.general)
	class := objc.ID(objc.GetClass("NSString"))
	typ := class.Send(pasteboard.utf8, "public.utf8-plain-text")
	s := class.Send(pasteboard.utf8, text)
	pb.Send(pasteboard.clear)
	if !objc.Send[bool](pb, pasteboard.write, s, typ) {
		return errors.New("could not write to the macOS pasteboard")
	}
	return nil
}
func Set(text string) { _ = Write(text) }
