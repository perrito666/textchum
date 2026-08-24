//! Randomized consistency test for the document edit/undo/redo pipeline.
//!
//! A deterministic PRNG drives thousands of random edits against a
//! `Document` while a plain `String` mirrors every operation as the ground
//! truth. Invariants checked continuously:
//!
//! * document text equals the mirror after every edit;
//! * a full undo walk always lands exactly on the initial text;
//! * a full redo walk always lands back on the final text;
//! * undo/redo results (as UTF-16 edits) replayed onto a mirror keep it in
//!   sync — the same contract the macOS text view relies on.

use textchum_core::Document;

/// xorshift64*: tiny, deterministic, good enough for fuzzing.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Alphabet mixing ASCII, multi-byte chars, a surrogate-pair emoji, and
/// newlines, to stress every unit-conversion path.
const ALPHABET: &[&str] = &["a", "b", " ", "é", "🎉", "\n", "ß", "0"];

fn random_snippet(rng: &mut Rng) -> String {
    let len = rng.below(4);
    (0..len).map(|_| ALPHABET[rng.below(ALPHABET.len())]).collect()
}

/// A random valid UTF-16 range within `text` (avoiding surrogate splits by
/// only picking char boundaries, as AppKit does for real edits).
fn random_range(rng: &mut Rng, text: &str) -> (usize, usize) {
    let boundaries: Vec<usize> = {
        let mut cu = 0;
        let mut all = vec![0];
        for ch in text.chars() {
            cu += ch.len_utf16();
            all.push(cu);
        }
        all
    };
    let a = boundaries[rng.below(boundaries.len())];
    let b = boundaries[rng.below(boundaries.len())];
    (a.min(b), a.max(b))
}

/// Applies a UTF-16 range replacement to a plain String mirror.
fn apply_to_mirror(mirror: &str, start: usize, end: usize, replacement: &str) -> String {
    let units: Vec<u16> = mirror.encode_utf16().collect();
    let mut out: Vec<u16> = Vec::with_capacity(units.len());
    out.extend_from_slice(&units[..start]);
    out.extend(replacement.encode_utf16());
    out.extend_from_slice(&units[end..]);
    String::from_utf16(&out).expect("mirror edits never split surrogates")
}

#[test]
fn random_edits_undo_redo_stay_consistent() {
    let mut rng = Rng(0x5EED_CAFE_F00D_D00D);

    for round in 0..20 {
        let mut doc = Document::new();
        let mut mirror = String::new();
        let initial = mirror.clone();

        // Forward phase: random edits — some inside explicit groups —
        // mirrored and verified.
        let edits = 200 + rng.below(300);
        for _ in 0..edits {
            match rng.below(12) {
                0 => doc.break_undo_group(),
                1 => {
                    // A compound operation: several edits as one undo step.
                    doc.begin_edit_group();
                    for _ in 0..1 + rng.below(3) {
                        let (start, end) = random_range(&mut rng, &mirror);
                        let snippet = random_snippet(&mut rng);
                        doc.replace_utf16(start, end, &snippet).unwrap_or_else(|e| {
                            panic!("round {round}: rejected grouped edit: {e}")
                        });
                        mirror = apply_to_mirror(&mirror, start, end, &snippet);
                    }
                    doc.end_edit_group();
                    assert_eq!(doc.text(), mirror, "round {round}: diverged in group");
                }
                _ => {
                    let (start, end) = random_range(&mut rng, &mirror);
                    let snippet = random_snippet(&mut rng);
                    doc.replace_utf16(start, end, &snippet)
                        .unwrap_or_else(|e| panic!("round {round}: rejected valid edit: {e}"));
                    mirror = apply_to_mirror(&mirror, start, end, &snippet);
                    assert_eq!(doc.text(), mirror, "round {round}: text diverged mid-edit");
                }
            }
        }
        let final_text = mirror.clone();

        // Interleaved phase: random undo/redo walk, replaying the reported
        // edits onto the mirror exactly like a shell view would.
        for _ in 0..100 {
            let edits = if rng.below(2) == 0 { doc.undo() } else { doc.redo() };
            for edit in edits {
                mirror = apply_to_mirror(&mirror, edit.start_utf16, edit.end_utf16, &edit.text);
            }
            assert_eq!(doc.text(), mirror, "round {round}: replayed step diverged");
        }

        // Full unwind must reach the initial text; full replay the final.
        loop {
            let edits = doc.undo();
            if edits.is_empty() {
                break;
            }
            for edit in edits {
                mirror = apply_to_mirror(&mirror, edit.start_utf16, edit.end_utf16, &edit.text);
            }
        }
        assert_eq!(doc.text(), initial, "round {round}: full undo missed initial state");
        assert_eq!(mirror, initial, "round {round}: mirror missed initial state");

        loop {
            let edits = doc.redo();
            if edits.is_empty() {
                break;
            }
            for edit in edits {
                mirror = apply_to_mirror(&mirror, edit.start_utf16, edit.end_utf16, &edit.text);
            }
        }
        assert_eq!(doc.text(), final_text, "round {round}: full redo missed final state");
        assert_eq!(mirror, final_text, "round {round}: mirror missed final state");
    }
}
