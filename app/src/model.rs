//! Data model: the generative-UI DSL nodes. Everything the app persists for real
//! lives in store.rs.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Generative-UI DSL (the egui-native analog of CopilotKit's generative UI):
// The model emits a ```ui fenced JSON block in its answer; we render it as
// native egui widgets and send interactions back as user messages
// (the renderAndWaitForResponse pattern).
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UiNode {
    Text {
        text: String,
    },
    Heading {
        level: Option<u8>,
        text: String,
    },
    Button {
        label: String,
        action: String,
    },
    Table {
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    Form {
        submit: String,
        fields: Vec<FormField>,
    },
    BarChart {
        title: Option<String>,
        bars: Vec<Bar>,
    },
    Divider {},
    Columns {
        children: Vec<UiNode>,
    },
    Card {
        title: Option<String>,
        children: Vec<UiNode>,
    },
    /// Local input: a slider. Its value is available to `computed` nodes of
    /// the same block and is sent with a `form` submit of the same block.
    Slider {
        name: String,
        label: String,
        min: f64,
        max: f64,
        #[serde(default)]
        step: Option<f64>,
        #[serde(default)]
        value: Option<f64>,
    },
    /// Local input: a numeric text field.
    Number {
        name: String,
        label: String,
        #[serde(default)]
        value: Option<f64>,
    },
    /// Local input: an on/off switch (`true`/`false`).
    Toggle {
        name: String,
        label: String,
        #[serde(default)]
        value: Option<bool>,
    },
    /// Local input: a single-line text field.
    TextInput {
        name: String,
        label: String,
        #[serde(default)]
        value: Option<String>,
    },
    /// Local input: a drop-down. Numeric options count as numbers in
    /// `computed` expressions, everything else is 0.
    Select {
        name: String,
        label: String,
        options: Vec<String>,
        #[serde(default)]
        value: Option<String>,
    },
    /// Result of a formula over the inputs of the same block, evaluated
    /// locally on every change (see formula.rs) — no model round trip.
    Computed {
        label: String,
        expr: String,
        #[serde(default)]
        decimals: Option<u8>,
        #[serde(default)]
        unit: Option<String>,
    },
    LineChart {
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        x_label: Option<String>,
        #[serde(default)]
        y_label: Option<String>,
        series: Vec<Series>,
    },
    PieChart {
        #[serde(default)]
        title: Option<String>,
        slices: Vec<Bar>,
    },
    /// 0..1 as a pixel meter with 20 blocks plus a percentage.
    Progress {
        label: String,
        value: f64,
    },
    /// Locally checkable list; the state is kept per message and block.
    Checklist {
        items: Vec<CheckItem>,
    },
    /// Two-column key/value list.
    Kv {
        rows: Vec<[String; 2]>,
    },
}

/// One line of a `line_chart`: points as [x, y].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Series {
    pub name: String,
    pub points: Vec<[f64; 2]>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckItem {
    pub text: String,
    #[serde(default)]
    pub done: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FormField {
    pub name: String,
    pub label: String,
    #[serde(default = "field_text")]
    pub kind: String, // "text" | "number" | "checkbox"
    #[serde(default)]
    pub value: Option<String>,
}

fn field_text() -> String {
    "text".into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bar {
    pub label: String,
    pub value: f64,
}

/// An interaction the user made with a rendered UI block.
pub enum UiEvent {
    Button {
        action: String,
    },
    Form {
        submit: String,
        values: serde_json::Value,
    },
}

/// Most entries a list inside one node keeps (audit M7: a block of up to
/// 512 KiB could hold 15 000 bars, each painted every frame; the node
/// budget never saw them, bars are not nodes).
pub const MAX_BARS: usize = 100;
pub const MAX_PIE_SLICES: usize = 64;
pub const MAX_LIST_ITEMS: usize = 200;
pub const MAX_FORM_FIELDS: usize = 64;
pub const MAX_SERIES: usize = 12;
pub const MAX_POINTS: usize = 2_000;

/// `v` cut to `max` entries; returns how many were dropped.
fn cut<T>(v: &mut Vec<T>, max: usize) -> usize {
    let dropped = v.len().saturating_sub(max);
    v.truncate(max);
    dropped
}

/// Cap every list inside `nodes` (bars, slices, items, rows, series,
/// points, options, fields, recursively); returns how many entries were
/// dropped. Tables are capped where they render (32 × 200 cells).
pub fn cap_lists(nodes: &mut [UiNode]) -> usize {
    nodes.iter_mut().map(cap_node).sum()
}

fn cap_node(node: &mut UiNode) -> usize {
    match node {
        UiNode::Columns { children } | UiNode::Card { children, .. } => cap_lists(children),
        UiNode::BarChart { bars, .. } => cut(bars, MAX_BARS),
        UiNode::PieChart { slices, .. } => cut(slices, MAX_PIE_SLICES),
        UiNode::Checklist { items } => cut(items, MAX_LIST_ITEMS),
        UiNode::Kv { rows } => cut(rows, MAX_LIST_ITEMS),
        UiNode::Select { options, .. } => cut(options, MAX_LIST_ITEMS),
        UiNode::Form { fields, .. } => cut(fields, MAX_FORM_FIELDS),
        UiNode::LineChart { series, .. } => {
            cut(series, MAX_SERIES)
                + series
                    .iter_mut()
                    .map(|s| cut(&mut s.points, MAX_POINTS))
                    .sum::<usize>()
        }
        _ => 0,
    }
}

/// [`cap_lists`], with a note node at the end when anything was dropped,
/// so a cut chart or list never looks complete.
pub fn cap_with_note(nodes: &mut Vec<UiNode>) {
    let dropped = cap_lists(nodes);
    if dropped > 0 {
        nodes.push(UiNode::Text {
            text: crate::i18n::trf(
                "{n} more entries of this view not shown.",
                &[("n", &dropped)],
            ),
        });
    }
}

/// How many nodes `nodes` holds, containers and their children included.
pub fn node_count(nodes: &[UiNode]) -> usize {
    nodes
        .iter()
        .map(|n| match n {
            UiNode::Columns { children } | UiNode::Card { children, .. } => {
                1 + node_count(children)
            }
            _ => 1,
        })
        .sum()
}

/// Extract and parse all ```ui fenced blocks in a message text.
pub fn parse_ui_blocks(text: &str) -> Vec<UiNode> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("```ui") {
        let after = &rest[start + 5..];
        let Some(nl) = after.find('\n') else { break };
        let body = &after[nl + 1..];
        let Some(end) = body.find("```") else { break };
        let json = &body[..end];
        if let Ok(node) = serde_json::from_str::<UiNode>(json) {
            out.push(node);
        } else if let Ok(nodes) = serde_json::from_str::<Vec<UiNode>>(json) {
            out.extend(nodes);
        }
        rest = &body[end + 3..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Audit M7: lists inside a node are capped, the cut is said, and the
    /// node count sees nested children.
    #[test]
    fn lists_are_capped_with_a_note() {
        let bars: Vec<String> = (0..15_000)
            .map(|i| format!("{{\"label\":\"b{i}\",\"value\":{i}}}"))
            .collect();
        let points: Vec<String> = (0..5_000).map(|i| format!("[{i},{i}]")).collect();
        let text = format!(
            "```ui\n[{{\"type\":\"bar_chart\",\"bars\":[{}]}},\
             {{\"type\":\"card\",\"children\":[{{\"type\":\"line_chart\",\"series\":[{{\"name\":\"s\",\"points\":[{}]}}]}}]}}]\n```",
            bars.join(","),
            points.join(",")
        );
        let mut nodes = parse_ui_blocks(&text);
        assert_eq!(node_count(&nodes), 3);
        cap_with_note(&mut nodes);
        match &nodes[0] {
            UiNode::BarChart { bars, .. } => assert_eq!(bars.len(), MAX_BARS),
            other => panic!("expected bars, got {other:?}"),
        }
        match &nodes[1] {
            UiNode::Card { children, .. } => match &children[0] {
                UiNode::LineChart { series, .. } => assert_eq!(series[0].points.len(), MAX_POINTS),
                other => panic!("expected line chart, got {other:?}"),
            },
            other => panic!("expected card, got {other:?}"),
        }
        let dropped = (15_000 - MAX_BARS) + (5_000 - MAX_POINTS);
        match nodes.last() {
            Some(UiNode::Text { text }) => assert!(text.contains(&dropped.to_string()), "{text}"),
            other => panic!("expected the note, got {other:?}"),
        }
        // nothing cut: no note
        let mut small = parse_ui_blocks("```ui\n{\"type\":\"divider\"}\n```");
        cap_with_note(&mut small);
        assert_eq!(small.len(), 1);
    }

    #[test]
    fn old_ui_blocks_parse_unchanged() {
        let text = "```ui\n{\"type\":\"card\",\"title\":\"T\",\"children\":[\
{\"type\":\"text\",\"text\":\"Hallo\"},\
{\"type\":\"heading\",\"level\":2,\"text\":\"H\"},\
{\"type\":\"button\",\"label\":\"OK\",\"action\":\"ok\"},\
{\"type\":\"table\",\"columns\":[\"A\"],\"rows\":[[\"1\"]]},\
{\"type\":\"form\",\"submit\":\"S\",\"fields\":[{\"name\":\"n\",\"label\":\"L\"}]},\
{\"type\":\"bar_chart\",\"bars\":[{\"label\":\"a\",\"value\":1.0}]},\
{\"type\":\"divider\"},\
{\"type\":\"columns\",\"children\":[{\"type\":\"text\",\"text\":\"x\"}]}]}\n```";
        let nodes = parse_ui_blocks(text);
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            UiNode::Card { children, .. } => {
                assert_eq!(children.len(), 8);
                match &children[4] {
                    UiNode::Form { fields, .. } => {
                        assert_eq!(fields[0].kind, "text", "kind default is unchanged");
                    }
                    other => panic!("expected form, got {other:?}"),
                }
            }
            other => panic!("expected card, got {other:?}"),
        }
    }

    #[test]
    fn new_ui_nodes_parse_from_json() {
        let text = "```ui\n[\
{\"type\":\"slider\",\"name\":\"rate\",\"label\":\"Zins\",\"min\":0,\"max\":10,\"step\":0.5,\"value\":2},\
{\"type\":\"slider\",\"name\":\"plain\",\"label\":\"Ohne\",\"min\":0,\"max\":1},\
{\"type\":\"number\",\"name\":\"years\",\"label\":\"Jahre\",\"value\":10},\
{\"type\":\"number\",\"name\":\"empty\",\"label\":\"Leer\"},\
{\"type\":\"select\",\"name\":\"mode\",\"label\":\"Modus\",\"options\":[\"1\",\"2\"],\"value\":\"2\"},\
{\"type\":\"computed\",\"label\":\"Ergebnis\",\"expr\":\"rate*years\",\"decimals\":2,\"unit\":\"€\"},\
{\"type\":\"computed\",\"label\":\"Ohne Einheit\",\"expr\":\"1+1\"},\
{\"type\":\"line_chart\",\"title\":\"T\",\"x_label\":\"x\",\"y_label\":\"y\",\"series\":[{\"name\":\"s\",\"points\":[[0,1],[1,2.5]]}]},\
{\"type\":\"line_chart\",\"series\":[]},\
{\"type\":\"pie_chart\",\"title\":\"P\",\"slices\":[{\"label\":\"a\",\"value\":3},{\"label\":\"b\",\"value\":1}]},\
{\"type\":\"progress\",\"label\":\"Fortschritt\",\"value\":0.42},\
{\"type\":\"checklist\",\"items\":[{\"text\":\"eins\",\"done\":true},{\"text\":\"zwei\"}]},\
{\"type\":\"kv\",\"rows\":[[\"Schlüssel\",\"Wert\"]]}\n]```";
        let nodes = parse_ui_blocks(text);
        assert_eq!(nodes.len(), 13);
        match &nodes[0] {
            UiNode::Slider {
                name,
                label,
                min,
                max,
                step,
                value,
            } => {
                assert_eq!(name, "rate");
                assert_eq!(label, "Zins");
                assert_eq!((*min, *max), (0.0, 10.0));
                assert_eq!(*step, Some(0.5));
                assert_eq!(*value, Some(2.0));
            }
            other => panic!("expected slider, got {other:?}"),
        }
        match &nodes[1] {
            UiNode::Slider { step, value, .. } => {
                assert!(step.is_none());
                assert!(value.is_none());
            }
            other => panic!("expected slider, got {other:?}"),
        }
        match &nodes[2] {
            UiNode::Number { name, value, .. } => {
                assert_eq!(name, "years");
                assert_eq!(*value, Some(10.0));
            }
            other => panic!("expected number, got {other:?}"),
        }
        match &nodes[3] {
            UiNode::Number { value, .. } => assert!(value.is_none()),
            other => panic!("expected number, got {other:?}"),
        }
        match &nodes[4] {
            UiNode::Select { options, value, .. } => {
                assert_eq!(options, &vec!["1".to_string(), "2".to_string()]);
                assert_eq!(value.as_deref(), Some("2"));
            }
            other => panic!("expected select, got {other:?}"),
        }
        match &nodes[5] {
            UiNode::Computed {
                label,
                expr,
                decimals,
                unit,
            } => {
                assert_eq!(label, "Ergebnis");
                assert_eq!(expr, "rate*years");
                assert_eq!(*decimals, Some(2));
                assert_eq!(unit.as_deref(), Some("€"));
            }
            other => panic!("expected computed, got {other:?}"),
        }
        match &nodes[6] {
            UiNode::Computed { decimals, unit, .. } => {
                assert!(decimals.is_none());
                assert!(unit.is_none());
            }
            other => panic!("expected computed, got {other:?}"),
        }
        match &nodes[7] {
            UiNode::LineChart {
                title,
                x_label,
                y_label,
                series,
            } => {
                assert_eq!(title.as_deref(), Some("T"));
                assert_eq!(x_label.as_deref(), Some("x"));
                assert_eq!(y_label.as_deref(), Some("y"));
                assert_eq!(series.len(), 1);
                assert_eq!(series[0].name, "s");
                assert_eq!(series[0].points, vec![[0.0, 1.0], [1.0, 2.5]]);
            }
            other => panic!("expected line_chart, got {other:?}"),
        }
        match &nodes[8] {
            UiNode::LineChart { series, .. } => assert!(series.is_empty()),
            other => panic!("expected line_chart, got {other:?}"),
        }
        match &nodes[9] {
            UiNode::PieChart { title, slices } => {
                assert_eq!(title.as_deref(), Some("P"));
                assert_eq!(slices.len(), 2);
                assert_eq!(slices[1].value, 1.0);
            }
            other => panic!("expected pie_chart, got {other:?}"),
        }
        match &nodes[10] {
            UiNode::Progress { label, value } => {
                assert_eq!(label, "Fortschritt");
                assert_eq!(*value, 0.42);
            }
            other => panic!("expected progress, got {other:?}"),
        }
        match &nodes[11] {
            UiNode::Checklist { items } => {
                assert_eq!(items.len(), 2);
                assert!(items[0].done);
                assert!(!items[1].done, "done defaults to false");
            }
            other => panic!("expected checklist, got {other:?}"),
        }
        match &nodes[12] {
            UiNode::Kv { rows } => {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0], ["Schlüssel".to_string(), "Wert".to_string()]);
            }
            other => panic!("expected kv, got {other:?}"),
        }
    }

    #[test]
    fn new_ui_nodes_inside_card_and_columns() {
        let text = "```ui\n{\"type\":\"card\",\"children\":[\
{\"type\":\"columns\",\"children\":[\
{\"type\":\"slider\",\"name\":\"a\",\"label\":\"A\",\"min\":0,\"max\":1},\
{\"type\":\"computed\",\"label\":\"C\",\"expr\":\"a*2\"}]},\
{\"type\":\"progress\",\"label\":\"P\",\"value\":1}]}\n```";
        let nodes = parse_ui_blocks(text);
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            UiNode::Card { children, .. } => {
                assert_eq!(children.len(), 2);
                match &children[0] {
                    UiNode::Columns { children } => assert_eq!(children.len(), 2),
                    other => panic!("expected columns, got {other:?}"),
                }
            }
            other => panic!("expected card, got {other:?}"),
        }
    }
}
