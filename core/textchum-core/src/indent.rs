//! Two indentation decisions, shared so both shells behave alike.
//!
//! Both are about the leading whitespace of a line and nowhere else,
//! which is what makes them safe: inside the text they do nothing
//! surprising, because they do nothing at all.

/// The width of `text` in columns, with tabs advancing to the next
/// multiple of `tab_width`.
pub fn column_width(text: &str, tab_width: usize) -> usize {
    let tab_width = tab_width.max(1);
    let mut column = 0;
    for character in text.chars() {
        if character == '\t' {
            column += tab_width - (column % tab_width);
        } else {
            column += 1;
        }
    }
    column
}

/// How many characters backspace should remove, given the text of the
/// line before the caret.
///
/// The rule other editors use, and the reason it never surprises
/// anyone: **when everything between the start of the line and the
/// caret is whitespace**, backspace deletes back to the previous tab
/// stop — the nearest smaller multiple of the tab width. Anywhere else
/// in the line it deletes one character, as always. It is the position
/// that decides, not a modifier and not a mode: inside the text it
/// cannot eat more than a character, and inside the indentation there
/// is nothing to lose but indentation.
///
/// A line indented with tabs already gets this from the single tab
/// character, so a run containing one is left alone rather than having
/// its width guessed at.
pub fn backspace_width(before_caret: &str, tab_width: usize) -> usize {
    let tab_width = tab_width.max(1);
    if before_caret.is_empty() {
        return 0;
    }
    if !before_caret.chars().all(|c| c == ' ') {
        return 1;
    }
    let column = before_caret.chars().count();
    let remainder = column % tab_width;
    if remainder == 0 { tab_width } else { remainder }
}

/// The indentation a line should take when asked to line up with the
/// block above it.
///
/// `previous` is the nearest non-blank line above, `current_indent` the
/// line's own leading whitespace. Lining up with the block is what is
/// wanted most of the time; when the line is already there, one level
/// deeper is the only other thing the request can mean, so it is not a
/// dead end.
pub fn aligned_indent(
    previous: Option<&str>,
    current_indent: &str,
    tab_width: usize,
    use_tabs: bool,
) -> String {
    let tab_width = tab_width.max(1);
    let target = previous
        .map(|line| column_width(leading_whitespace(line), tab_width))
        .unwrap_or(0);
    let current = column_width(current_indent, tab_width);
    let wanted = if current < target {
        target
    } else {
        // Already level with the block, or past it: the next stop.
        current + tab_width - (current % tab_width)
    };
    indent_of_width(wanted, tab_width, use_tabs)
}

/// The leading whitespace of a line.
pub fn leading_whitespace(line: &str) -> &str {
    let end = line
        .char_indices()
        .find(|(_, c)| !c.is_whitespace())
        .map(|(at, _)| at)
        .unwrap_or(line.len());
    &line[..end]
}

/// A run of whitespace `width` columns wide.
fn indent_of_width(width: usize, tab_width: usize, use_tabs: bool) -> String {
    if use_tabs {
        let tabs = width / tab_width;
        let spaces = width % tab_width;
        "\t".repeat(tabs) + &" ".repeat(spaces)
    } else {
        " ".repeat(width)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backspace_inside_the_text_takes_one_character() {
        assert_eq!(backspace_width("    let x", 4), 1);
        assert_eq!(backspace_width("x", 4), 1);
        // Whitespace *after* something is not indentation.
        assert_eq!(backspace_width("let x = ", 4), 1);
    }

    #[test]
    fn backspace_in_the_indentation_takes_a_whole_level() {
        assert_eq!(backspace_width("    ", 4), 4);
        assert_eq!(backspace_width("        ", 4), 4);
        // Off the stop: back to it, not past it.
        assert_eq!(backspace_width("      ", 4), 2);
        assert_eq!(backspace_width("     ", 4), 1);
        assert_eq!(backspace_width("  ", 4), 2);
        assert_eq!(backspace_width(" ", 4), 1);
    }

    #[test]
    fn a_tab_indented_line_is_left_to_its_tab_character() {
        // One character already is one level; guessing at a mixed run's
        // width would be worse than leaving it.
        assert_eq!(backspace_width("\t", 4), 1);
        assert_eq!(backspace_width("\t  ", 4), 1);
        assert_eq!(backspace_width("  \t", 4), 1);
    }

    #[test]
    fn backspace_at_the_start_of_a_line_takes_nothing_of_this_line() {
        // The caller joins with the line above, as backspace always has.
        assert_eq!(backspace_width("", 4), 0);
    }

    #[test]
    fn other_tab_widths_work_the_same_way() {
        assert_eq!(backspace_width("  ", 2), 2);
        assert_eq!(backspace_width("   ", 2), 1);
        assert_eq!(backspace_width("        ", 8), 8);
    }

    #[test]
    fn columns_count_tabs_to_the_next_stop() {
        assert_eq!(column_width("", 4), 0);
        assert_eq!(column_width("  ", 4), 2);
        assert_eq!(column_width("\t", 4), 4);
        assert_eq!(column_width("  \t", 4), 4);
        assert_eq!(column_width("\t\t", 4), 8);
        assert_eq!(column_width("\t ", 4), 5);
    }

    #[test]
    fn aligning_matches_the_block_above() {
        assert_eq!(aligned_indent(Some("        deep()"), "", 4, false), "        ");
        assert_eq!(aligned_indent(Some("    shallow()"), "", 4, false), "    ");
        // Nothing above: the first stop.
        assert_eq!(aligned_indent(None, "", 4, false), "    ");
    }

    #[test]
    fn aligning_again_goes_one_level_deeper() {
        // Already level with the block: the request can only mean the
        // next level in.
        assert_eq!(aligned_indent(Some("    thing()"), "    ", 4, false), "        ");
        // Past it already: still the next stop, not back to the block.
        assert_eq!(aligned_indent(Some("    thing()"), "        ", 4, false), "            ");
    }

    #[test]
    fn a_partial_indent_is_completed_before_it_is_deepened() {
        // Two spaces under a four-space block: line up first.
        assert_eq!(aligned_indent(Some("    thing()"), "  ", 4, false), "    ");
    }

    #[test]
    fn a_tab_document_gets_tabs_back() {
        assert_eq!(aligned_indent(Some("\t\tdeep()"), "", 4, true), "\t\t");
        assert_eq!(aligned_indent(Some("\tthing()"), "\t", 4, true), "\t\t");
        // A width that is not a whole number of tabs keeps the
        // remainder in spaces rather than lying about the column.
        assert_eq!(aligned_indent(Some("      six()"), "", 4, true), "\t  ");
    }

    #[test]
    fn leading_whitespace_is_what_comes_before_the_first_word() {
        assert_eq!(leading_whitespace("    x"), "    ");
        assert_eq!(leading_whitespace("\t\tx"), "\t\t");
        assert_eq!(leading_whitespace("x"), "");
        assert_eq!(leading_whitespace("    "), "    ");
        assert_eq!(leading_whitespace(""), "");
    }
}
