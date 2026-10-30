//! Terminal key sequences; application shortcuts are filtered before this layer.
pub(super) fn key(
    text: &str,
    control: bool,
    shift: bool,
    alt: bool,
    application_cursor: bool,
) -> Vec<u8> {
    use slint::platform::Key;
    let is = |key: Key| text == slint::SharedString::from(key).as_str();
    let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(control);
    for (key, code) in [
        (Key::UpArrow, 'A'),
        (Key::DownArrow, 'B'),
        (Key::RightArrow, 'C'),
        (Key::LeftArrow, 'D'),
        (Key::Home, 'H'),
        (Key::End, 'F'),
    ] {
        if is(key) {
            return if modifier > 1 {
                format!("\x1b[1;{modifier}{code}").into_bytes()
            } else {
                format!("\x1b{}{code}", if application_cursor { 'O' } else { '[' }).into_bytes()
            };
        }
    }
    for (key, code) in [
        (Key::Insert, 2),
        (Key::Delete, 3),
        (Key::PageUp, 5),
        (Key::PageDown, 6),
        (Key::F5, 15),
        (Key::F6, 17),
        (Key::F7, 18),
        (Key::F8, 19),
        (Key::F9, 20),
        (Key::F10, 21),
        (Key::F11, 23),
        (Key::F12, 24),
    ] {
        if is(key) {
            return if modifier > 1 {
                format!("\x1b[{code};{modifier}~").into_bytes()
            } else {
                format!("\x1b[{code}~").into_bytes()
            };
        }
    }
    for (key, code) in [
        (Key::F1, 'P'),
        (Key::F2, 'Q'),
        (Key::F3, 'R'),
        (Key::F4, 'S'),
    ] {
        if is(key) {
            return if modifier > 1 {
                format!("\x1b[1;{modifier}{code}").into_bytes()
            } else {
                format!("\x1bO{code}").into_bytes()
            };
        }
    }
    let mut bytes = if is(Key::Return) {
        vec![b'\r']
    } else if is(Key::Backspace) {
        vec![127]
    } else if is(Key::Escape) {
        vec![27]
    } else if is(Key::Tab) {
        if shift {
            b"\x1b[Z".to_vec()
        } else {
            vec![b'\t']
        }
    } else if control && text.len() == 1 {
        let c = text.as_bytes()[0];
        match c {
            b'a'..=b'z' | b'A'..=b'Z' => vec![c & 31],
            b' ' | b'@' => vec![0],
            b'['..=b'_' => vec![c & 31],
            b'?' => vec![127],
            _ => Vec::new(),
        }
    } else if text
        .chars()
        .all(|c| !('\u{e000}'..='\u{f8ff}').contains(&c))
    {
        text.as_bytes().to_vec()
    } else {
        Vec::new()
    };
    if alt && !bytes.is_empty() {
        bytes.insert(0, 27);
    }
    bytes
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v61_terminal_input_preserves_control_and_application_keys() {
        assert_eq!(key("c", true, false, false, false), [3]);
        assert_eq!(key("界", false, false, false, false), "界".as_bytes());
        let up = slint::SharedString::from(slint::platform::Key::UpArrow);
        assert_eq!(key(&up, false, false, false, true), b"\x1bOA");
        assert_eq!(key(&up, true, false, false, true), b"\x1b[1;5A");
        let enter = slint::SharedString::from(slint::platform::Key::Return);
        assert_eq!(key(&enter, false, false, false, false), b"\r");
    }
}
