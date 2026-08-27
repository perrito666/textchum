//! Captures that grammars emit under names the theme's table does not
//! carry. Before the alias table these rendered as plain text.

fn styles_of(source: &str, language: &str) -> Vec<(String, u32)> {
    let mut doc = textchum_core::Document::new();
    doc.replace_utf16(0, 0, source).unwrap();
    doc.set_language(Some(language));
    let units: Vec<u16> = source.encode_utf16().collect();
    doc.highlights(0, units.len())
        .unwrap()
        .iter()
        .map(|s| {
            (
                String::from_utf16_lossy(&units[s.start_utf16..s.end_utf16]),
                s.style,
            )
        })
        .collect()
}

#[test]
fn toml_booleans_are_coloured() {
    let spans = styles_of("enabled = true\nname = \"harbor\"\n", "toml");
    let boolean = spans.iter().find(|(text, _)| text == "true");
    assert!(
        boolean.is_some(),
        "`true` produced no span at all: {spans:?}"
    );
    assert_eq!(
        boolean.unwrap().1,
        textchum_core::theme::resolve("constant.builtin").unwrap(),
        "a TOML boolean should paint as a builtin constant"
    );
}

#[test]
fn yaml_booleans_are_coloured() {
    let spans = styles_of("enabled: true\nport: 8080\n", "yaml");
    assert!(
        spans.iter().any(|(text, _)| text == "true"),
        "`true` produced no span at all: {spans:?}"
    );
}
