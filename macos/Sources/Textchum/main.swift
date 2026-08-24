import AppKit

// `--smoke-test` exercises the full Swift ↔ core round trip headlessly and
// exits; it is what CI runs, and a quick sanity check for humans.
if CommandLine.arguments.contains("--smoke-test") {
    // Top-level code runs on the main thread; assumeIsolated makes that
    // visible to the compiler.
    exit(MainActor.assumeIsolated { runSmokeTest() })
}

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.regular)
app.run()
