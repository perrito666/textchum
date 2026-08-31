//! The pinned context: which lines answer "where am I?".

fn document(language: &str, source: &str) -> textchum_core::Document {
    let mut doc = textchum_core::Document::new();
    doc.replace_utf16(0, 0, source).unwrap();
    doc.set_language(Some(language));
    doc
}

const PYTHON: &str = "\
class Greeter:
    name = \"world\"

    def greet(self):
        for word in self.name:
            print(word)
        return None
";

#[test]
fn the_class_and_the_def_pin_for_a_line_inside_both() {
    let doc = document("python", PYTHON);
    // Line 5 is print(word): inside the class, the def and the for.
    assert_eq!(doc.context_lines(5, 5), vec![0, 3, 4]);
}

#[test]
fn a_construct_starting_on_the_visible_line_is_not_context() {
    let doc = document("python", PYTHON);
    // Line 1 is a statement in the class body: the class pins, the
    // statement does not, and neither does the body itself.
    assert_eq!(doc.context_lines(1, 5), vec![0]);
}

#[test]
fn the_cap_keeps_the_innermost_rows() {
    let doc = document("python", PYTHON);
    assert_eq!(doc.context_lines(5, 2), vec![3, 4]);
}

#[test]
fn the_pins_cover_lines_of_their_own() {
    // With line 3 at the top, the class line pins; that pin covers
    // line 3, so the def under it becomes context too, and the for
    // under that. The pins settle on the deeper answer: a pin held one
    // line long beats a breadcrumb with a step missing.
    let doc = document("python", PYTHON);
    assert_eq!(doc.context_lines(3, 5), vec![0, 3, 4]);
}

#[test]
fn plain_text_and_the_top_of_the_file_pin_nothing() {
    let mut plain = textchum_core::Document::new();
    plain.replace_utf16(0, 0, PYTHON).unwrap();
    assert_eq!(plain.context_lines(5, 5), Vec::<usize>::new());
    let doc = document("python", PYTHON);
    assert_eq!(doc.context_lines(0, 5), Vec::<usize>::new());
}

#[test]
fn rust_nesting_pins_the_impl_and_the_fn() {
    let source = "\
impl Item {
    fn label(&self) -> String {
        match self.kind {
            Kind::Plain => String::new(),
        }
    }
}
";
    let doc = document("rust", source);
    assert_eq!(doc.context_lines(3, 5), vec![0, 1, 2]);
}
