import AppKit

// Apps launched from Finder inherit the minimal system PATH, which is
// missing Homebrew, npm, cargo, and friends — exactly where language
// servers and ctags live. Merge the login shell's PATH (plus a few
// well-known tool directories) before anything spawns a process.
adoptLoginShellPath()

// `--smoke-test` exercises the full Swift ↔ core round trip headlessly and
// exits; it is what CI runs, and a quick sanity check for humans.
if CommandLine.arguments.contains("--smoke-test") {
    // Top-level code runs on the main thread; assumeIsolated makes that
    // visible to the compiler.
    exit(MainActor.assumeIsolated { runSmokeTest() })
}

MainActor.assumeIsolated {
    let app = NSApplication.shared
    let delegate = AppDelegate()
    app.delegate = delegate
    app.setActivationPolicy(.regular)
    app.run()
}
