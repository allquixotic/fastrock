//! Pure projections into native Slint models. No UI-thread I/O.
use super::{
    editor::{Editor, identity_field},
    rich::RichDoc,
    types::*,
    view::View,
};
use crate::ui::{
    RallyCell, RallyChoice, RallyField, RallyLane, RallyMetric, RallyProposal, RallyRichBlock,
    RallyRow,
};
use slint::{Color, ModelRc, SharedString, StyledText, VecModel};
use std::{collections::BTreeMap, rc::Rc};
pub fn model<T: Clone + 'static>(values: Vec<T>) -> ModelRc<T> {
    Rc::new(VecModel::from(values)).into()
}
pub fn strings(values: impl IntoIterator<Item = impl AsRef<str>>) -> ModelRc<SharedString> {
    model(values.into_iter().map(|s| s.as_ref().into()).collect())
}
pub fn choice(label: impl Into<String>, value: impl Into<String>) -> RallyChoice {
    RallyChoice {
        label: label.into().into(),
        value: value.into().into(),
    }
}
pub fn object_label(o: &Object) -> String {
    let name = [
        "DisplayName",
        "Name",
        "_refObjectName",
        "UserName",
        "FormattedID",
        "ObjectID",
    ]
    .into_iter()
    .map(|k| o.text(k))
    .find(|s| !s.is_empty())
    .unwrap_or_default();
    if o.kind() == "User" && !o.text("UserName").is_empty() && name != o.text("UserName") {
        format!("{name} · {}", o.text("UserName"))
    } else {
        name
    }
}
pub fn choices(rows: &[Object], blank: &str) -> ModelRc<RallyChoice> {
    model(
        std::iter::once(choice(blank, ""))
            .chain(rows.iter().map(|o| {
                choice(
                    object_label(o).trim().to_string().replace('\n', " "),
                    o.text("_ref"),
                )
            }))
            .collect(),
    )
}
pub fn field(f: &Field, e: &Editor) -> RallyField {
    let value = e.draft.get(&f.name).cloned().unwrap_or_default();
    let text = if f.attribute_type == "COLLECTION" {
        serde_json::to_string(&value).unwrap_or_default()
    } else {
        e.draft.text(&f.name)
    };
    let kind = if f.read_only || identity_field(&f.name) {
        "readonly"
    } else if f.name == "DisplayColor" || !f.allowed_values.is_empty() {
        "choice"
    } else {
        match f.attribute_type.as_str() {
            "BOOLEAN" => "boolean",
            "OBJECT" => "reference",
            "COLLECTION" => "collection",
            "TEXT" => "rich",
            _ => "text",
        }
    };
    let rich = RichDoc::parse(&text);
    let markdown = rich.markdown();
    let comparison = e
        .conflicts
        .get(&f.name)
        .map(|c| {
            format!(
                "Before: {}\nMine: {}\nRally: {}",
                c.before, c.local, c.remote
            )
        })
        .unwrap_or_default();
    let mut choices = f
        .allowed_values
        .iter()
        .map(|value| choice(value, value))
        .collect::<Vec<_>>();
    if f.name == "DisplayColor" && choices.is_empty() {
        choices = [
            ("Dark Blue", "#105cab"),
            ("Blue", "#21a2e0"),
            ("Green", "#107c1e"),
            ("Purple", "#4a1d7e"),
            ("Pink", "#df1a7b"),
            ("Burnt Orange", "#ee6c19"),
            ("Orange", "#f9a814"),
            ("Yellow", "#fce205"),
            ("Grey", "#848689"),
        ]
        .iter()
        .map(|(label, value)| choice(*label, *value))
        .collect();
    }
    if kind == "choice" && !f.required {
        choices.insert(0, choice("(None)", ""));
    }
    if kind == "choice" && !choices.iter().any(|choice| choice.value.as_str() == text) {
        choices.push(choice(&text, &text));
    }
    let choice_index = choices
        .iter()
        .position(|choice| choice.value.as_str() == text)
        .unwrap_or(0) as i32;
    RallyField {
        choice_index,
        name: f.name.clone().into(),
        label: f.label().into(),
        value: text.into(),
        plain: rich.text.clone().into(),
        markdown: markdown.clone().into(),
        rich: StyledText::from_markdown(&markdown)
            .unwrap_or_else(|_| StyledText::from_plain_text(&rich.text)),
        rich_blocks: model(
            rich.preview()
                .into_iter()
                .map(|(text, heading)| RallyRichBlock {
                    text: StyledText::from_markdown(&text)
                        .unwrap_or_else(|_| StyledText::from_plain_text(&text)),
                    heading: heading as i32,
                })
                .collect(),
        ),
        kind: kind.into(),
        choices: model(choices),
        read_only: f.read_only || identity_field(&f.name),
        required: f.required,
        conflict: e.conflicts.contains_key(&f.name),
        comparison: comparison.into(),
    }
}
pub fn row(o: &Object, view: &View, fields: &[Field], selected: bool, position: usize) -> RallyRow {
    let state = o.text(if view.page == "teamboard" {
        "ScheduleState"
    } else {
        state_field(&o.kind())
    });
    let mut status = vec![];
    if o.flag("Blocked") {
        status.push("Blocked".into());
    }
    if o.flag("Ready") {
        status.push("Ready".into());
    }
    if o.count("Tasks") > 0 {
        status.push(format!("{} tasks", o.count("Tasks")));
    }
    if view.display.age_days > 0 {
        if let Ok(date) = chrono::DateTime::parse_from_rfc3339(&o.text("LastUpdateDate")) {
            let days = (chrono::Utc::now() - date.with_timezone(&chrono::Utc)).num_days();
            if days >= view.display.age_days as i64 {
                status.push(format!("{days}d old"));
            }
        }
    }
    let color = if view.display.color_by == "Work Item" {
        let text = o.text("DisplayColor");
        u32::from_str_radix(text.trim_start_matches('#'), 16)
            .ok()
            .map(|rgb| Color::from_rgb_u8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8))
            .unwrap_or(Color::from_rgb_u8(97, 140, 204))
    } else {
        let key = o.text(&view.display.color_by);
        let hash = key
            .bytes()
            .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
        Color::from_rgb_u8(
            80 + (hash % 120) as u8,
            80 + ((hash >> 8) % 120) as u8,
            80 + ((hash >> 16) % 120) as u8,
        )
    };
    let cells = view
        .columns
        .iter()
        .map(|name| {
            let f = fields.iter().find(|f| &f.name == name);
            let kind = f
                .filter(|f| {
                    !f.read_only
                        && !identity_field(name)
                        && !["TEXT", "COLLECTION"].contains(&f.attribute_type.as_str())
                })
                .map(|f| {
                    if !f.allowed_values.is_empty() {
                        "choice"
                    } else if f.attribute_type == "OBJECT" {
                        "reference"
                    } else if f.attribute_type == "BOOLEAN" {
                        "boolean"
                    } else {
                        "text"
                    }
                })
                .unwrap_or("readonly");
            RallyCell {
                field: name.clone().into(),
                text: if name == "Rank" {
                    position.to_string()
                } else {
                    o.text(name)
                }
                .into(),
                kind: kind.into(),
                choices: strings(f.map(|f| f.allowed_values.clone()).unwrap_or_default()),
                width: if name == "Name" { 220.0 } else { 100.0 },
            }
        })
        .collect();
    RallyRow {
        group_header: false,
        reference: o.text("_ref").into(),
        id: o.id().into(),
        title: if o.kind() == "ConversationPost" {
            RichDoc::parse(&o.text("Text")).text
        } else if !o.text("Name").is_empty() {
            o.text("Name")
        } else {
            RichDoc::parse(&o.text("Text")).text
        }
        .into(),
        kind: o.kind().into(),
        state: state.into(),
        group: group_name(o, &view.group).into(),
        owner: o.text("Owner").into(),
        iteration: o.text("Iteration").into(),
        estimate: format!("{} points", o.number("PlanEstimate")).into(),
        status: status.join(" · ").into(),
        color,
        selected,
        cells: model(cells),
        card_fields: model(
            view.card_fields
                .iter()
                .map(|name| {
                    choice(
                        name,
                        if name == "Tasks" {
                            o.count(name).to_string()
                        } else {
                            o.text(name)
                        },
                    )
                })
                .collect(),
        ),
    }
}
pub fn group_name(o: &Object, group: &str) -> String {
    if group == "None" || group.is_empty() {
        String::new()
    } else {
        let text = o.text(group);
        if text.is_empty() {
            "Unassigned".into()
        } else {
            text
        }
    }
}
pub fn lanes(
    rows: &[Object],
    view: &View,
    fields: &[Field],
    workflow: &[Object],
    selected: &BTreeMap<String, Object>,
    collapsed: &std::collections::BTreeSet<String>,
    scroll: &std::collections::BTreeMap<String, f32>,
) -> Vec<RallyLane> {
    let mut groups = BTreeMap::<String, Vec<(usize, &Object)>>::new();
    for (i, o) in rows.iter().enumerate() {
        groups
            .entry(group_name(o, &view.group))
            .or_default()
            .push((i, o));
    }
    if groups.is_empty() {
        groups.insert(String::new(), vec![]);
    }
    let field = if view.page == "teamboard" {
        "ScheduleState"
    } else {
        state_field(page(&view.page).kind)
    };
    let mut states = workflow
        .iter()
        .map(|o| (o.text("Name"), o.text("_ref")))
        .collect::<Vec<_>>();
    for o in rows {
        let name = o.text(field);
        if !states.iter().any(|(s, _)| s == &name) {
            states.push((name, o.reference(field)));
        }
    }
    if states.is_empty() {
        states.push(("Unassigned".into(), String::new()));
    }
    groups
        .into_iter()
        .flat_map(|(group, items)| {
            states
                .iter()
                .map(move |(name, value)| {
                    let cards = items
                        .iter()
                        .filter(|(_, o)| {
                            o.text(field) == *name
                                || (o.text(field).is_empty() && name == "Unassigned")
                        })
                        .map(|(i, o)| {
                            row(
                                o,
                                view,
                                fields,
                                selected.contains_key(&o.text("_ref")),
                                i + 1,
                            )
                        })
                        .collect::<Vec<_>>();
                    let count = cards.len();
                    let over = view.display.wip_limit > 0 && count > view.display.wip_limit;
                    let value = if value.is_empty() { name } else { value };
                    RallyLane {
                        name: name.clone().into(),
                        value: value.clone().into(),
                        group: group.clone().into(),
                        caption: format!(
                            "{}{} · {count}{}",
                            if group.is_empty() {
                                String::new()
                            } else {
                                format!("{group} / ")
                            },
                            name,
                            if over { " · WIP exceeded" } else { "" }
                        )
                        .into(),
                        over_limit: over,
                        collapsed: collapsed.contains(&format!("{group}|{value}")),
                        scroll_y: scroll
                            .get(&format!("{group}|{value}"))
                            .copied()
                            .unwrap_or_default(),
                        cards: model(cards),
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect()
}
pub fn proposals(plan: &super::assistant::Plan) -> ModelRc<RallyProposal> {
    model(
        plan.changes
            .iter()
            .enumerate()
            .map(|(i, c)| RallyProposal {
                index: i as i32,
                selected: c.selected,
                caption: format!("{} {} {}", c.operation, c.kind, c.before.id()).into(),
                comparison: c
                    .fields
                    .iter()
                    .map(|(key, value)| {
                        format!(
                            "{key}: {} → {value}",
                            c.before.get(key).unwrap_or(&serde_json::Value::Null)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
                    .into(),
                outcome: c.outcome.clone().into(),
            })
            .collect(),
    )
}
pub fn analytics(rows: &[Object], view: &View, total: usize) -> String {
    let mut text = format!(
        "Loaded {} of {total} matching items\nEstimates: {}\nBlocked: {}    Ready: {}\n\n",
        rows.len(),
        rows.iter().map(|o| o.number("PlanEstimate")).sum::<f64>(),
        rows.iter().filter(|o| o.flag("Blocked")).count(),
        rows.iter().filter(|o| o.flag("Ready")).count()
    );
    let key = if view.mode == "planning" {
        "Iteration"
    } else {
        state_field(page(&view.page).kind)
    };
    let mut groups = BTreeMap::<String, (usize, f64)>::new();
    for o in rows {
        let group = groups.entry(o.text(key)).or_default();
        group.0 += 1;
        group.1 += o.number("PlanEstimate");
    }
    for (name, (count, estimate)) in groups {
        text.push_str(&format!("{name}: {count} items · {estimate} points\n"));
    }
    if view.mode == "timeline" {
        text.push_str("\nPlanned dates\n");
        for o in rows {
            text.push_str(&format!(
                "{} {}   {} → {}\n",
                o.id(),
                o.text("Name"),
                o.text("PlannedStartDate"),
                o.text("PlannedEndDate")
            ));
        }
    }
    text
}

/// Preserve native editor instances and their caret/selection on data updates.
pub fn update<T: Clone + PartialEq + 'static>(current: ModelRc<T>, rows: Vec<T>) -> ModelRc<T> {
    use slint::Model;
    if let Some(model) = current.as_any().downcast_ref::<VecModel<T>>() {
        let old = model.row_count();
        for (i, row) in rows.iter().take(old).enumerate() {
            if model.row_data(i).as_ref() != Some(row) {
                model.set_row_data(i, row.clone());
            }
        }
        if rows.len() > old {
            model.extend(rows.into_iter().skip(old));
        } else {
            for _ in rows.len()..old {
                model.remove(rows.len());
            }
        }
        current
    } else {
        model(rows)
    }
}

pub fn metrics(rows: &[Object], view: &View, total: usize) -> Vec<RallyMetric> {
    let mut result = vec![RallyMetric {
        label: "Loaded work".into(),
        value: format!(
            "{} of {total} matching items; calculations use loaded items only",
            rows.len()
        )
        .into(),
        fraction: if total == 0 {
            0.0
        } else {
            rows.len() as f32 / total as f32
        },
    }];
    if view.mode == "timeline" {
        for item in rows {
            result.push(RallyMetric {
                label: format!("{} {}", item.id(), item.text("Name")).into(),
                value: format!(
                    "{} → {}",
                    item.text("PlannedStartDate"),
                    item.text("PlannedEndDate")
                )
                .into(),
                fraction: if item.text("PlannedEndDate").is_empty() {
                    0.0
                } else {
                    1.0
                },
            });
        }
    } else {
        let key = if view.mode == "planning" {
            "Iteration"
        } else {
            state_field(page(&view.page).kind)
        };
        let mut groups = BTreeMap::<String, (usize, f64)>::new();
        for item in rows {
            let group = groups.entry(item.text(key)).or_default();
            group.0 += 1;
            group.1 += item.number("PlanEstimate");
        }
        for (name, (count, points)) in groups {
            result.push(RallyMetric {
                label: if name.is_empty() {
                    "Unassigned".into()
                } else {
                    name.into()
                },
                value: format!("{count} work items · {points} points").into(),
                fraction: count as f32 / rows.len().max(1) as f32,
            });
        }
    }
    result
}

pub fn list_rows(
    items: &[Object],
    view: &View,
    fields: &[Field],
    selected: &BTreeMap<String, Object>,
    start: usize,
) -> Vec<RallyRow> {
    let rows = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            row(
                item,
                view,
                fields,
                selected.contains_key(&item.text("_ref")),
                start + i,
            )
        })
        .collect::<Vec<_>>();
    if view.mode != "list" || view.group == "None" || view.group.is_empty() {
        return rows;
    }
    let mut groups = BTreeMap::<String, Vec<RallyRow>>::new();
    for row in rows {
        groups.entry(row.group.to_string()).or_default().push(row);
    }
    let mut result = vec![];
    for (name, rows) in groups {
        result.push(RallyRow {
            group_header: true,
            title: format!("{name} · {} loaded items", rows.len()).into(),
            ..Default::default()
        });
        result.extend(rows);
    }
    result
}
pub fn totals(items: &[Object], view: &View, fields: &[Field]) -> Vec<RallyChoice> {
    view.columns
        .iter()
        .filter(|name| {
            fields.iter().any(|field| {
                &field.name == *name
                    && ["DECIMAL", "INTEGER", "QUANTITY"].contains(&field.attribute_type.as_str())
            })
        })
        .map(|name| {
            choice(
                name,
                items
                    .iter()
                    .map(|item| item.number(name))
                    .sum::<f64>()
                    .to_string(),
            )
        })
        .collect()
}

#[cfg(test)]
mod picker_tests {
    use super::*;
    #[test]
    fn user_labels_need_no_name_attribute() {
        let o = serde_json::json!({"_type":"User","DisplayName":"Sean","UserName":"sean@example"})
            .as_object()
            .unwrap()
            .clone();
        assert_eq!(object_label(&o), "Sean · sean@example");
    }
}
