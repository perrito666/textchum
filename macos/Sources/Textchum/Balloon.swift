import AppKit

/// A balloon beside the text: hover documentation, a finding's
/// message, the signature of the call being typed.
///
/// It is a borderless child window of the document's window, and it
/// can never become key. A popover's window could, and did: the
/// moment the pointer rested on a finding the balloon took the
/// keyboard, the caret vanished from the text, and the error being
/// explained could not be fixed. A balloon is read, scrolled and
/// copied from, never typed into, so the keyboard stays where it was.
@MainActor
final class Balloon {
    private final class Window: NSWindow {
        /// Only when asked for by name (1.3): a balloon that shows
        /// itself never takes the keyboard (1.1).
        var takesKeyboard = false
        var onEscape: (() -> Void)?
        override var canBecomeKey: Bool { takesKeyboard }
        override var canBecomeMain: Bool { false }

        // Escape inside goes back to the text: heard here, before the
        // text inside answers it with a completion list of its own.
        override func sendEvent(_ event: NSEvent) {
            if event.type == .keyDown, event.keyCode == 53, let onEscape {
                onEscape()
                return
            }
            super.sendEvent(event)
        }
    }

    private let window: Window
    private weak var parent: NSWindow?
    private(set) var isShown = false

    /// Where a balloon of `size` goes for the text it is about at
    /// `anchor` (screen coordinates), on a screen whose usable part is
    /// `visible`: under the text when it fits there, over it when it
    /// does not, never off the screen sideways.
    static func frame(size: NSSize, anchor: NSRect, visible: NSRect) -> NSRect {
        let gap: CGFloat = 6
        let below = anchor.minY - gap - size.height
        let y = below >= visible.minY ? below : anchor.maxY + gap
        let x = max(visible.minX, min(anchor.minX - 10, visible.maxX - size.width))
        return NSRect(x: x, y: y, width: size.width, height: size.height)
    }

    /// Shows `content` — sized already — beside `anchor`, as a child of
    /// `parent`.
    init(content: NSView, anchor: NSRect, parent: NSWindow) {
        let backdrop = NSVisualEffectView(frame: NSRect(origin: .zero, size: content.frame.size))
        backdrop.material = .popover
        backdrop.state = .active
        backdrop.wantsLayer = true
        backdrop.layer?.cornerRadius = 8
        backdrop.layer?.masksToBounds = true
        backdrop.layer?.borderWidth = 1
        backdrop.layer?.borderColor = NSColor.separatorColor.cgColor
        content.frame.origin = .zero
        content.autoresizingMask = [.width, .height]
        backdrop.addSubview(content)

        let visible = (parent.screen ?? NSScreen.main)?.visibleFrame ?? .infinite
        let frame = Self.frame(size: backdrop.frame.size, anchor: anchor, visible: visible)
        window = Window(contentRect: frame, styleMask: [.borderless], backing: .buffered, defer: false)
        window.isOpaque = false
        window.backgroundColor = .clear
        window.hasShadow = true
        window.isReleasedWhenClosed = false
        // No fading in and out: a balloon that animates while the
        // pointer moves along a line is hard to read.
        window.animationBehavior = .none
        window.contentView = backdrop
        self.parent = parent
        parent.addChildWindow(window, ordered: .above)
        window.orderFront(nil)
        isShown = true
    }

    /// Whether the keyboard has been moved inside, to scroll, select and
    /// copy with it.
    var hasKeyboard: Bool { window.isKeyWindow }

    /// Moves the keyboard inside: the user asked again for what the
    /// balloon shows (1.3). The text in it takes the keys, so the arrows
    /// scroll and ⌘C copies; Escape calls `onEscape`, which brings the
    /// keyboard back out.
    func enter(onEscape: @escaping () -> Void) {
        window.takesKeyboard = true
        window.onEscape = onEscape
        window.makeKey()
        if let text = Self.firstTextView(in: window.contentView) {
            window.makeFirstResponder(text)
        }
    }

    private static func firstTextView(in view: NSView?) -> NSTextView? {
        guard let view else { return nil }
        if let text = view as? NSTextView { return text }
        for sub in view.subviews {
            if let found = firstTextView(in: sub) { return found }
        }
        return nil
    }

    func close() {
        guard isShown else { return }
        isShown = false
        parent?.removeChildWindow(window)
        window.orderOut(nil)
    }

    /// The window itself, for the smoke test to look at.
    var debugWindow: NSWindow { window }
}
