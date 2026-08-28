// AppKit for the Settings window check below: this test builds a
// real window, not a stand-in.
import AppKit
import Foundation
import SwiftUI
import TextchumKit

/// Headless verification that the shell and the core actually talk to each
/// other: text edits round-trip through the core (including a UTF-16 range
/// edit over a surrogate pair, the trickiest unit conversion we do), and an
/// asynchronous event makes it from a core thread back to the main queue.
///
/// Returns a process exit code: 0 on success.
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
        EditorWindowController.autoIndentedNewline(
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
    let prose = EditorWindowController.ranges(of: whole, excluding: skipped)
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
    guard EditorWindowController.shouldAdoptSidebarWidth(320, current: 188, collapsed: false)
    else {
        print("FAIL: a different width from another window should be adopted")
        return 1
    }
    guard !EditorWindowController.shouldAdoptSidebarWidth(188, current: 188, collapsed: false)
    else {
        print("FAIL: adopting the width already held is how a feedback loop starts")
        return 1
    }
    guard !EditorWindowController.shouldAdoptSidebarWidth(188.2, current: 188, collapsed: false)
    else {
        print("FAIL: a sub-point difference is the same width coming back")
        return 1
    }
    guard !EditorWindowController.shouldAdoptSidebarWidth(320, current: 0, collapsed: true)
    else {
        print("FAIL: a collapsed navigator must stay collapsed")
        return 1
    }
    print("sidebar width sync ok (adopts changes, ignores echoes and collapsed)")

    // Which keys a running snippet takes. The routing needs a window
    // server; the table it consults is a pure decision and is checked
    // here — a snippet that swallowed Return, or missed Shift-Tab,
    // would be a mode with the wrong way out.
    guard EditorWindowController.snippetKey(for: #selector(NSResponder.insertTab(_:)))
            == .nextStop,
        EditorWindowController.snippetKey(for: #selector(NSResponder.insertBacktab(_:)))
            == .previousStop,
        EditorWindowController.snippetKey(for: #selector(NSResponder.cancelOperation(_:)))
            == .cancel,
        EditorWindowController.snippetKey(for: #selector(NSResponder.insertNewline(_:))) == nil,
        EditorWindowController.snippetKey(for: #selector(NSResponder.moveDown(_:))) == nil
    else {
        print("FAIL: snippet key routing")
        return 1
    }
    print("snippet keys ok (tab, shift-tab, escape; everything else falls through)")

    print("smoke test passed")
    return 0
}

