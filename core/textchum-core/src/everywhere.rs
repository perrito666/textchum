//! Search Everywhere: one box for files, symbols, commands and
//! settings, each in a group of its own, the group with the best match
//! first.
//!
//! The shells gather the candidates — the project's files, what the
//! language server answered, their own menus, the core's switches —
//! and this orders them, so both shells put the same rows in the same
//! places for the same query.

use crate::completion::match_quality;

/// What a row is, and the order groups take when their best matches
/// are as good as each other's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    File,
    Symbol,
    Command,
    Setting,
}

impl Kind {
    /// How many rows the group shows at most: enough to choose from,
    /// few enough that the next group is in view.
    fn room(self) -> usize {
        match self {
            Kind::File | Kind::Symbol | Kind::Command => 8,
            Kind::Setting => 5,
        }
    }

    /// The group's heading.
    pub fn title(self) -> String {
        match self {
            Kind::File => crate::i18n::tr("Files"),
            Kind::Symbol => crate::i18n::tr("Symbols"),
            Kind::Command => crate::i18n::tr("Commands"),
            Kind::Setting => crate::i18n::tr("Settings"),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::File => "files",
            Kind::Symbol => "symbols",
            Kind::Command => "commands",
            Kind::Setting => "settings",
        }
    }

    pub fn from_name(name: &str) -> Option<Kind> {
        [Kind::File, Kind::Symbol, Kind::Command, Kind::Setting].into_iter().find(|kind| kind.name() == name)
    }
}

/// How well `label` answers `query`, smaller being better; None when
/// it does not. A path is read by its last part first — `main.rs` is
/// found by `main` — and a menu path by its command — `File › Save As…`
/// by `save` — then as a whole.
fn quality(label: &str, query: &str) -> Option<u8> {
    let last = label.rsplit(['/', '›']).next().unwrap_or(label).trim();
    match (match_quality(last, query), match_quality(label, query)) {
        (Some(own), Some(whole)) => Some(own.min(whole + 1)),
        (Some(own), None) => Some(own),
        // Only the whole path matches: below every match of its name.
        (None, Some(whole)) => Some(whole + 1),
        (None, None) => None,
    }
}

/// The rows to show for `query`: for each group that has a match, its
/// kind and the indices of its matching labels, best first and as many
/// as the group has room for. Groups come best match first. Within a
/// group, rows that match equally keep the order they were given in,
/// which is the order the source ranked them. An empty query keeps
/// every group in its given order, the first rows of each.
pub fn arrange(query: &str, groups: &[(Kind, Vec<String>)]) -> Vec<(Kind, Vec<usize>)> {
    let query = query.trim();
    let mut arranged: Vec<(u8, Kind, Vec<usize>)> = groups
        .iter()
        .filter_map(|(kind, labels)| {
            let mut scored: Vec<(u8, usize)> = labels
                .iter()
                .enumerate()
                .filter_map(|(at, label)| Some((quality(label, query)?, at)))
                .collect();
            scored.sort();
            scored.truncate(kind.room());
            let best = scored.first()?.0;
            Some((best, *kind, scored.into_iter().map(|(_, at)| at).collect()))
        })
        .collect();
    arranged.sort_by_key(|(best, kind, _)| (*best, *kind));
    arranged.into_iter().map(|(_, kind, rows)| (kind, rows)).collect()
}

/// [`arrange`] across the C boundary: `groups_json` is `[{kind,
/// labels}]` with kind one of `files`, `symbols`, `commands`,
/// `settings`; the answer is `[{kind, title, rows}]`, rows being
/// indices into that group's labels.
pub fn arrange_json(query: &str, groups_json: &str) -> String {
    let parsed: serde_json::Value = serde_json::from_str(groups_json).unwrap_or_default();
    let groups: Vec<(Kind, Vec<String>)> = parsed
        .as_array()
        .map(|groups| {
            groups
                .iter()
                .filter_map(|group| {
                    let kind = Kind::from_name(group["kind"].as_str()?)?;
                    let labels = group["labels"]
                        .as_array()?
                        .iter()
                        .map(|label| label.as_str().unwrap_or_default().to_owned())
                        .collect();
                    Some((kind, labels))
                })
                .collect()
        })
        .unwrap_or_default();
    let arranged: Vec<serde_json::Value> = arrange(query, &groups)
        .into_iter()
        .map(|(kind, rows)| serde_json::json!({"kind": kind.name(), "title": kind.title(), "rows": rows}))
        .collect();
    serde_json::Value::Array(arranged).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn the_group_with_the_best_match_comes_first() {
        let groups = vec![
            (Kind::File, labels(&["src/server.rs", "docs/save.md"])),
            (Kind::Command, labels(&["File › Save", "File › Save As…", "Edit › Undo"])),
            (Kind::Setting, labels(&["Save even when a save preprocessor fails"])),
        ];
        let arranged = arrange("save", &groups);
        let order: Vec<Kind> = arranged.iter().map(|(kind, _)| *kind).collect();
        // "Save" is the whole of a command, and only the start of a file
        // name and of a setting.
        assert_eq!(order, [Kind::Command, Kind::File, Kind::Setting]);
        assert_eq!(arranged[0].1, [0, 1], "the exact command first, Undo not at all");
        assert_eq!(arranged[1].1, [1], "the file whose name starts with it");
    }

    #[test]
    fn a_path_is_found_by_its_name_before_its_directories() {
        let groups = vec![(Kind::File, labels(&["main/other.rs", "src/main.rs"]))];
        assert_eq!(arrange("main", &groups)[0].1, [1, 0]);
    }

    #[test]
    fn groups_have_room_for_a_few_and_nothing_typed_keeps_their_order() {
        let many: Vec<String> = (0..20).map(|n| format!("file{n}.rs")).collect();
        let groups = vec![(Kind::Setting, labels(&["Show line numbers"])), (Kind::File, many)];
        let arranged = arrange("", &groups);
        assert_eq!(arranged[0].0, Kind::File, "equal matches: files before settings");
        assert_eq!(arranged[0].1, (0..8).collect::<Vec<_>>());
        assert!(arrange("zzz", &groups).is_empty());
    }

    #[test]
    fn across_the_c_boundary() {
        let json = r#"[{"kind": "commands", "labels": ["Edit › Undo", "Edit › Redo"]}, {"kind": "nonsense", "labels": []}]"#;
        let arranged: serde_json::Value = serde_json::from_str(&arrange_json("redo", json)).unwrap();
        assert_eq!(arranged[0]["kind"], "commands");
        assert_eq!(arranged[0]["rows"], serde_json::json!([1]));
        assert_eq!(arranged.as_array().unwrap().len(), 1);
    }
}
