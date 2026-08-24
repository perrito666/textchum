import Foundation
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
        guard let undone = document.undo(), undone.replacement.isEmpty else {
            print("FAIL: undo returned no edit")
            return 1
        }
        guard document.text == "hello", document.redo() != nil, document.text == "hello world"
        else {
            print("FAIL: undo/redo walk: got \(document.text.debugDescription)")
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
        print("document lifecycle ok (edit/undo/save/reopen)")
        try? FileManager.default.removeItem(at: smokeDir)
    } catch {
        print("FAIL: document lifecycle: \(error)")
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
        try config.save()
        let reloaded = CoreConfig(path: configPath)
        guard reloaded.fontFamily == "Menlo", reloaded.tabWidth == 8 else {
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
        try? FileManager.default.removeItem(at: configDir)
    } catch {
        print("FAIL: configuration: \(error)")
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

    print("smoke test passed")
    return 0
}
