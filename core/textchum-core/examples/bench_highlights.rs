//! Rough timings for the per-keystroke work on a large file:
//! `cargo run --release --example bench_highlights <file>`.

fn main() {
    let path = std::env::args().nth(1).expect("a file to time");
    let source = std::fs::read_to_string(&path).expect("readable");
    let mut doc = textchum_core::Document::new();
    doc.replace_utf16(0, 0, &source).unwrap();
    doc.set_language(Some("go"));
    let length = source.encode_utf16().count();

    let start = std::time::Instant::now();
    for _ in 0..20 {
        let _ = doc
            .highlights(length / 2, (length / 2 + 16_000).min(length))
            .unwrap();
    }
    println!("highlights (viewport+margin): {:?}/call", start.elapsed() / 20);

    let start = std::time::Instant::now();
    for _ in 0..100 {
        let _ = doc.context_lines(1400, 5);
    }
    println!("context_lines: {:?}/call", start.elapsed() / 100);

    let start = std::time::Instant::now();
    for _ in 0..20 {
        let _ = doc.fold_ranges();
    }
    println!("fold_ranges: {:?}/call", start.elapsed() / 20);

    // One keystroke in the middle of the file, as the choke point does it.
    let start = std::time::Instant::now();
    for i in 0..50 {
        doc.replace_utf16(length / 2, length / 2, "x").unwrap();
        let _ = i;
    }
    println!("replace one char: {:?}/call", start.elapsed() / 50);
}
