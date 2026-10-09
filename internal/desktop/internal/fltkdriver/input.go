package fltkdriver

import "golang.org/x/mobile/event/key"

// FLTK key values use ASCII and the X11 keysym values on all platforms.
// Keeping this translation independent of CGo lets headless tests cover it.
func keyCode(code int) key.Code {
	if code >= 'a' && code <= 'z' {
		return key.CodeA + key.Code(code-'a')
	}
	if code >= 'A' && code <= 'Z' {
		return key.CodeA + key.Code(code-'A')
	}
	if code >= '1' && code <= '9' {
		return key.Code1 + key.Code(code-'1')
	}
	if code >= 0xffbe && code <= 0xffc9 {
		return key.CodeF1 + key.Code(code-0xffbe)
	}
	switch code {
	case '0':
		return key.Code0
	case ' ':
		return key.CodeSpacebar
	case ',':
		return key.CodeComma
	case '.':
		return key.CodeFullStop
	case '-':
		return key.CodeHyphenMinus
	case '=':
		return key.CodeEqualSign
	case '[':
		return key.CodeLeftSquareBracket
	case ']':
		return key.CodeRightSquareBracket
	case '\\':
		return key.CodeBackslash
	case ';':
		return key.CodeSemicolon
	case '\'':
		return key.CodeApostrophe
	case '`':
		return key.CodeGraveAccent
	case '/':
		return key.CodeSlash
	case 0xff08:
		return key.CodeDeleteBackspace
	case 0xff09:
		return key.CodeTab
	case 0xff0d:
		return key.CodeReturnEnter
	case 0xff1b:
		return key.CodeEscape
	case 0xffff:
		return key.CodeDeleteForward
	case 0xff50:
		return key.CodeHome
	case 0xff51:
		return key.CodeLeftArrow
	case 0xff52:
		return key.CodeUpArrow
	case 0xff53:
		return key.CodeRightArrow
	case 0xff54:
		return key.CodeDownArrow
	case 0xff55:
		return key.CodePageUp
	case 0xff56:
		return key.CodePageDown
	case 0xff57:
		return key.CodeEnd
	case 0xff63:
		return key.CodeInsert
	case 0xff8d:
		return key.CodeKeypadEnter
	case 0xffe1:
		return key.CodeLeftShift
	case 0xffe2:
		return key.CodeRightShift
	case 0xffe3:
		return key.CodeLeftControl
	case 0xffe4:
		return key.CodeRightControl
	case 0xffe9:
		return key.CodeLeftAlt
	case 0xffea:
		return key.CodeRightAlt
	case 0xffeb:
		return key.CodeLeftGUI
	case 0xffec:
		return key.CodeRightGUI
	}
	return key.CodeUnknown
}

func modifiers(state int) key.Modifiers {
	var mods key.Modifiers
	if state&0x10000 != 0 {
		mods |= key.ModShift
	}
	if state&0x40000 != 0 {
		mods |= key.ModControl
	}
	if state&0x80000 != 0 {
		mods |= key.ModAlt
	}
	if state&0x400000 != 0 {
		mods |= key.ModMeta
	}
	return mods
}
