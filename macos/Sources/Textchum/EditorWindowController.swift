import AppKit
import TextchumKit

/// One editor window: a text view kept in lockstep with a core buffer.
///
/// The synchronization protocol — the most delicate piece of the app — is:
///
/// 1. The core buffer is the source of truth; the text view's storage is a
///    display cache.
/// 2. Every change AppKit is about to make (typing, paste, drop, undo — they
///    all funnel through `shouldChangeTextIn`) is applied to the core buffer
///    *first*, as the same UTF-16 range edit.
/// 3. If the core rejects the edit, the view change is refused too, so the
///    two sides can only move together.
/// 4. After each change the window title shows the core's view of the
///    document, and debug builds assert both sides are byte-identical.
final class EditorWindowController: NSWindowController {
    private let buffer = CoreBuffer()
    private var textView: NSTextView?
    private var pongSubtitle = ""

    convenience init() {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 720, height: 480),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.center()
        window.tabbingMode = .automatic
        self.init(window: window)

        let scrollView = NSTextView.scrollableTextView()
        let textView = scrollView.documentView as! NSTextView
        textView.font = .monospacedSystemFont(ofSize: 13, weight: .regular)
        textView.isRichText = false
        textView.allowsUndo = true
        textView.isAutomaticQuoteSubstitutionEnabled = false
        textView.isAutomaticDashSubstitutionEnabled = false
        textView.isAutomaticTextReplacementEnabled = false
        textView.delegate = self
        self.textView = textView

        seedWelcomeText(into: textView)
        window.contentView = scrollView
        updateTitle()
    }

    /// Called (on the main queue) when the core answers the launch ping.
    func coreDidRespond(toPing sequence: UInt64) {
        pongSubtitle = "core \(Core.version) · pong \(sequence)"
        updateTitle()
    }

    private func seedWelcomeText(into textView: NSTextView) {
        // Seed through the core so even the initial content exercises the
        // edit path rather than being assigned behind the core's back.
        try? buffer.insert("Welcome to Textchum.\n", atByteOffset: 0)
        textView.string = buffer.text
    }

    private func updateTitle() {
        window?.title = "Textchum — \(buffer.lengthInBytes) bytes"
        window?.subtitle = pongSubtitle
    }
}

extension EditorWindowController: NSTextViewDelegate {
    func textView(
        _ textView: NSTextView,
        shouldChangeTextIn affectedCharRange: NSRange,
        replacementString: String?
    ) -> Bool {
        // A nil replacement is an attribute-only change; no text moves.
        guard let replacementString else { return true }
        do {
            try buffer.replace(utf16Range: affectedCharRange, with: replacementString)
            return true
        } catch {
            // Core refused: refuse the view edit as well so neither side
            // moves. Rejections here indicate a sync bug worth surfacing.
            NSSound.beep()
            NSLog("edit rejected by core: \(error)")
            return false
        }
    }

    func textDidChange(_ notification: Notification) {
        updateTitle()
        #if DEBUG
            // The invariant behind the whole design. O(document) per edit is
            // acceptable while documents are small; revisit with checksums
            // before large-file work.
            if let textView, buffer.text != textView.string {
                assertionFailure("core buffer and text view diverged")
            }
        #endif
    }
}
