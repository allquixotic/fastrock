//! Native text-input document adapter. HTML is data; original bytes stay intact
//! until an actual edit. Slint owns shaping, selection, scrolling and clipboard.
#[derive(Clone, Debug, Default)]
struct Span {
    start: usize,
    end: usize,
    tag: String,
    href: String,
}
#[derive(Clone, Debug, Default)]
pub struct RichDoc {
    pub text: String,
    spans: Vec<Span>,
}
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut cursor = 0;
    while let Some(start) = s[cursor..].find('&') {
        let start = cursor + start;
        out.push_str(&s[cursor..start]);
        if let Some(end) = s[start..].find(';').filter(|end| *end < 16) {
            let entity = &s[start + 1..start + end];
            let ch = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" => Some(' '),
                _ => entity
                    .strip_prefix("#x")
                    .and_then(|n| u32::from_str_radix(n, 16).ok())
                    .or_else(|| entity.strip_prefix('#').and_then(|n| n.parse::<u32>().ok()))
                    .and_then(char::from_u32),
            };
            if let Some(ch) = ch {
                out.push(ch);
                cursor = start + end + 1;
                continue;
            }
        }
        out.push('&');
        cursor = start + 1;
    }
    out.push_str(&s[cursor..]);
    out
}
fn safe_link(s: &str) -> bool {
    url::Url::parse(s).is_ok_and(|u| ["https", "http", "mailto"].contains(&u.scheme()))
        && !s.chars().any(char::is_control)
}
impl RichDoc {
    pub fn parse(html: &str) -> Self {
        let mut doc = Self::default();
        let mut stack: Vec<(String, usize, String)> = vec![];
        let mut cursor = 0;
        let mut suppress = 0usize;
        let mut ordered_lists = 0usize;
        while cursor < html.len() {
            let next = html[cursor..]
                .find('<')
                .map(|n| cursor + n)
                .unwrap_or(html.len());
            if suppress == 0 {
                doc.text.push_str(&unescape(&html[cursor..next]));
            }
            if next == html.len() {
                break;
            }
            let Some(end) = html[next..].find('>').map(|n| next + n) else {
                doc.text.push_str(&unescape(&html[next..]));
                break;
            };
            let raw = html[next + 1..end].trim();
            let closing = raw.starts_with('/');
            let tag = raw
                .trim_start_matches('/')
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_end_matches('/')
                .to_ascii_lowercase();
            if ["script", "style"].contains(&tag.as_str()) {
                if closing {
                    suppress = suppress.saturating_sub(1);
                } else {
                    suppress += 1;
                }
                cursor = end + 1;
                continue;
            }
            if suppress == 0 {
                if tag == "ol" {
                    if closing {
                        ordered_lists = ordered_lists.saturating_sub(1);
                    } else {
                        ordered_lists += 1;
                    }
                }
                if tag == "br" {
                    doc.text.push('\n');
                } else if [
                    "p",
                    "div",
                    "li",
                    "h1",
                    "h2",
                    "h3",
                    "h4",
                    "blockquote",
                    "pre",
                    "ul",
                    "ol",
                ]
                .contains(&tag.as_str())
                    && (!doc.text.is_empty() && !doc.text.ends_with('\n'))
                {
                    doc.text.push('\n');
                }
                if closing {
                    if let Some(index) = stack
                        .iter()
                        .rposition(|(open, _, _)| open == &tag || (tag == "li" && open == "oli"))
                    {
                        let (tag, start, href) = stack.remove(index);
                        if start < doc.text.len() {
                            doc.spans.push(Span {
                                start,
                                end: doc.text.len(),
                                tag,
                                href,
                            });
                        }
                    }
                } else if [
                    "b",
                    "strong",
                    "i",
                    "em",
                    "u",
                    "s",
                    "strike",
                    "del",
                    "h1",
                    "h2",
                    "h3",
                    "li",
                    "a",
                    "code",
                    "blockquote",
                ]
                .contains(&tag.as_str())
                {
                    let href = if tag == "a" {
                        attribute(raw, "href")
                            .filter(|s| safe_link(s))
                            .unwrap_or_default()
                    } else {
                        String::new()
                    };
                    stack.push((
                        if tag == "li" && ordered_lists > 0 {
                            "oli".into()
                        } else {
                            tag
                        },
                        doc.text.len(),
                        href,
                    ));
                }
            }
            cursor = end + 1;
        }
        for (tag, start, href) in stack {
            if start < doc.text.len() {
                doc.spans.push(Span {
                    start,
                    end: doc.text.len(),
                    tag,
                    href,
                });
            }
        }
        while doc.text.ends_with('\n') {
            doc.text.pop();
        }
        for span in &mut doc.spans {
            span.end = span.end.min(doc.text.len());
        }
        doc
    }
    pub fn edit_text(&mut self, next: String) {
        let mut start = self
            .text
            .bytes()
            .zip(next.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        while !self.text.is_char_boundary(start) || !next.is_char_boundary(start) {
            start -= 1;
        }
        let mut suffix = self.text[start..]
            .bytes()
            .rev()
            .zip(next[start..].bytes().rev())
            .take_while(|(a, b)| a == b)
            .count();
        while !self.text.is_char_boundary(self.text.len() - suffix)
            || !next.is_char_boundary(next.len() - suffix)
        {
            suffix -= 1;
        }
        let old_end = self.text.len() - suffix;
        let new_end = next.len() - suffix;
        let delta = new_end as isize - old_end as isize;
        for span in &mut self.spans {
            span.start = if span.start >= old_end {
                span.start.saturating_add_signed(delta)
            } else {
                span.start.min(start)
            };
            span.end = if span.end >= old_end {
                span.end.saturating_add_signed(delta)
            } else {
                span.end.min(new_end)
            };
        }
        self.spans
            .retain(|s| s.start < s.end && s.end <= next.len());
        self.text = next;
    }
    pub fn format(&mut self, mark: &str, a: usize, b: usize, href: &str) {
        let (start, end) = (a.min(b).min(self.text.len()), a.max(b).min(self.text.len()));
        if start == end || !self.text.is_char_boundary(start) || !self.text.is_char_boundary(end) {
            return;
        }
        let tag = match mark {
            "B" => "strong",
            "I" => "em",
            "U" => "u",
            "S" => "s",
            "H1" => "h1",
            "H2" => "h2",
            "• List" => "li",
            "1. List" => "oli",
            "Link" => "a",
            _ => return,
        };
        if tag == "a" && !safe_link(href) {
            return;
        }
        if let Some(index) = self
            .spans
            .iter()
            .position(|s| s.start == start && s.end == end && s.tag == tag)
        {
            self.spans.remove(index);
        } else {
            self.spans.push(Span {
                start,
                end,
                tag: tag.into(),
                href: href.into(),
            });
        }
    }
    pub fn html(&self) -> String {
        let mut points = vec![0, self.text.len()];
        for s in &self.spans {
            points.extend([s.start, s.end]);
        }
        points.sort_unstable();
        points.dedup();
        let mut out = String::from("<p>");
        for range in points.windows(2) {
            let (start, end) = (range[0], range[1]);
            let spans = self
                .spans
                .iter()
                .filter(|s| s.start <= start && s.end >= end)
                .collect::<Vec<_>>();
            for s in &spans {
                if s.tag == "a" {
                    out.push_str(&format!("<a href=\"{}\">", escape(&s.href)));
                } else if s.tag == "oli" {
                    out.push_str("<ol><li>");
                } else if s.tag == "li" {
                    out.push_str("<ul><li>");
                } else {
                    out.push_str(&format!("<{}>", s.tag));
                }
            }
            out.push_str(&escape(&self.text[start..end]).replace('\n', "<br>"));
            for s in spans.iter().rev() {
                if s.tag == "oli" {
                    out.push_str("</li></ol>");
                } else if s.tag == "li" {
                    out.push_str("</li></ul>");
                } else {
                    out.push_str(&format!("</{}>", s.tag));
                }
            }
        }
        out.push_str("</p>");
        out
    }
    pub fn preview(&self) -> Vec<(String, u8)> {
        let mut offset = 0;
        self.text
            .split('\n')
            .map(|line| {
                let end = offset + line.len();
                let mut points = vec![offset, end];
                for span in &self.spans {
                    if span.start < end && span.end > offset {
                        points.extend([span.start.max(offset), span.end.min(end)]);
                    }
                }
                points.sort_unstable();
                points.dedup();
                let heading = self
                    .spans
                    .iter()
                    .filter(|s| s.start < end && s.end > offset)
                    .find_map(|s| s.tag.strip_prefix('h').and_then(|n| n.parse::<u8>().ok()))
                    .unwrap_or(0);
                let list = self.spans.iter().find(|s| {
                    s.start <= offset && s.end >= end && ["li", "oli"].contains(&s.tag.as_str())
                });
                let mut out = if list.is_some_and(|s| s.tag == "oli") {
                    "1. ".into()
                } else if list.is_some() {
                    "- ".into()
                } else {
                    String::new()
                };
                for range in points.windows(2) {
                    let (a, b) = (range[0], range[1]);
                    let spans = self
                        .spans
                        .iter()
                        .filter(|s| s.start <= a && s.end >= b)
                        .collect::<Vec<_>>();
                    let mut text = self.text[a..b]
                        .replace('\\', "\\\\")
                        .replace('*', "\\*")
                        .replace('_', "\\_")
                        .replace('[', "\\[")
                        .replace('`', "\\`")
                        .replace('<', "&lt;");
                    for span in spans {
                        text = match span.tag.as_str() {
                            "b" | "strong" => format!("**{text}**"),
                            "i" | "em" => format!("_{text}_"),
                            "u" => format!("<u>{text}</u>"),
                            "s" | "del" | "strike" => format!("~~{text}~~"),
                            "code" => format!("`{text}`"),
                            "a" if !span.href.is_empty() => {
                                format!("[{text}]({})", span.href.replace(')', "%29"))
                            }
                            _ => text,
                        };
                    }
                    out.push_str(&text);
                }
                offset = end + 1;
                (out, heading)
            })
            .collect()
    }
    pub fn markdown(&self) -> String {
        self.preview()
            .into_iter()
            .map(|(text, _)| text)
            .collect::<Vec<_>>()
            .join("\n\n")
    }
    pub fn legacy(value: &serde_json::Value) -> anyhow::Result<String> {
        if value["Changed"].as_bool() != Some(true) {
            return Ok(value["Original"].as_str().unwrap_or("").into());
        }
        let text = if let Some(s) = value["Text"].as_str() {
            s.into()
        } else {
            value["Text"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_u64().and_then(|n| char::from_u32(n as u32)))
                .collect::<String>()
        };
        let mut doc = Self {
            text,
            spans: vec![],
        };
        let offsets = doc
            .text
            .char_indices()
            .map(|(i, _)| i)
            .chain(Some(doc.text.len()))
            .collect::<Vec<_>>();
        let mut start = 0;
        for span in value["Spans"].as_array().into_iter().flatten() {
            let end = span["End"].as_u64().unwrap_or(0) as usize;
            anyhow::ensure!(
                end > start && end < offsets.len(),
                "Legacy rich span is invalid; original session retained"
            );
            let format = &span["Format"];
            let style = format["Style"].as_u64().unwrap_or(0);
            for (bit, mark) in [(1, "B"), (2, "I"), (4, "U"), (8, "S")] {
                if style & bit != 0 {
                    doc.format(mark, offsets[start], offsets[end], "");
                }
            }
            let heading = format["Heading"].as_u64().unwrap_or(0);
            if heading > 0 {
                doc.format(
                    if heading == 1 { "H1" } else { "H2" },
                    offsets[start],
                    offsets[end],
                    "",
                );
            }
            let list = format["List"].as_u64().unwrap_or(0);
            if list > 0 {
                doc.format(
                    if list == 1 { "• List" } else { "1. List" },
                    offsets[start],
                    offsets[end],
                    "",
                );
            }
            if let Some(link) = format["Link"].as_str().filter(|s| !s.is_empty()) {
                doc.format("Link", offsets[start], offsets[end], link);
            }
            start = end;
        }
        Ok(doc.html())
    }
}
fn attribute(tag: &str, key: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let at = lower.find(&format!("{key}="))? + key.len() + 1;
    let rest = tag[at..].trim_start();
    let first = rest.chars().next()?;
    let value = if matches!(first, '\'' | '"') {
        let rest = &rest[first.len_utf8()..];
        &rest[..rest.find(first)?]
    } else {
        rest.split_whitespace().next()?
    };
    Some(unescape(value))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_entities_and_formats_utf8_without_manual_glyph_geometry() {
        let mut doc = RichDoc::parse("<p>A &amp; <b>β</b></p><p>Next</p>");
        assert_eq!(doc.text, "A & β\nNext");
        doc.format("U", 4, 6, "");
        assert!(doc.html().contains("<u>"));
        doc.edit_text("A & β🙂\nNext".into());
        assert!(doc.html().contains("β🙂"));
    }
    #[test]
    fn malicious_markup_does_not_become_links_or_text() {
        let doc = RichDoc::parse("hello<script>run()</script><a href='javascript:evil'>click</a>");
        assert_eq!(doc.text, "helloclick");
        assert!(!doc.html().contains("javascript"));
    }
}

#[cfg(test)]
mod acceptance_tests {
    use super::*;
    #[test]
    fn v20_native_preview_preserves_headings_lists_links_and_unicode() {
        let doc = RichDoc::parse(
            "<h2>世界</h2><ol><li>One</li><li><b>Two</b></li></ol><p><a href=\"https://example.com\">Link</a></p>",
        );
        let blocks = doc.preview();
        assert!(
            blocks
                .iter()
                .any(|(text, heading)| text.contains("世界") && *heading == 2)
        );
        assert!(blocks.iter().any(|(text, _)| text.starts_with("1. ")));
        assert!(blocks.iter().any(|(text, _)| text.contains("**Two**")));
        assert!(
            blocks
                .iter()
                .any(|(text, _)| text.contains("https://example.com"))
        );
        assert!(doc.html().contains("<ol>"));
    }
}
