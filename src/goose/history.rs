//! Stable ordinal cursors: appending streamed items cannot move an older page.
use super::{error, text};
use codex_app_server_protocol::JSONRPCErrorError;
use serde_json::Value;

pub(super) struct Page {
    pub offset: usize,
    pub limit: usize,
    pub descending: bool,
    pub next: Option<String>,
}

pub(super) fn page(params: &Value, total: usize, turns: bool) -> Result<Page, JSONRPCErrorError> {
    let descending = params["sortDirection"] != "asc";
    let kind = if turns { "turns" } else { "items" };
    let direction = if descending { "before" } else { "after" };
    let prefix = format!("goose-{kind}-{direction}:");
    let limit = params["limit"].as_u64().unwrap_or(100).clamp(1, 100) as usize;
    let cursor = text(params, "cursor");
    let boundary = if cursor.is_empty() {
        if descending { total } else { 0 }
    } else if let Some(raw) = cursor.strip_prefix(&prefix) {
        raw.parse::<usize>()
            .map_err(|_| error("Invalid Goose history cursor"))?
            .min(total)
    } else if let Some(raw) = cursor.strip_prefix("goose-page:") {
        // Compatibility with an already issued 0.3.x offset cursor.
        let offset = raw
            .parse::<usize>()
            .map_err(|_| error("Invalid Goose history cursor"))?
            .min(total);
        if descending { total - offset } else { offset }
    } else {
        return Err(error("History cursor belongs to a different view"));
    };
    let offset = if descending {
        total - boundary
    } else {
        boundary
    };
    let next_boundary = if descending {
        boundary.saturating_sub(limit)
    } else {
        boundary.saturating_add(limit).min(total)
    };
    let more = if descending {
        next_boundary > 0
    } else {
        next_boundary < total
    };
    Ok(Page {
        offset,
        limit,
        descending,
        next: more.then(|| format!("{prefix}{next_boundary}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn v43_cursor_keeps_older_boundary_when_streaming_appends() {
        let first = page(&json!({"limit":3}), 10, false).unwrap();
        assert_eq!(first.next.as_deref(), Some("goose-items-before:7"));
        let next = page(&json!({"limit":3,"cursor":first.next}), 16, false).unwrap();
        assert_eq!(next.offset, 9);
        assert_eq!(next.next.as_deref(), Some("goose-items-before:4"));
        assert!(page(&json!({"cursor":next.next}), 16, true).is_err());
        assert!(page(&json!({"cursor":"goose-items-before:invalid"}), 16, false).is_err());
        assert_eq!(
            page(&json!({"cursor":"goose-page:5"}), 10, false)
                .unwrap()
                .offset,
            5
        );
        let ascending = page(
            &json!({"limit":3,"sortDirection":"asc","cursor":"goose-items-after:3"}),
            16,
            false,
        )
        .unwrap();
        assert_eq!(ascending.offset, 3);
        assert_eq!(ascending.next.as_deref(), Some("goose-items-after:6"));
    }
}
