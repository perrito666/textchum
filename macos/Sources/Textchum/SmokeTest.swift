// AppKit for the Settings window check below: this test builds a
// real window, not a stand-in.
import AppKit
import Combine
import Foundation
import SwiftUI
import TextchumKit

/// Headless verification that the shell and the core actually talk to each
/// other: text edits round-trip through the core (including a UTF-16 range
/// edit over a surrogate pair, the trickiest unit conversion we do), and an
/// asynchronous event makes it from a core thread back to the main queue.
///
/// Returns a process exit code: 0 on success.
/// Turns the runloop until `condition` holds, or the deadline passes.
/// `RunLoop.run(until:)` on its own returns immediately when no source
/// or timer is registered, which makes a plain sleep-and-check flaky on
/// a machine with nothing else going on.
@MainActor
func spin(untilTrue condition: () -> Bool, seconds: TimeInterval = 2) {
    let deadline = Date().addingTimeInterval(seconds)
    while !condition(), Date() < deadline {
        RunLoop.main.run(mode: .default, before: Date().addingTimeInterval(0.01))
        usleep(2000)
    }
}

@MainActor
func runSmokeTest() -> Int32 {
    print("textchum core \(Core.version)")

    // Buffer round trip.
    let buffer = CoreBuffer()
    do {
        try buffer.insert("hello world", atByteOffset: 0)
        try buffer.insert(",", atByteOffset: 5)
        try buffer.replace(utf16Range: NSRange(location: 0, length: 5), with: "🎉")
        try buffer.replace(utf16Range: NSRange(location: 0, length: 2), with: "hi")
    } catch {
        print("FAIL: core rejected a valid edit: \(error)")
        return 1
    }
    let expected = "hi, world"
    guard buffer.text == expected, buffer.lengthInBytes == expected.utf8.count else {
        print("FAIL: buffer round trip: got \(buffer.text.debugDescription)")
        return 1
    }
    print("buffer round trip ok (\(buffer.lengthInBytes) bytes)")

    // Invalid edits must be rejected, not crash.
    if (try? buffer.deleteBytes(from: 5, to: 2)) != nil {
        print("FAIL: core accepted an inverted range")
        return 1
    }
    print("input validation ok")

    // Document lifecycle: edit, undo, save, reopen — through the C boundary.
    let smokeDir = FileManager.default.temporaryDirectory
        .appendingPathComponent("textchum-smoke-\(ProcessInfo.processInfo.processIdentifier)")
    do {
        try FileManager.default.createDirectory(at: smokeDir, withIntermediateDirectories: true)
        let filePath = smokeDir.appendingPathComponent("smoke.txt").path

        let document = CoreDocument()
        guard !document.isDirty else {
            print("FAIL: untitled document born dirty")
            return 1
        }
        try document.replace(utf16Range: NSRange(location: 0, length: 0), with: "hello")
        // Contiguous typing coalesces into one undo step; break the run so
        // the two inserts stay separate steps.
        document.breakUndoCoalescing()
        try document.replace(utf16Range: NSRange(location: 5, length: 0), with: " world")
        guard document.isDirty, document.canUndo else {
            print("FAIL: edits not reflected in dirty/undo state")
            return 1
        }
        let undone = document.undo()
        guard undone.count == 1, undone[0].replacement.isEmpty else {
            print("FAIL: undo returned no edit")
            return 1
        }
        guard document.text == "hello", !document.redo().isEmpty,
            document.text == "hello world"
        else {
            print("FAIL: undo/redo walk: got \(document.text.debugDescription)")
            return 1
        }

        // Grouped edits (the shape of a replace-all) undo as one step.
        document.beginEditGroup()
        try document.replace(utf16Range: NSRange(location: 6, length: 5), with: "W")
        try document.replace(utf16Range: NSRange(location: 0, length: 5), with: "H")
        document.endEditGroup()
        guard document.text == "H W" else {
            print("FAIL: grouped edits: got \(document.text.debugDescription)")
            return 1
        }
        let groupUndo = document.undo()
        guard groupUndo.count == 2, document.text == "hello world" else {
            print("FAIL: group undo: \(groupUndo.count) edits, \(document.text.debugDescription)")
            return 1
        }
        try document.save(to: filePath)
        guard !document.isDirty, document.path == filePath else {
            print("FAIL: save did not clear dirty state or adopt path")
            return 1
        }

        let reopened = try CoreDocument(contentsOf: filePath)
        guard reopened.text == "hello world", reopened.encodingName == "UTF-8" else {
            print("FAIL: reopened document mismatch: \(reopened.text.debugDescription)")
            return 1
        }

        // External change → reload picks it up as one clean, undoable step.
        try "changed elsewhere".write(toFile: filePath, atomically: true, encoding: .utf8)
        guard let reloadEdit = try reopened.reload(), reloadEdit.replacement == "changed elsewhere",
            reopened.text == "changed elsewhere", !reopened.isDirty
        else {
            print("FAIL: reload after external change")
            return 1
        }
        guard try reopened.reload() == nil else {
            print("FAIL: reload of unchanged file should be a no-op")
            return 1
        }
        guard reopened.undo().count == 1, reopened.text == "hello world" else {
            print("FAIL: reload not undoable")
            return 1
        }
        print("document lifecycle ok (edit/undo/group/save/reopen/reload)")
        try? FileManager.default.removeItem(at: smokeDir)
    } catch {
        print("FAIL: document lifecycle: \(error)")
        return 1
    }

    // Syntax highlighting: detection by extension, styled spans, injections.
    do {
        let syntaxDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("textchum-smoke-syn-\(ProcessInfo.processInfo.processIdentifier)")
        try FileManager.default.createDirectory(at: syntaxDir, withIntermediateDirectories: true)
        let rustPath = syntaxDir.appendingPathComponent("smoke.rs").path
        try "// comment\nfn main() { let s = \"hi\"; }\n".write(
            toFile: rustPath, atomically: true, encoding: .utf8)

        guard !CoreTheme.styles.isEmpty else {
            print("FAIL: style table is empty")
            return 1
        }
        let rustDoc = try CoreDocument(contentsOf: rustPath)
        guard rustDoc.languageName == "rust" else {
            print("FAIL: language not detected: \(String(describing: rustDoc.languageName))")
            return 1
        }
        let spans = rustDoc.highlights(in: NSRange(location: 0, length: rustDoc.lengthInUTF16))
        guard !spans.isEmpty, spans.allSatisfy({ CoreTheme.styles.indices.contains($0.styleIndex) })
        else {
            print("FAIL: no spans, or style index out of table bounds")
            return 1
        }

        let markdown = CoreDocument()
        try markdown.replace(
            utf16Range: NSRange(location: 0, length: 0),
            with: "# Title\n\n```rust\nfn x() {}\n```\n")
        guard markdown.setLanguage("markdown") else {
            print("FAIL: markdown language rejected")
            return 1
        }
        let mdSpans = markdown.highlights(
            in: NSRange(location: 0, length: markdown.lengthInUTF16))
        guard mdSpans.count > 2 else {
            print("FAIL: markdown spans missing (injections broken?)")
            return 1
        }
        print("syntax highlighting ok (\(spans.count) rust spans, \(mdSpans.count) md spans)")
        try? FileManager.default.removeItem(at: syntaxDir)
    } catch {
        print("FAIL: syntax highlighting: \(error)")
        return 1
    }

    // Workspace: project roots resolve to the nearest marker.
    do {
        let wsDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("textchum-smoke-ws-\(ProcessInfo.processInfo.processIdentifier)")
        let project = wsDir.appendingPathComponent("proj")
        let nested = project.appendingPathComponent("src/deep")
        try FileManager.default.createDirectory(at: nested, withIntermediateDirectories: true)
        try "".write(
            toFile: project.appendingPathComponent("Cargo.toml").path,
            atomically: true, encoding: .utf8)
        let file = nested.appendingPathComponent("main.rs").path
        try "fn main() {}".write(toFile: file, atomically: true, encoding: .utf8)

        guard CoreWorkspace.projectRoot(forPath: file) == project.path else {
            print("FAIL: project root resolution")
            return 1
        }
        guard CoreWorkspace.projectRoot(forPath: "/") == nil else {
            print("FAIL: filesystem root must be loose")
            return 1
        }
        print("workspace ok (nearest-marker project roots)")
        try? FileManager.default.removeItem(at: wsDir)
    } catch {
        print("FAIL: workspace: \(error)")
        return 1
    }

    // Configuration: round trip, hand-edit preservation, breakage recovery.
    do {
        let configDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("textchum-smoke-cfg-\(ProcessInfo.processInfo.processIdentifier)")
        try FileManager.default.createDirectory(at: configDir, withIntermediateDirectories: true)
        let configPath = configDir.appendingPathComponent("config.json").path

        let config = CoreConfig(path: configPath)
        guard config.loadWarning == nil, config.fontSize == 13, config.tabWidth == 4 else {
            print("FAIL: fresh config did not load defaults")
            return 1
        }
        config.fontFamily = "Menlo"
        config.tabWidth = 8
        config.appearance = .dark
        try config.save()
        let reloaded = CoreConfig(path: configPath)
        guard reloaded.fontFamily == "Menlo", reloaded.tabWidth == 8,
            reloaded.appearance == .dark
        else {
            print("FAIL: config round trip")
            return 1
        }

        // Break the file by hand; the app must fall back to defaults,
        // warn, preserve the broken content, and back it up on next save.
        try "{ broken".write(toFile: configPath, atomically: true, encoding: .utf8)
        let broken = CoreConfig(path: configPath)
        guard broken.loadWarning != nil, broken.tabWidth == 4 else {
            print("FAIL: broken config not detected or defaults not applied")
            return 1
        }
        broken.tabWidth = 2
        try broken.save()
        let backup = try String(contentsOfFile: configPath + ".bak", encoding: .utf8)
        guard backup == "{ broken", CoreConfig(path: configPath).tabWidth == 2 else {
            print("FAIL: broken config not backed up before overwrite")
            return 1
        }
        print("configuration ok (round trip, breakage recovery, backup)")

        // Live reload: an external rewrite lands after reload(), and
        // per-project editor overrides resolve into the settings.
        let live = CoreConfig(path: configPath)
        try #"{"editor": {"tab_width": 6}, "workspace": {"projects": {"/proj": {"editor": {"tab_width": 2, "font_size": 15}}}}}"#
            .write(toFile: configPath, atomically: true, encoding: .utf8)
        live.reload()
        guard live.tabWidth == 6 else {
            print("FAIL: config reload did not follow the disk")
            return 1
        }
        guard EditorSettings(config: live, projectRoot: "/proj").tabWidth == 2,
            EditorSettings(config: live, projectRoot: "/elsewhere").tabWidth == 6
        else {
            print("FAIL: per-project editor overrides did not resolve")
            return 1
        }
        print("config reload + project overrides ok")

        // The session belongs to the configuration's profile, so a
        // scratch run cannot overwrite the real app's session.
        let realDirectory = SessionStore.directory
        SessionStore.useProfile(ofConfigAt: configPath)
        guard SessionStore.path == configDir.appendingPathComponent("session.json").path
        else {
            print("FAIL: session did not follow the configuration's profile")
            return 1
        }
        var session = SessionState()
        session.windows = [.init(path: "/tmp/one.txt", caret: 3, scroll: 0)]
        SessionStore.save(session)
        guard SessionStore.load()?.windows.first?.path == "/tmp/one.txt" else {
            print("FAIL: session round trip")
            return 1
        }
        SessionStore.directory = realDirectory
        print("session profile ok (scoped to the configuration's directory)")
        try? FileManager.default.removeItem(at: configDir)
    } catch {
        print("FAIL: configuration: \(error)")
        return 1
    }

    // Themes: built-ins switch the style table, user JSON applies, the
    // template round-trips, breakage reports instead of applying.
    let builtins = CoreTheme.builtinNames
    let defaultStyles = CoreTheme.styles
    guard builtins.count >= 3, builtins.first == "Textchum", !defaultStyles.isEmpty else {
        print("FAIL: built-in theme set (\(builtins))")
        return 1
    }
    guard CoreTheme.setBuiltin(named: "Graphite"),
        CoreTheme.styles.first?.lightRGBA != defaultStyles.first?.lightRGBA
    else {
        print("FAIL: switching themes did not change the style table")
        return 1
    }
    guard
        CoreTheme.setJSON(
            ##"{"name": "T", "styles": {"attribute": {"light": "#123456"}}}"##) == nil,
        CoreTheme.styles.first?.lightRGBA == 0x123456FF,
        CoreTheme.setJSON("{ nope") != nil,
        CoreTheme.styles.first?.lightRGBA == 0x123456FF
    else {
        print("FAIL: user theme JSON handling")
        return 1
    }
    guard CoreTheme.setJSON(CoreTheme.templateJSON) == nil,
        CoreTheme.styles.first?.lightRGBA == defaultStyles.first?.lightRGBA,
        CoreTheme.setBuiltin(named: "Textchum")
    else {
        print("FAIL: theme template does not reproduce the default palette")
        return 1
    }
    print("themes ok (\(builtins.count) built-ins, user JSON, template)")

    // Auto-indent: return inherits the line's indentation, deepens
    // after openers in the document's own style, and stays plain when
    // there is nothing to inherit.
    func newline(_ text: String, caret: Int, tabWidth: Int = 4) -> String? {
        DocumentController.autoIndentedNewline(
            in: text as NSString,
            selection: NSRange(location: caret, length: 0),
            tabWidth: tabWidth)
    }
    guard newline("    let x = 1", caret: 13) == "\n    ",
        newline("\tfn main() {", caret: 12) == "\n\t\t",
        newline("def f():", caret: 8, tabWidth: 2) == "\n  ",
        newline("plain line", caret: 10) == nil,
        newline("    early", caret: 2) == "\n  ",
        newline("if x {   ", caret: 9) == "\n    "
    else {
        print("FAIL: auto-indent rules")
        return 1
    }
    print("auto-indent ok (inherit, deepen, tab/space styles)")

    // New-with-format plumbing: the language list crosses the FFI, and
    // an untitled document can speak a language before its first save.
    guard CoreLanguages.all.contains(where: { $0.name == "rust" && $0.fileExtension == "rs" }),
        CoreLanguages.all.contains(where: { $0.name == "make" })
    else {
        print("FAIL: language list: \(CoreLanguages.all)")
        return 1
    }
    let untitled = CoreDocument()
    guard untitled.setLanguage("rust") else {
        print("FAIL: untitled document rejected a known language")
        return 1
    }
    try? untitled.replace(utf16Range: NSRange(location: 0, length: 0), with: "fn main() {}\n")
    guard !untitled.highlights(in: NSRange(location: 0, length: 13)).isEmpty else {
        print("FAIL: untitled rust document produced no highlights")
        return 1
    }
    print("new-with-format ok (language list + pre-save highlighting)")

    // Jump stack: back retraces origins, forward returns, and a fresh
    // jump rewrites the future from the current point.
    let stack = JumpStack()
    let locA = JumpLocation(path: "/a", line: 1, character: 0)
    let locB = JumpLocation(path: "/b", line: 2, character: 0)
    let locC = JumpLocation(path: "/c", line: 3, character: 0)
    stack.noteJump(from: locA)  // a → b
    stack.noteJump(from: locB)  // b → c
    guard stack.goBack(from: locC) == locB,
        stack.goBack(from: locB) == locA,
        stack.goForward(from: locA) == locB,
        stack.canGoForward
    else {
        print("FAIL: jump stack traversal")
        return 1
    }
    stack.noteJump(from: locB)  // a new jump from here…
    guard !stack.canGoForward, stack.goBack(from: locC) == locB else {
        print("FAIL: a fresh jump must discard the forward trail")
        return 1
    }
    print("jump stack ok (back, forward, truncation on new jump)")

    // Snippets: the core expands the body, hands back the first
    // placeholder to select, walks the stops on Tab, mirrors linked ones
    // and gives the keys back at the end.
    let snippetDoc = CoreDocument()
    let body = "let ${1:name} = ${1:name}.frob(${2:arg});$0"
    let expanded = snippetDoc.expandSnippet(body, at: 0)
    guard expanded == "let name = name.frob(arg);" else {
        print("FAIL: snippet expansion: \(expanded)")
        return 1
    }
    try? snippetDoc.replace(utf16Range: NSRange(location: 0, length: 0), with: expanded)
    guard snippetDoc.beginSnippet(at: 0) == NSRange(location: 4, length: 4),
        snippetDoc.isSnippetActive
    else {
        print("FAIL: snippet session did not start on the first placeholder")
        return 1
    }
    try? snippetDoc.replace(utf16Range: NSRange(location: 4, length: 4), with: "value")
    let mirrored = snippetDoc.syncSnippet()
    guard mirrored.count == 1, snippetDoc.text == "let value = value.frob(arg);" else {
        print("FAIL: linked stop did not mirror: \(snippetDoc.text)")
        return 1
    }
    guard snippetDoc.advanceSnippet(forward: true) == NSRange(location: 23, length: 3),
        snippetDoc.advanceSnippet(forward: true) == NSRange(location: 28, length: 0),
        !snippetDoc.isSnippetActive
    else {
        print("FAIL: tab did not walk the snippet to its exit")
        return 1
    }

    // A snippet with only an exit point is a caret position, not a mode.
    let exitDoc = CoreDocument()
    let exitText = exitDoc.expandSnippet("done()$0 end", at: 0)
    try? exitDoc.replace(utf16Range: NSRange(location: 0, length: 0), with: exitText)
    guard exitText == "done() end",
        exitDoc.beginSnippet(at: 0) == NSRange(location: 6, length: 0),
        !exitDoc.isSnippetActive
    else {
        print("FAIL: snippet exit point")
        return 1
    }

    // Escapes survive, and a body with no constructs is left alone.
    let escapeDoc = CoreDocument()
    guard escapeDoc.expandSnippet(#"cost \$5"#, at: 0) == "cost $5" else {
        print("FAIL: snippet escape")
        return 1
    }

    // A server's items arrive marked: a declared insertTextFormat
    // decides, and placeholders written without one speak for
    // themselves — but a bare dollar does not.
    let parsed = CompletionPopup.parse(
        resultJSON: """
            [{"label": "frob", "insertText": "frob(${1:x})", "insertTextFormat": 2},
             {"label": "cost", "insertText": "cost $5", "insertTextFormat": 1},
             {"label": "wrap", "insertText": "wrap(${1:x})"},
             {"label": "home", "insertText": "echo $HOME"}]
            """)
    let flags = parsed.map { "\($0.label):\($0.isSnippet)" }.sorted()
    guard flags == ["cost:false", "frob:true", "home:false", "wrap:true"] else {
        print("FAIL: snippet items not recognized: \(flags)")
        return 1
    }
    print("snippets ok (expansion, tabstops, linked stops, exit, escapes)")

    // Hugo: a post's headings outline it, and the prose the spell
    // checker reads excludes front matter and shortcode calls.
    let post = [
        "+++",
        "title = \"Harbor\"",
        "slug = \"harbr\"",
        "+++",
        "",
        "# Opening",
        "",
        "Prose with {{< figure src=\"a.png\" >}} inside.",
        "",
        "## Later",
    ].joined(separator: "\n")
    let headings = CoreWorkspace.markdownHeadings(in: post)
    guard headings.map(\.text) == ["Opening", "Later"],
        headings.first?.level == 1, headings.last?.level == 2
    else {
        print("FAIL: markdown headings: \(headings)")
        return 1
    }
    let skipped = CoreWorkspace.hugoNonProseRanges(in: post)
    let whole = NSRange(location: 0, length: (post as NSString).length)
    let prose = DocumentController.ranges(of: whole, excluding: skipped)
    let proseText = prose.map { (post as NSString).substring(with: $0) }.joined()
    guard skipped.count == 2, !proseText.contains("harbr"),
        !proseText.contains("{{<"), proseText.contains("Prose with")
    else {
        print("FAIL: hugo prose ranges: \(skipped) -> \(proseText)")
        return 1
    }
    print("hugo ok (headings outline, front matter and shortcodes out of the prose)")

    // Highlighting: the theme's typographic flags survive the trip to
    // the palette, and a document past the old colouring cap still has
    // spans to paint (viewport scoping decides how many are asked for,
    // but the core must answer for any offset).
    guard HighlightPalette.hasTypographicStyles else {
        print("FAIL: the built-in theme lost its bold/italic flags")
        return 1
    }
    guard let commentStyle = CoreTheme.commentStyleID else {
        print("FAIL: the theme has no comment capture")
        return 1
    }
    let commentTraits = HighlightPalette.traits(forStyle: Int(commentStyle))
    guard commentTraits.italic else {
        print("FAIL: comments are meant to be italic in the default theme")
        return 1
    }
    do {
        let big = CoreDocument()
        let unit = "/// a comment\npub fn f() {}\n"
        try big.replace(
            utf16Range: NSRange(location: 0, length: 0),
            with: String(repeating: unit, count: 12_000))
        big.setLanguage("rust")
        let length = big.lengthInUTF16
        guard length > 300_000 else {
            print("FAIL: test document too small to prove the point: \(length)")
            return 1
        }
        // Deep inside the document, far past the cap colouring used to
        // give up at.
        let window = NSRange(location: length - 20_000, length: 8_000)
        let spans = big.highlights(in: window)
        guard !spans.isEmpty,
            spans.contains(where: { $0.styleIndex == commentStyle })
        else {
            print("FAIL: no spans deep inside a large document")
            return 1
        }
        print("highlighting ok (typographic flags, spans at \(length) UTF-16 units)")
    } catch {
        print("FAIL: large-document highlighting: \(error)")
        return 1
    }

    // Async event round trip: core dispatch thread → main queue.
    var receivedSequence: UInt64?
    let coreApp = CoreApp { event in
        if case let .pong(sequence) = event {
            receivedSequence = sequence
            CFRunLoopStop(CFRunLoopGetMain())
        }
    }
    coreApp.ping(sequence: 42)
    // Give the main run loop up to five seconds to receive the pong.
    let outcome = CFRunLoopRunInMode(.defaultMode, 5.0, false)
    guard receivedSequence == 42, outcome == .stopped else {
        print("FAIL: pong not delivered (got \(String(describing: receivedSequence)))")
        return 1
    }
    print("event round trip ok (pong 42)")

    // The Settings window, built for real. It has been broken more than
    // once by adding a row to the tallest tab: a window that cannot be
    // resized and content that cannot scroll turns one more setting
    // into settings nobody can reach, and nothing complains at build
    // time. These two properties are what make that impossible.
    // A scratch profile: this must never read or write a real one.
    let settingsScratch = FileManager.default.temporaryDirectory
        .appendingPathComponent("textchum-smoke-settings-\(getpid()).json").path
    let settingsModel = SettingsModel(config: CoreConfig(path: settingsScratch))
    let settingsWindow = SettingsWindowController(model: settingsModel).window
    guard let settingsWindow else {
        print("FAIL: settings window was not created")
        return 1
    }
    guard settingsWindow.styleMask.contains(.resizable) else {
        print("FAIL: the settings window is not resizable")
        return 1
    }
    // Small enough to prove the content yields rather than pinning the
    // window open at the height of whatever was added last.
    guard settingsWindow.contentMinSize.height <= 400,
        settingsWindow.contentMinSize.width <= 640
    else {
        print("FAIL: settings minimum size is \(settingsWindow.contentMinSize)")
        return 1
    }
    print("settings window ok (resizable, minimum \(Int(settingsWindow.contentMinSize.width))x\(Int(settingsWindow.contentMinSize.height)))")
    // Only the window properties are asserted here. Whether the form
    // scrolls is a layout question, and a detached NSHostingController
    // never runs SwiftUI's layout — its scroll view reports a document
    // of zero — so a headless check of it would pass whatever the code
    // did. That half is verified by looking at the window; this half is
    // the part that can be verified honestly, and it is the part that
    // has actually regressed before.

    // One sidebar width across every window. The wiring needs a window
    // server, but the two ways it goes wrong are pure decisions and are
    // checked here: adopting a width already held sets two windows
    // answering each other forever, and adopting one while collapsed
    // reopens a navigator the user closed.
    guard Workbench.shouldAdoptSidebarWidth(320, current: 188, collapsed: false)
    else {
        print("FAIL: a different width from another window should be adopted")
        return 1
    }
    guard !Workbench.shouldAdoptSidebarWidth(188, current: 188, collapsed: false)
    else {
        print("FAIL: adopting the width already held is how a feedback loop starts")
        return 1
    }
    guard !Workbench.shouldAdoptSidebarWidth(188.2, current: 188, collapsed: false)
    else {
        print("FAIL: a sub-point difference is the same width coming back")
        return 1
    }
    guard !Workbench.shouldAdoptSidebarWidth(320, current: 0, collapsed: true)
    else {
        print("FAIL: a collapsed navigator must stay collapsed")
        return 1
    }
    print("sidebar width sync ok (adopts changes, ignores echoes and collapsed)")

    // Revealing a file in the tree publishes to SwiftUI, and the call
    // comes from windowDidBecomeKey, which AppKit sends in the middle of
    // a display cycle. State set while a view is being laid out from it
    // is what AttributeGraph ends the process over, so the change waits
    // for the next turn — and never happens at all when nothing moved.
    let tree = FileTreeState()
    var publishes = 0
    let watch = tree.objectWillChange.sink { _ in publishes += 1 }
    tree.reveal(path: "/site/content/posts/first.md", under: "/site")
    guard tree.expanded.isEmpty, publishes == 0 else {
        print("FAIL: revealing published during the call")
        return 1
    }
    // RunLoop.run(until:) returns at once when nothing is scheduled on
    // it, so the wait is for the work rather than for a length of time.
    spin(untilTrue: { tree.expanded.count == 3 })
    guard tree.expanded.count == 3, publishes > 0 else {
        print("FAIL: revealing did not expand the folders: \(tree.expanded)")
        return 1
    }
    let settled = publishes
    tree.reveal(path: "/site/content/posts/first.md", under: "/site")
    spin(untilTrue: { false }, seconds: 0.2)
    guard publishes == settled else {
        print("FAIL: revealing the same file again published anyway")
        return 1
    }
    watch.cancel()
    print("reveal in tree ok (deferred off the display cycle, quiet when nothing moved)")

    // Which keys a running snippet takes. The routing needs a window
    // server; the table it consults is a pure decision and is checked
    // here — a snippet that swallowed Return, or missed Shift-Tab,
    // would be a mode with the wrong way out.
    guard DocumentController.snippetKey(for: #selector(NSResponder.insertTab(_:)))
            == .nextStop,
        DocumentController.snippetKey(for: #selector(NSResponder.insertBacktab(_:)))
            == .previousStop,
        DocumentController.snippetKey(for: #selector(NSResponder.cancelOperation(_:)))
            == .cancel,
        DocumentController.snippetKey(for: #selector(NSResponder.insertNewline(_:))) == nil,
        DocumentController.snippetKey(for: #selector(NSResponder.moveDown(_:))) == nil
    else {
        print("FAIL: snippet key routing")
        return 1
    }
    print("snippet keys ok (tab, shift-tab, escape; everything else falls through)")

    // Importing a theme from another editor: the file goes in, a theme
    // of ours comes out in the themes directory, and it is one the core
    // can then wear.
    let importDir = FileManager.default.temporaryDirectory
        .appendingPathComponent("textchum-import-\(ProcessInfo.processInfo.processIdentifier)")
    let importSource = importDir.appendingPathComponent("source")
    let importThemes = importDir.appendingPathComponent("themes")
    try? FileManager.default.createDirectory(
        at: importSource, withIntermediateDirectories: true)
    let vscodeTheme = """
        {
          // A theme as VS Code writes them, comments and all.
          "name": "Smoke Night",
          "type": "dark",
          "tokenColors": [
            {"scope": "comment", "settings": {"foreground": "#5A6472", "fontStyle": "italic"}},
            {"scope": "keyword", "settings": {"foreground": "#C678DD"}},
            {"scope": "string", "settings": {"foreground": "#98C379"}},
          ]
        }
        """
    try? vscodeTheme.write(
        to: importSource.appendingPathComponent("night.json"), atomically: true, encoding: .utf8)
    let imported = CoreTheme.importThemes(
        at: importSource.path, from: .vsCode, into: importThemes.path)
    guard imported.errors.isEmpty, imported.written == ["Smoke Night"],
        imported.appearances == ["dark"]
    else {
        print("FAIL: theme import: \(imported.written) \(imported.errors)")
        return 1
    }
    guard
        let importedJSON = try? String(
            contentsOf: importThemes.appendingPathComponent("Smoke Night.json"), encoding: .utf8),
        CoreTheme.setJSON(importedJSON) == nil
    else {
        print("FAIL: an imported theme must be one the core can wear")
        return 1
    }
    // A keyword's colour reaches the kinds of keyword the source never
    // named separately.
    guard let conditional = CoreTheme.styleID(for: "conditional"),
        CoreTheme.styles[Int(conditional)].darkRGBA == 0xC678_DDFF
    else {
        print("FAIL: imported colours did not reach every capture")
        return 1
    }
    CoreTheme.setBuiltin(named: "Textchum")
    try? FileManager.default.removeItem(at: importDir)
    print("theme import ok (VS Code JSON with comments, inherited captures, wearable)")

    // A file-icon pack: the tree asks the core for an image per file,
    // and the answers follow VS Code's order — whole name, then the
    // longest extension, then the language.
    let packDir = FileManager.default.temporaryDirectory
        .appendingPathComponent("textchum-pack-\(ProcessInfo.processInfo.processIdentifier)")
    try? FileManager.default.createDirectory(
        at: packDir.appendingPathComponent("icons"), withIntermediateDirectories: true)
    let square = """
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16">\
        <rect width="16" height="16" fill="#4488CC"/></svg>
        """
    for name in ["rust.svg", "docker.svg", "default.svg"] {
        try? square.write(
            to: packDir.appendingPathComponent("icons/\(name)"),
            atomically: true, encoding: .utf8)
    }
    try? """
        {
          "iconDefinitions": {
            "_rust": {"iconPath": "./icons/rust.svg"},
            "_docker": {"iconPath": "./icons/docker.svg"},
            "_default": {"iconPath": "./icons/default.svg"}
          },
          "fileExtensions": {"rs": "_rust"},
          "fileNames": {"dockerfile": "_docker"},
          "languageIds": {"rust": "_rust"},
          "file": "_default"
        }
        """.write(
        to: packDir.appendingPathComponent("icons.json"), atomically: true, encoding: .utf8)

    do {
        _ = try CoreIcons.load(at: packDir.appendingPathComponent("icons.json").path)
    } catch {
        print("FAIL: icon pack did not load: \(error)")
        return 1
    }
    guard CoreIcons.isActive,
        CoreIcons.icon(forFilename: "main.rs", language: nil, light: false) != nil,
        CoreIcons.icon(forFilename: "Dockerfile", language: nil, light: false) != nil,
        CoreIcons.icon(forFilename: "notes.xyz", language: nil, light: false) != nil
    else {
        print("FAIL: icon pack lookups")
        return 1
    }
    CoreIcons.clear()
    guard !CoreIcons.isActive,
        CoreIcons.icon(forFilename: "main.rs", language: nil, light: false) == nil
    else {
        print("FAIL: clearing the pack must return the tree to system icons")
        return 1
    }
    try? FileManager.default.removeItem(at: packDir)
    print("icon pack ok (loaded, looked up by name and extension, cleared)")

    // Go to Line reads what people actually type and paste, and
    // resolves it against the document, clamped to what is there.
    guard CoreDocument.parseGoTo("412")?.line == 412,
        CoreDocument.parseGoTo("412:8").map({ ($0.line, $0.column) }) ?? (0, 0) == (412, 8),
        CoreDocument.parseGoTo("src/main.rs:412:8").map({ ($0.line, $0.column) }) ?? (0, 0)
            == (412, 8),
        CoreDocument.parseGoTo(#"C:\src\main.rs:412:8"#).map({ ($0.line, $0.column) }) ?? (0, 0)
            == (412, 8),
        CoreDocument.parseGoTo("main.rs, line 412")?.line == 412,
        CoreDocument.parseGoTo("utf8.rs:12")?.line == 12,
        CoreDocument.parseGoTo("nowhere") == nil
    else {
        print("FAIL: go-to-line parsing")
        return 1
    }
    let lineDoc = CoreDocument()
    try? lineDoc.replace(utf16Range: NSRange(location: 0, length: 0), with: "one\ntwo\nthree")
    guard lineDoc.lineCount == 3,
        lineDoc.offset(ofLine: 2, column: 1) == 4,
        lineDoc.offset(ofLine: 2, column: 3) == 6,
        // Past the end of the line, and past the end of the document.
        lineDoc.offset(ofLine: 2, column: 99) == 7,
        lineDoc.offset(ofLine: 9999, column: 1) == 8
    else {
        print("FAIL: go-to-line offsets")
        return 1
    }
    print("go to line ok (compiler shapes, drive letters, clamping)")

    // Find References splits its answer: what calls this, then what
    // checks it. Telling them apart is a convention, so the rules are
    // held here — including the ones that must not fire.
    let tests = [
        "/p/tests/helpers.rs", "/p/spec/models/user_spec.rb",
        "/p/src/__tests__/Button.tsx", "/p/src/parser_test.go",
        "/p/src/test_parser.py", "/p/src/Button.test.ts",
        "/p/src/ParserTest.java", "/p/src/AppTests.swift",
    ]
    let notTests = [
        "/p/src/main.rs", "/p/src/latest.rs", "/p/src/protest.go",
        "/p/src/manifest.json", "/p/testing-library/index.js",
    ]
    guard tests.allSatisfy({ CoreReferences.isTest(path: $0) }),
        notTests.allSatisfy({ !CoreReferences.isTest(path: $0) })
    else {
        print("FAIL: test-path classification")
        return 1
    }
    print("reference split ok (conventions matched, near-misses left alone)")

    // Jump to Definition with the caret already on the definition has
    // nowhere to go, and asks who uses the symbol instead. The decision
    // is the core's; what is checked here is that it survives the
    // bridge with its answer intact.
    let definitionResult = """
        [{"uri": "file:///p/lib.rs", "range": {"start": {"line": 40, "character": 3}, \
        "end": {"line": 40, "character": 9}}}]
        """
    guard
        case .references = CoreDefinition.decide(
            result: definitionResult, path: "/p/lib.rs", line: 40, character: 5)
    else {
        print("FAIL: the caret on the definition did not ask for references")
        return 1
    }
    guard
        case .jump(let definitionTarget) = CoreDefinition.decide(
            result: definitionResult, path: "/p/main.rs", line: 2, character: 5),
        definitionTarget.path == "/p/lib.rs", definitionTarget.line == 40
    else {
        print("FAIL: a definition elsewhere was not a jump")
        return 1
    }
    guard case .nothing = CoreDefinition.decide(
        result: "null", path: "/p/lib.rs", line: 1, character: 1)
    else {
        print("FAIL: an empty definition answer was not nothing")
        return 1
    }
    // The declaration comes back among the references; the uses are
    // what is left after dropping it.
    let uses = CoreDefinition.elsewhere(
        result: """
            [{"uri": "file:///p/lib.rs", "range": {"start": {"line": 40, "character": 3}, \
            "end": {"line": 40, "character": 9}}}, \
            {"uri": "file:///p/main.rs", "range": {"start": {"line": 7, "character": 8}, \
            "end": {"line": 7, "character": 14}}}]
            """,
        path: "/p/lib.rs", line: 40, character: 5)
    guard uses.count == 1, uses[0].path == "/p/main.rs", uses[0].line == 7 else {
        print("FAIL: the caret's own line survived the reference filter")
        return 1
    }
    print("definition key ok (jump, on-the-definition, uses without the declaration)")

    // The context menu is the editor's own, not AppKit's. What matters
    // is what it holds — the commands that act on the place clicked —
    // and what it leaves out: Services, Speech, Substitutions and the
    // rest of the general text menu.
    let contextBench = Workbench(sidebar: nil)
    let contextEditor = DocumentController(document: CoreDocument())
    contextBench.add(contextEditor)
    contextBench.window?.makeKeyAndOrderFront(nil)
    let contextView = NSTextView(frame: NSRect(x: 0, y: 0, width: 400, height: 200))
    contextView.string = "def greet(name):\n    return name\n"
    let clicked = 6
    guard
        let contextMenu = contextEditor.textView(
            contextView, menu: NSMenu(), for: NSEvent(), at: clicked)
    else {
        print("FAIL: no context menu was built")
        return 1
    }
    let contextTitles = contextMenu.items.map { $0.title }
    let wanted = ["Cut", "Copy", "Paste", "Format Document", "File Properties…"]
    guard wanted.allSatisfy({ contextTitles.contains($0) }) else {
        print("FAIL: context menu is missing items: \(contextTitles)")
        return 1
    }
    let unwanted = ["Services", "Speech", "Substitutions", "Transformations", "Share"]
    guard !unwanted.contains(where: { contextTitles.contains($0) }) else {
        print("FAIL: context menu kept AppKit's items: \(contextTitles)")
        return 1
    }
    // Every command carries the character that was clicked, so it can
    // answer about that place rather than about the caret.
    let carried = contextMenu.items.compactMap {
        ($0.representedObject as? DocumentController.ContextCommand)?.index
    }
    guard !carried.isEmpty, carried.allSatisfy({ $0 == clicked }) else {
        print("FAIL: context commands do not carry the clicked character")
        return 1
    }
    contextBench.window?.close()
    print("context menu ok (editor commands, AppKit's extras left out, clicked position)")

    // A window is a tab bar over a row of columns. A column shows one
    // file and holds one or more views of it, stacked. TextKit 2 lets
    // several layout managers share one content storage, which is what
    // makes two views two views of one document; what does not come
    // free is the painting, since colour lives on the layout manager.
    let bench = Workbench(sidebar: nil)
    let first = DocumentController(document: CoreDocument())
    let second = DocumentController(document: CoreDocument())
    bench.add(first)
    bench.add(second)
    bench.window?.makeKeyAndOrderFront(nil)
    guard bench.documents.count == 2, bench.focusedDocument === second else {
        print("FAIL: the window did not take both tabs")
        return 1
    }
    // Choosing a tab changes what the column with the keyboard shows.
    bench.showInFocusedPane(ObjectIdentifier(first))
    guard bench.focusedDocument === first else {
        print("FAIL: choosing a tab did not reach the focused column")
        return 1
    }
    // Columns: three of them, each taking a tab of its own.
    bench.newColumn()
    bench.newColumn()
    guard bench.columns.count == 3, bench.focusedColumn == 2 else {
        print("FAIL: the window has \(bench.columns.count) columns")
        return 1
    }
    bench.showInFocusedPane(ObjectIdentifier(second))
    guard bench.document(inColumn: 2) === second, bench.document(inColumn: 0) === first
    else {
        print("FAIL: the columns do not hold a tab each")
        return 1
    }
    // Views: the same file twice in one column, and the two views are
    // of one document rather than two copies of it.
    bench.addViewToFocusedColumn()
    guard bench.columns[2].views.count == 2, second.paintTargetCount == 2 else {
        print("FAIL: the column did not take a second view")
        return 1
    }
    guard let firstStorage = second.primaryView?.textLayoutManager?.textContentManager,
        second.secondaryView?.textLayoutManager?.textContentManager === firstStorage
    else {
        print("FAIL: the two views are not on one document")
        return 1
    }
    // Switching a column's tab shows the new file the way that file is
    // shown — one view here, since `first` has never been split — and
    // the file that left keeps its own shape for when it comes back.
    bench.showInFocusedPane(ObjectIdentifier(first))
    guard bench.columns[2].views.count == 1, first.paintTargetCount == 3,
        second.paintTargetCount == 0, second.openDocument.layout.views == 2
    else {
        print("FAIL: the shape did not follow the file")
        return 1
    }
    bench.addViewToFocusedColumn()
    guard bench.columns[2].views.count == 2, first.paintTargetCount == 4 else {
        print("FAIL: the column did not take a second view")
        return 1
    }
    bench.closeFocusedView()
    guard bench.columns[2].views.count == 1, first.paintTargetCount == 3 else {
        print("FAIL: closing a view left it behind")
        return 1
    }
    // And the file that was split comes back split: two views, because
    // that is how this file is shown.
    bench.showInFocusedPane(ObjectIdentifier(second))
    guard bench.columns[2].views.count == 2, second.openDocument.layout.views == 2 else {
        print("FAIL: the file did not come back the way it was shown")
        return 1
    }
    bench.showInFocusedPane(ObjectIdentifier(first))
    // One file in every column at once.
    bench.showEverywhere(ObjectIdentifier(first))
    guard bench.columns.allSatisfy({ $0.document === first }) else {
        print("FAIL: the file did not reach every column")
        return 1
    }
    // The keyboard moves through the panes and comes back round.
    bench.focus(column: 0)
    bench.focusOtherPane()
    guard bench.focusedColumn == 1 else {
        print("FAIL: the focus did not move to the next pane")
        return 1
    }
    bench.focusOtherPane()
    bench.focusOtherPane()
    guard bench.focusedColumn == 0 else {
        print("FAIL: the focus did not come back round")
        return 1
    }
    bench.closeColumn()
    bench.closeColumn()
    guard bench.columns.count == 1, !bench.isSplit else {
        print("FAIL: closing the columns left \(bench.columns.count) behind")
        return 1
    }
    // Closing a tab closes the document; the column showing it moves to
    // what is left.
    bench.closeTab(ObjectIdentifier(second))
    guard bench.documents.count == 1, bench.focusedDocument === first else {
        print("FAIL: closing a tab did not leave the other one showing")
        return 1
    }
    bench.window?.close()
    print("window ok (columns, views of one file, tabs across them)")

    // Typing a delimiter over a selection wraps it, and the selection
    // stays on what was wrapped, so the next one nests: [({"hello"})].
    let wrapBench = Workbench(sidebar: nil)
    let wrapDocument = DocumentController(document: CoreDocument())
    wrapBench.add(wrapDocument)
    wrapBench.window?.makeKeyAndOrderFront(nil)
    guard let wrapView = wrapDocument.primaryView else {
        print("FAIL: no view to type into")
        return 1
    }
    wrapView.string = "hello"
    wrapDocument.noteTextReplaced()
    // What AppKit does when a character is typed: ask the delegate
    // about the selected range, and apply it only if allowed. Typing
    // goes through the many-ranges door, so the test does too.
    func type(_ characters: String) {
        let range = wrapView.selectedRange()
        let allowed = wrapDocument.textView(
            wrapView,
            shouldChangeTextInRanges: [NSValue(range: range)],
            replacementStrings: [characters])
        if allowed {
            wrapView.textStorage?.replaceCharacters(in: range, with: characters)
            wrapView.setSelectedRange(
                NSRange(location: range.location + (characters as NSString).length, length: 0))
        }
    }
    for delimiter in ["[", "(", "{", "\""] {
        type(delimiter)
    }
    guard wrapView.string == "[({\"hello\"})]" else {
        print("FAIL: wrapping gave \(wrapView.string)")
        return 1
    }
    let selected = (wrapView.string as NSString).substring(with: wrapView.selectedRange())
    guard selected == "hello" else {
        print("FAIL: the selection moved off what was wrapped: \(selected)")
        return 1
    }
    // Anything that is not a delimiter replaces, as before.
    type("x")
    guard wrapView.string == "[({\"x\"})]" else {
        print("FAIL: a letter should replace, not wrap: \(wrapView.string)")
        return 1
    }
    guard wrapDocument.coreDocument.text == "[({\"x\"})]" else {
        print("FAIL: the core did not follow: \(wrapDocument.coreDocument.text)")
        return 1
    }
    wrapBench.window?.close()
    print("wrapping ok (delimiters nest, letters do not wrap)")

    // The pinned context: scrolled into a Python method, the class line
    // and the def line hold the top of the view; the status bar knows
    // where the caret is and what the file is.
    let pinBench = Workbench(sidebar: nil)
    let pinDocument = DocumentController(document: CoreDocument())
    pinBench.add(pinDocument)
    pinBench.window?.makeKeyAndOrderFront(nil)
    guard let pinView = pinDocument.views.first else {
        print("FAIL: no view to pin over")
        return 1
    }
    let pinBody = (0..<120).map { "        line_\($0) = \($0)" }.joined(separator: "\n")
    pinView.textView.string = "class Greeter:\n    def greet(self):\n\(pinBody)\n"
    pinDocument.noteTextReplaced()
    _ = pinDocument.coreDocument.setLanguage("python")
    pinView.gutter.invalidateLineStarts()
    pinView.textView.layoutSubtreeIfNeeded()
    let target = (pinView.textView.string as NSString).range(of: "line_100")
    pinView.textView.scrollRangeToVisible(target)
    pinView.textView.layoutSubtreeIfNeeded()
    pinDocument.updateContextStrip(for: pinView)
    guard pinView.contextStrip.lines == [0, 1] else {
        print("FAIL: the pins say \(pinView.contextStrip.lines), not the class and the def")
        return 1
    }
    pinView.textView.setSelectedRange(NSRange(location: target.location, length: 0))
    let status = pinDocument.statusInfo
    guard status.line == 103, status.language == "python", status.tabWidth > 0 else {
        print("FAIL: the status bar would say Ln \(status.line), \(status.language ?? "nil")")
        return 1
    }
    pinBench.statusBar.show(status)
    pinBench.window?.close()
    print("context ok (pins stack the class and the def, the bar knows the caret)")

    // A link clicked in the Markdown preview goes to the browser and
    // leaves the pane where it was. Which links count as a place in the
    // page is the core's rule, tested there; this is about the preview
    // asking at all.
    let previewBench = Workbench(sidebar: nil)
    let previewDocument = DocumentController(document: CoreDocument())
    _ = previewDocument.coreDocument.setLanguage("markdown")
    previewBench.add(previewDocument)
    previewBench.window?.makeKeyAndOrderFront(nil)
    if previewDocument.previewWebViewForTest == nil {
        previewDocument.togglePreview(nil)
    }
    guard let previewWeb = previewDocument.previewWebViewForTest else {
        print("FAIL: no Markdown preview")
        return 1
    }
    guard previewWeb.navigationDelegate === previewDocument else {
        print("FAIL: the preview decides navigation on its own")
        return 1
    }
    guard !CorePreview.isPlaceInPage(here: "about:blank", target: "https://example.com/"),
        CorePreview.isPlaceInPage(here: "about:blank", target: "about:blank#notes")
    else {
        print("FAIL: the preview link rule is not the core's")
        return 1
    }
    previewBench.window?.close()
    print("preview links ok (the browser gets them, anchors stay)")

    // Folding hides the lines after the one that opens a block. TextKit
    // 2 lays out what the content storage offers, so what is checked
    // here is the layout itself: the folded lines have to take no room.
    let foldBench = Workbench(sidebar: nil)
    let foldDocument = CoreDocument()
    _ = foldDocument.setLanguage("rust")
    try? foldDocument.replace(
        utf16Range: NSRange(location: 0, length: 0),
        with: "fn main() {\n    let a = 1;\n    let b = 2;\n}\nafter\n")
    foldBench.add(DocumentController(document: foldDocument))
    foldBench.window?.makeKeyAndOrderFront(nil)
    guard let folder = foldBench.focusedDocument, let foldView = folder.primaryView else {
        print("FAIL: the folding window has no view")
        return 1
    }
    guard !folder.hasFolds else {
        print("FAIL: a fresh document is already folded")
        return 1
    }
    let heightOf: () -> CGFloat = {
        guard let layout = foldView.textLayoutManager else { return 0 }
        layout.ensureLayout(for: layout.documentRange)
        var height: CGFloat = 0
        layout.enumerateTextLayoutFragments(from: nil, options: [.ensuresLayout]) { fragment in
            height = max(height, fragment.layoutFragmentFrame.maxY)
            return true
        }
        return height
    }
    let unfolded = heightOf()
    folder.foldAll(nil)
    guard folder.hasFolds else {
        print("FAIL: nothing folded in a document with a block")
        return 1
    }
    let folded = heightOf()
    guard folded < unfolded - 20 else {
        print("FAIL: folding took no height away (\(unfolded) → \(folded))")
        return 1
    }
    folder.unfoldAll(nil)
    guard !folder.hasFolds, heightOf() >= unfolded - 0.5 else {
        print("FAIL: unfolding did not give the lines back")
        return 1
    }
    foldBench.window?.close()
    print("folding ok (folded lines take no height, given back on unfold)")

    // What a file remembers about itself, kept per project: the shape
    // it is shown in, what is folded, and the language it was told it
    // is. The record is the core's; what is checked here is the round
    // trip through the bridge and the sweep.
    let recordScratch = NSTemporaryDirectory() + "textchum-records-\(getpid())"
    let recordRoot = recordScratch + "/engine"
    try? FileManager.default.createDirectory(
        atPath: recordRoot, withIntermediateDirectories: true)
    let recorded = CoreProjectState.FileState(
        views: 2,
        dividers: [0.4],
        folds: [(start: 12, end: 48)],
        language: "rust",
        places: [.init(caret: 90, scroll: 12), .init()])
    guard
        CoreProjectState.setFileState(
            recorded, root: recordRoot, directory: recordScratch, inProject: false,
            path: recordRoot + "/src/parser.rs")
    else {
        print("FAIL: the record could not be written")
        return 1
    }
    guard
        let readBack = CoreProjectState.fileState(
            root: recordRoot, directory: recordScratch, inProject: false,
            path: recordRoot + "/src/parser.rs"), readBack == recorded
    else {
        print("FAIL: the file did not remember itself")
        return 1
    }
    guard CoreProjectState.records(directory: recordScratch).count == 1 else {
        print("FAIL: the record is not listed")
        return 1
    }
    // A record for a root that is gone is swept; one whose root is
    // there stays, whatever the keep window says.
    try? FileManager.default.removeItem(atPath: recordRoot)
    guard CoreProjectState.sweep(directory: recordScratch, keepDays: 90) == 1,
        CoreProjectState.records(directory: recordScratch).isEmpty
    else {
        print("FAIL: the sweep did not forget a record for a root that is gone")
        return 1
    }
    try? FileManager.default.removeItem(atPath: recordScratch)
    print("project records ok (written, read back, swept)")

    // The interface in another language. The catalogues are the core's,
    // so both shells say the same things in the same words; what is
    // checked here is the bridge and the fallback.
    CoreI18n.use("es")
    guard t("Close Tab") == "Cerrar pestaña" else {
        print("FAIL: the catalogue did not answer in Spanish: \(t("Close Tab"))")
        return 1
    }
    guard t("Save changes to {}?", "main.rs") == "¿Guardar los cambios en main.rs?" else {
        print("FAIL: the argument did not land: \(t("Save changes to {}?", "main.rs"))")
        return 1
    }
    guard t("A phrase nobody has translated") == "A phrase nobody has translated" else {
        print("FAIL: an untranslated phrase should read as itself")
        return 1
    }
    // Plurals belong to the catalogue: Spanish and French each have
    // their own rule, and one string gave "1 archivos".
    guard tn("{} file", "{} files", 1) == "1 archivo",
        tn("{} file", "{} files", 4) == "4 archivos"
    else {
        print("FAIL: the plural rule did not come from the catalogue")
        return 1
    }
    CoreI18n.use("en")
    guard t("Close Tab") == "Close Tab", CoreI18n.languages.count >= 3,
        tn("{} file", "{} files", 1) == "1 file", tn("{} file", "{} files", 2) == "2 files"
    else {
        print("FAIL: English is the text in the source")
        return 1
    }
    print("interface language ok (gettext catalogue, plurals, fallback)")

    // Every bundled keyboard profile names commands this platform has,
    // and every shortcut it gives parses. A profile naming a command
    // nobody answers to is a profile that quietly does nothing.
    let keyScratch = NSTemporaryDirectory() + "textchum-keys-\(getpid()).json"
    let keyConfig = CoreConfig(path: keyScratch)
    for (id, _) in keyConfig.keyProfileChoices where !id.isEmpty {
        keyConfig.keysProfile = id
        let bindings = keyConfig.effectiveKeys
        guard !bindings.isEmpty else {
            print("FAIL: profile \(id) binds nothing")
            return 1
        }
        for (action, spec) in bindings {
            guard AppDelegate.parseShortcut(spec) != nil else {
                print("FAIL: profile \(id) has unparseable \(spec) for \(action)")
                return 1
            }
        }
    }
    try? FileManager.default.removeItem(atPath: keyScratch)
    print("keyboard profiles ok (every binding parses)")

    // The store holds documents; controllers hold views of them. A
    // path opens once however many views ask for it, and a rename
    // follows the document rather than making a second one.
    let storeScratch = FileManager.default.temporaryDirectory
        .appendingPathComponent("textchum-smoke-store-\(getpid()).txt").path
    try? "one\n".write(toFile: storeScratch, atomically: true, encoding: .utf8)
    let firstDocument = DocumentStore.shared.open(CoreDocument(), path: storeScratch)
    let openCount = DocumentStore.shared.count
    let againDocument = DocumentStore.shared.open(CoreDocument(), path: storeScratch)
    guard againDocument.id == firstDocument.id, DocumentStore.shared.count == openCount else {
        print("FAIL: opening the same path twice made a second document")
        return 1
    }
    guard DocumentStore.shared.document(forPath: storeScratch)?.id == firstDocument.id else {
        print("FAIL: the path does not name its document")
        return 1
    }
    DocumentStore.shared.rename(firstDocument.id, from: storeScratch, to: "/tmp/renamed")
    guard DocumentStore.shared.document(forPath: storeScratch) == nil,
        DocumentStore.shared.document(forPath: "/tmp/renamed")?.id == firstDocument.id
    else {
        print("FAIL: the rename did not move the index")
        return 1
    }
    // Findings belong to the document, so every view of it agrees.
    let finding = try? JSONDecoder().decode(
        CoreDiagnostic.self,
        from: Data(
            """
            {"line": 0, "character": 0, "endLine": 0, "endCharacter": 1,
             "severity": 1, "message": "from the document"}
            """.utf8))
    guard let finding else {
        print("FAIL: could not make a finding to hand to the document")
        return 1
    }
    firstDocument.diagnostics = [finding]
    guard DocumentStore.shared.document(id: firstDocument.id)?.diagnostics.count == 1 else {
        print("FAIL: the findings did not stay with the document")
        return 1
    }
    DocumentStore.shared.close(firstDocument.id)
    guard DocumentStore.shared.document(id: firstDocument.id) == nil,
        DocumentStore.shared.document(forPath: "/tmp/renamed") == nil
    else {
        print("FAIL: closing left the document behind")
        return 1
    }

    // A closed file is kept whole: reopening it is taking the closing
    // back, so what was typed and never saved is still there.
    guard let takenBack = DocumentStore.shared.reclaim(path: "/tmp/renamed") else {
        print("FAIL: the closed document did not come back")
        return 1
    }
    guard takenBack.id == firstDocument.id else {
        print("FAIL: reopening made a second document")
        return 1
    }
    guard DocumentStore.shared.document(forPath: "/tmp/renamed") != nil else {
        print("FAIL: the document that came back is not open")
        return 1
    }
    guard DocumentStore.shared.reclaim(path: "/tmp/never-closed") == nil else {
        print("FAIL: a file that was never closed came out of the cache")
        return 1
    }
    DocumentStore.shared.close(takenBack.id)
    try? FileManager.default.removeItem(atPath: storeScratch)
    print("documents ok (one per path, renamed, closed and taken back)")

    // Selecting a word marks the other places it appears. The rules
    // live in the core; what is checked here is that they survive the
    // bridge, including the offsets, which are UTF-16 units.
    let occurrenceText = "item = item + items"
    let wordSelection = CoreOccurrences.marks(
        in: occurrenceText, selection: 0..<4, base: 0,
        caseSensitive: true, wholeWord: true)
    guard wordSelection.count == 2, wordSelection[1].start == 7, wordSelection[1].end == 11
    else {
        print("FAIL: the selected word's occurrences are \(wordSelection.count)")
        return 1
    }
    // A partial word was selected for some other reason.
    guard
        CoreOccurrences.marks(
            in: occurrenceText, selection: 0..<3, base: 0,
            caseSensitive: true, wholeWord: true
        ).isEmpty
    else {
        print("FAIL: a partial word marked something")
        return 1
    }
    // Inside a longer name counts when asked, and the base offset is
    // the document's.
    let inside = CoreOccurrences.marks(
        in: occurrenceText, selection: 0..<4, base: 100,
        caseSensitive: true, wholeWord: false)
    guard inside.count == 3, inside[0].start == 100, inside[2].start == 114 else {
        print("FAIL: partial matches or base offset wrong: \(inside.map(\.start))")
        return 1
    }
    print("occurrence marks ok (whole words, inside longer names, document offsets)")

    // A project's settings can be copied onto another root, which is
    // what a second service in the same layout needs. The rules are the
    // core's; the bridge is what is checked here.
    let projectScratch = FileManager.default.temporaryDirectory
        .appendingPathComponent("textchum-smoke-projects-\(getpid()).json").path
    let projectConfig = CoreConfig(path: projectScratch)
    projectConfig.setWorkspaceFlag(root: "/work/a", key: "ctags_fallback", value: true)
    projectConfig.setLSPEntry(root: "/work/a", language: "python", command: "pylsp")
    guard projectConfig.copyProject(from: "/work/a", to: "/work/b") else {
        print("FAIL: copying a project's settings did nothing")
        return 1
    }
    guard projectConfig.lspJSON.contains("/work/b"),
        projectConfig.workspaceJSON.contains("/work/b")
    else {
        print("FAIL: the copy did not land in both sections")
        return 1
    }
    guard projectConfig.configuredProjects == ["/work/a", "/work/b"] else {
        print("FAIL: configured projects are \(projectConfig.configuredProjects)")
        return 1
    }
    projectConfig.removeProject(root: "/work/a")
    guard projectConfig.configuredProjects == ["/work/b"],
        !projectConfig.lspJSON.contains("/work/a")
    else {
        print("FAIL: removing a project left something behind")
        return 1
    }
    print("project settings ok (copied whole, listed, removed whole)")

    // Keyboard profiles: the bundled ones, an override on top of one,
    // and what the whole thing resolves to. The rules are the core's;
    // the bridge is what is checked here.
    let keysScratch = FileManager.default.temporaryDirectory
        .appendingPathComponent("textchum-smoke-keys-\(getpid()).json").path
    let keysConfig = CoreConfig(path: keysScratch)
    guard keysConfig.effectiveKeys.isEmpty else {
        print("FAIL: a fresh configuration already has bindings")
        return 1
    }
    keysConfig.keysProfile = "vscode"
    guard keysConfig.effectiveKeys["openQuickly"] == "cmd+p",
        keysConfig.effectiveKeys["renameSymbol"] == "f2"
    else {
        print("FAIL: the bundled profile did not resolve")
        return 1
    }
    keysConfig.setKeyBinding(action: "openQuickly", spec: "cmd+t")
    guard keysConfig.effectiveKeys["openQuickly"] == "cmd+t",
        keysConfig.effectiveKeys["renameSymbol"] == "f2"
    else {
        print("FAIL: an override did not win over the profile")
        return 1
    }
    keysConfig.clearKeyBindings()
    guard keysConfig.effectiveKeys["openQuickly"] == "cmd+p" else {
        print("FAIL: clearing the overrides did not restore the profile")
        return 1
    }
    guard keysConfig.keyProfileChoices.contains(where: { $0.id == "intellij" }) else {
        print("FAIL: the bundled profiles are not offered")
        return 1
    }
    // Function keys survive the round trip through a shortcut spec.
    guard let (functionKey, functionModifiers) = AppDelegate.parseShortcut("shift+f12"),
        AppDelegate.shortcutSpec(key: functionKey, modifiers: functionModifiers)
            == "shift+f12"
    else {
        print("FAIL: a function-key shortcut did not round-trip")
        return 1
    }
    print("keyboard profiles ok (bundled, overridden, reset, function keys)")

    // Icon packs: importing copies the pack into Textchum's folder, and
    // the list says which ones are ours.
    let packScratch = FileManager.default.temporaryDirectory
        .appendingPathComponent("textchum-smoke-icons-\(getpid())")
    let packSource = packScratch.appendingPathComponent("source/icons")
    try? FileManager.default.createDirectory(
        at: packSource, withIntermediateDirectories: true)
    try? "<svg/>".write(
        to: packSource.appendingPathComponent("rust.svg"), atomically: true, encoding: .utf8)
    let packTheme = packScratch.appendingPathComponent("source/icons.json")
    try? """
        {"iconDefinitions": {"_rust": {"iconPath": "./icons/rust.svg"}},
         "fileExtensions": {"rs": "_rust"}}
        """.write(to: packTheme, atomically: true, encoding: .utf8)
    let packLibrary = packScratch.appendingPathComponent("library").path
    let packConfig = CoreConfig(
        path: packScratch.appendingPathComponent("config.json").path)
    let importedPack = packConfig.importIconPack(from: packTheme.path, into: packLibrary)
    guard let importedPath = importedPack.path else {
        print("FAIL: importing an icon pack said \(importedPack.error ?? "nothing")")
        return 1
    }
    guard FileManager.default.fileExists(atPath: importedPath) else {
        print("FAIL: the imported pack is not where it says")
        return 1
    }
    let packs = packConfig.iconPacks(in: packLibrary)
    guard packs.count == 1, packs[0].imported else {
        print("FAIL: the imported pack is not on the list as ours")
        return 1
    }
    // A second import of the same pack is refused rather than merged.
    guard packConfig.importIconPack(from: packTheme.path, into: packLibrary).path == nil else {
        print("FAIL: importing the same pack twice was allowed")
        return 1
    }
    packConfig.removeIconPack(path: importedPath, from: packLibrary)
    guard packConfig.iconPacks(in: packLibrary).isEmpty else {
        print("FAIL: the deleted pack is still listed")
        return 1
    }
    try? FileManager.default.removeItem(at: packScratch)
    print("icon packs ok (imported, listed as ours, refused twice, deleted)")

    // --data-dir moves the whole profile, so a run can be given one
    // built for the occasion. What matters is that every path follows
    // it: a profile with the configuration in it and the session
    // somewhere else is not a profile.
    let profileArguments = ["Textchum", "--data-dir", "/tmp/textchum-profile", "a.py"]
    guard let namedProfile = AppPaths.dataDirectory(from: profileArguments),
        namedProfile.path == "/tmp/textchum-profile"
    else {
        print("FAIL: --data-dir did not name the profile")
        return 1
    }
    guard AppPaths.dataDirectory(from: ["Textchum", "a.py"]) == nil,
        AppPaths.dataDirectory(from: ["Textchum", "--data-dir"]) == nil
    else {
        print("FAIL: a profile was named where none was")
        return 1
    }
    print("profile paths ok (--data-dir names one, its absence names none)")

    // Text transformations. The rules are the core's; what is checked
    // here is that they survive the bridge, including the one that is
    // not obvious — an operation over lines keeps the line endings the
    // text came with.
    guard CoreTransform.apply("upper", to: "hello") == "HELLO",
        CoreTransform.apply("title", to: "don't be well-known") == "Don't Be Well-Known",
        CoreTransform.apply("sort", to: "pear\napple\n") == "apple\npear\n",
        CoreTransform.apply("dedupe", to: "a\nb\na") == "a\nb",
        CoreTransform.apply("join", to: "one\n    two") == "one two",
        CoreTransform.apply("trim", to: "one   \ntwo") == "one\ntwo",
        CoreTransform.apply("crlf", to: "one\ntwo") == "one\r\ntwo",
        CoreTransform.apply("sort", to: "pear\r\napple\r\n") == "apple\r\npear\r\n"
    else {
        print("FAIL: a transformation did not survive the bridge")
        return 1
    }
    guard CoreTransform.apply("nonexistent", to: "hello") == nil else {
        print("FAIL: an unknown transformation did something")
        return 1
    }
    guard CoreTransform.isLineWise("sort"), !CoreTransform.isLineWise("upper") else {
        print("FAIL: line-wise transformations are not saying so")
        return 1
    }
    print("transformations ok (case, lines, endings kept, unknown refused)")

    // Code actions. The protocol's loosest answer — an array mixing two
    // shapes, one of which may arrive without the edit it is about — so
    // what is checked is that each shape says what choosing it means.
    let actionsResult = """
        [{"title": "Import `HashMap`", "kind": "quickfix", "isPreferred": true, \
        "edit": {"changes": {}}}, \
        {"title": "Extract into function", "kind": "refactor.extract"}, \
        {"title": "Organize imports", "command": "rust-analyzer.organizeImports", \
        "arguments": ["file:///p/a.rs"]}]
        """
    let offered = CoreCodeActions.actions(inResultJSON: actionsResult)
    guard offered.count == 3, offered[0].preferred, offered[0].kind == "quickfix" else {
        print("FAIL: the code actions did not survive the bridge")
        return 1
    }
    guard case .edit = CoreCodeActions.outcome(inResultJSON: actionsResult, at: 0) else {
        print("FAIL: an action carrying an edit did not say so")
        return 1
    }
    guard case .resolve = CoreCodeActions.outcome(inResultJSON: actionsResult, at: 1) else {
        print("FAIL: an action with no edit did not ask to be resolved")
        return 1
    }
    guard case .command(let commandName, _) = CoreCodeActions.outcome(
        inResultJSON: actionsResult, at: 2),
        commandName == "rust-analyzer.organizeImports"
    else {
        print("FAIL: an action carrying a command did not say so")
        return 1
    }
    guard case .nothing = CoreCodeActions.outcome(inResultJSON: actionsResult, at: 99) else {
        print("FAIL: a stale choice did something")
        return 1
    }
    print("code actions ok (edit, resolve, command, stale choice)")

    // Repainting the syntax colours on every scroll turn was the
    // stutter; the margin around the viewport is there so that most
    // turns need no repaint at all. The wiring needs a window server,
    // the decision does not.
    let painted = NSRange(location: 1_000, length: 20_000)
    guard
        // Nothing painted yet.
        DocumentController.shouldRepaint(
            viewport: NSRange(location: 5_000, length: 4_000), painted: nil,
            documentLength: 50_000, paintedLength: nil),
        // Inside what is painted: the whole point of the margin.
        !DocumentController.shouldRepaint(
            viewport: NSRange(location: 5_000, length: 4_000), painted: painted,
            documentLength: 50_000, paintedLength: 50_000),
        // Off the top of it, and off the bottom of it.
        DocumentController.shouldRepaint(
            viewport: NSRange(location: 900, length: 4_000), painted: painted,
            documentLength: 50_000, paintedLength: 50_000),
        DocumentController.shouldRepaint(
            viewport: NSRange(location: 19_000, length: 4_000), painted: painted,
            documentLength: 50_000, paintedLength: 50_000),
        // A document that changed length: the offsets moved under it.
        DocumentController.shouldRepaint(
            viewport: NSRange(location: 5_000, length: 4_000), painted: painted,
            documentLength: 60_000, paintedLength: 50_000)
    else {
        print("FAIL: scroll repaint decision")
        return 1
    }
    print("scroll repaint ok (skips inside the margin, repaints past it)")

    // The gutter's git marks: a committed file, edited three ways.
    let repo = FileManager.default.temporaryDirectory
        .appendingPathComponent("textchum-gutter-\(ProcessInfo.processInfo.processIdentifier)")
    try? FileManager.default.removeItem(at: repo)
    try? FileManager.default.createDirectory(at: repo, withIntermediateDirectories: true)
    func git(_ arguments: [String]) {
        let task = Process()
        task.executableURL = URL(fileURLWithPath: "/usr/bin/git")
        task.arguments = ["-C", repo.path] + arguments
        task.standardOutput = FileHandle.nullDevice
        task.standardError = FileHandle.nullDevice
        try? task.run()
        task.waitUntilExit()
    }
    let tracked = repo.appendingPathComponent("thing.txt")
    try? "one\ntwo\nthree\nfour\n".write(to: tracked, atomically: true, encoding: .utf8)
    git(["init", "-q"])
    git(["config", "user.email", "t@e.invalid"])
    git(["config", "user.name", "T"])
    git(["add", "thing.txt"])
    git(["commit", "-qm", "first"])

    guard CoreChanges.marks(forPath: tracked.path, text: "one\ntwo\nthree\nfour\n").isEmpty
    else {
        print("FAIL: an unchanged file should carry no marks")
        return 1
    }
    // "two" edited, "three" deleted, "five" added at the end. The
    // removed mark lands on line 2 — the line "three" sat above — since
    // a deleted line occupies no place of its own.
    let edited = CoreChanges.marks(
        forPath: tracked.path, text: "one\nTWO\nfour\nfive\n")
    let described = edited.map { "\($0.line):\($0.kind.rawValue)" }
    guard described == ["1:modified", "2:removed", "3:added"] else {
        print("FAIL: gutter marks: \(described)")
        return 1
    }
    // A file with no committed version is not an error, and not marked.
    let untracked = repo.appendingPathComponent("never-committed.txt")
    try? "hello\n".write(to: untracked, atomically: true, encoding: .utf8)
    guard CoreChanges.marks(forPath: untracked.path, text: "hello\nworld\n").isEmpty else {
        print("FAIL: an untracked file should carry no marks")
        return 1
    }
    try? FileManager.default.removeItem(at: repo)
    print("git gutter ok (marks what changed, silent without a baseline)")

    // Blame: what git knows about one line, asked with the buffer's
    // text so an unsaved edit cannot shift the answer.
    let blameRepo = FileManager.default.temporaryDirectory
        .appendingPathComponent("textchum-blame-\(ProcessInfo.processInfo.processIdentifier)")
    try? FileManager.default.removeItem(at: blameRepo)
    try? FileManager.default.createDirectory(at: blameRepo, withIntermediateDirectories: true)
    func blameGit(_ arguments: [String]) {
        let task = Process()
        task.executableURL = URL(fileURLWithPath: "/usr/bin/git")
        task.arguments = ["-C", blameRepo.path] + arguments
        task.standardOutput = FileHandle.nullDevice
        task.standardError = FileHandle.nullDevice
        try? task.run()
        task.waitUntilExit()
    }
    let blamed = blameRepo.appendingPathComponent("thing.txt")
    try? "first\nsecond\n".write(to: blamed, atomically: true, encoding: .utf8)
    blameGit(["init", "-q"])
    blameGit(["config", "user.email", "ada@example.invalid"])
    blameGit(["config", "user.name", "Ada Lovelace"])
    blameGit(["add", "thing.txt"])
    blameGit(["commit", "-qm", "Add two lines\n\nAnd a reason for them."])

    do {
        // Two lines typed above "second" and not saved: on disk line 4
        // does not exist, in the buffer it is the committed line.
        let buffer = "first\ntyped one\ntyped two\nsecond\n"
        let committed = try CoreBlame.line(4, ofPath: blamed.path, text: buffer)
        guard !committed.uncommitted, committed.author == "Ada Lovelace",
            committed.summary == "Add two lines", committed.body == "And a reason for them.",
            committed.commit.count == 40, !committed.abbreviated.isEmpty,
            committed.renamedFrom.isEmpty
        else {
            print("FAIL: blame of a committed line: \(committed)")
            return 1
        }
        let typed = try CoreBlame.line(2, ofPath: blamed.path, text: buffer)
        guard typed.uncommitted, typed.commit.isEmpty else {
            print("FAIL: a typed line should say it is not committed")
            return 1
        }
        // The caret past the end is answered about the last line there
        // is, and says which line that was.
        let past = try CoreBlame.line(99, ofPath: blamed.path, text: buffer)
        guard past.line == 4 else {
            print("FAIL: blame past the end should answer about the last line, got \(past.line)")
            return 1
        }
    } catch {
        print("FAIL: blame threw: \(error)")
        return 1
    }
    // A file outside a repository says so rather than answering.
    let loose = FileManager.default.temporaryDirectory
        .appendingPathComponent("textchum-loose-\(ProcessInfo.processInfo.processIdentifier).txt")
    try? "hello\n".write(to: loose, atomically: true, encoding: .utf8)
    var refused = false
    do { _ = try CoreBlame.line(1, ofPath: loose.path, text: "hello\n") } catch { refused = true }
    guard refused else {
        print("FAIL: a file outside a repository should be refused, not answered")
        return 1
    }
    try? FileManager.default.removeItem(at: loose)
    try? FileManager.default.removeItem(at: blameRepo)
    print("blame ok (buffer-aware, uncommitted lines, past the end, outside a repo)")

    // The theme's bold and italic go into the text storage, which
    // invalidates layout over whatever range they are written to. The
    // pass must therefore write only what differs — a pass over
    // unchanged text writing sixteen thousand units is what moved the
    // view out from under the caret while typing.
    do {
        let plain = NSFont.monospacedSystemFont(ofSize: 13, weight: .regular)
        let italic = NSFontManager.shared.convert(plain, toHaveTrait: .italicFontMask)
        let storage = NSTextStorage(
            string: String(repeating: "abcdefghij", count: 400),
            attributes: [.font: plain])
        let whole = NSRange(location: 0, length: storage.length)
        let wanted = [NSRange(location: 100, length: 50): italic]

        let first = DocumentController.applyTraitFonts(
            wanted, over: whole, in: storage, plain: plain)
        guard first == 50 else {
            print("FAIL: the first trait pass wrote \(first) units, wanted 50")
            return 1
        }
        // Nothing changed, so nothing should be written — this is the
        // one that matters.
        let second = DocumentController.applyTraitFonts(
            wanted, over: whole, in: storage, plain: plain)
        guard second == 0 else {
            print("FAIL: an unchanged trait pass wrote \(second) units, wanted 0")
            return 1
        }
        // A span that lost its italic is put back to the plain font,
        // and only over the part that lost it.
        let narrowed = [NSRange(location: 100, length: 40): italic]
        let third = DocumentController.applyTraitFonts(
            narrowed, over: whole, in: storage, plain: plain)
        guard third == 10 else {
            print("FAIL: narrowing a trait wrote \(third) units, wanted 10")
            return 1
        }
        // 100..<140 is italic now, so 145 is back to plain and 139 is
        // the last italic character.
        guard storage.attribute(.font, at: 145, effectiveRange: nil) as? NSFont == plain,
            storage.attribute(.font, at: 139, effectiveRange: nil) as? NSFont == italic
        else {
            print("FAIL: trait fonts did not end up where they belong")
            return 1
        }
    }
    print("trait fonts ok (writes only what differs, so typing does not relayout)")

    // Backspace in a line's leading spaces takes a whole indent, and
    // one character anywhere else. It is the position that decides,
    // which is what keeps it from surprising anyone.
    guard CoreDocument.backspaceWidth(before: "    ", tabWidth: 4) == 4,
        CoreDocument.backspaceWidth(before: "      ", tabWidth: 4) == 2,
        CoreDocument.backspaceWidth(before: "    let x", tabWidth: 4) == 1,
        CoreDocument.backspaceWidth(before: "let x = ", tabWidth: 4) == 1,
        // A tab-indented line already has one character per level.
        CoreDocument.backspaceWidth(before: "\t\t", tabWidth: 4) == 1,
        CoreDocument.backspaceWidth(before: "", tabWidth: 4) == 0
    else {
        print("FAIL: backspace indent widths")
        return 1
    }
    // Tab in the indentation lines up with the block above, and goes a
    // level deeper once it is already level.
    guard
        CoreDocument.alignedIndent(
            previous: "        deep()", currentIndent: "", tabWidth: 4, useTabs: false)
            == "        ",
        CoreDocument.alignedIndent(
            previous: "    thing()", currentIndent: "    ", tabWidth: 4, useTabs: false)
            == "        ",
        CoreDocument.alignedIndent(
            previous: "    thing()", currentIndent: "  ", tabWidth: 4, useTabs: false)
            == "    ",
        CoreDocument.alignedIndent(
            previous: nil, currentIndent: "", tabWidth: 4, useTabs: false) == "    ",
        CoreDocument.alignedIndent(
            previous: "\t\tdeep()", currentIndent: "", tabWidth: 4, useTabs: true) == "\t\t"
    else {
        print("FAIL: aligned indentation")
        return 1
    }
    print("indentation ok (backspace by level, tab aligns with the block above)")

    // Every mouse move over the editor asks which character is under
    // the pointer. The first version of that added a line fragment's
    // own index to a document offset, and NSTextLineFragment answers
    // NSNotFound — Int.max — for a point it does not cover, so the
    // addition overflowed and trapped. Sweep the points that do it.
    do {
        let sweepView = NSTextView(frame: NSRect(x: 0, y: 0, width: 300, height: 120))
        sweepView.string = "one\ntwo\nthree\n"
        sweepView.textLayoutManager?.ensureLayout(
            for: sweepView.textLayoutManager!.documentRange)
        let length = (sweepView.string as NSString).length
        for x in stride(from: -200.0, through: 900.0, by: 37.0) {
            for y in stride(from: -200.0, through: 900.0, by: 37.0) {
                guard let index = DocumentController.characterIndex(
                    at: NSPoint(x: x, y: y), in: sweepView)
                else { continue }
                guard index >= 0, index < length else {
                    print("FAIL: character index \(index) at (\(x), \(y)), length \(length)")
                    return 1
                }
            }
        }
    }
    print("pointer character index ok (in range everywhere, including off the text)")

    print("smoke test passed")
    return 0
}
