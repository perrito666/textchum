import AppKit
import TextchumKit

// Apps launched from Finder inherit the minimal system PATH, which is
// missing Homebrew, npm, cargo, and friends — exactly where language
// servers and ctags live. Merge the login shell's PATH (plus a few
// well-known tool directories) before anything spawns a process.
adoptLoginShellPath()

// `--emit-theme [file]` writes a complete starter theme — every styled
// capture, default palette — and exits. Making a theme is "generate,
// open, change colors", never "guess the schema". Without a file the
// JSON goes to stdout.
if let flag = CommandLine.arguments.firstIndex(of: "--emit-theme") {
    let template = TextchumKit.CoreTheme.templateJSON + "\n"
    let target = CommandLine.arguments.dropFirst(flag + 1).first
    if let target, target != "-" {
        do {
            try template.write(toFile: target, atomically: true, encoding: .utf8)
            print("wrote \(target)")
        } catch {
            FileHandle.standardError.write(Data("could not write \(target): \(error)\n".utf8))
            exit(1)
        }
    } else {
        print(template, terminator: "")
    }
    exit(0)
}

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
