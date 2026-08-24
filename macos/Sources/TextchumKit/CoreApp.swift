import CTextchum
import Foundation

/// The root handle for a core instance and the receiving end of its events.
///
/// The core delivers events (today just pongs; later diagnostics, highlight
/// invalidations, and the rest) on a single dispatch thread it owns. This
/// class hides the C callback plumbing and hands the app typed events on the
/// main queue, which is the threading contract the rest of the shell relies
/// on.
public final class CoreApp {
    /// A typed event from the core.
    public enum Event: Equatable, Sendable {
        /// Reply to ``ping(sequence:)``.
        case pong(sequence: UInt64)
    }

    /// Retained context handed to the C callback as its `userdata`.
    private final class EventSink: @unchecked Sendable {
        let deliver: @Sendable (Event) -> Void

        init(deliver: @escaping @Sendable (Event) -> Void) {
            self.deliver = deliver
        }
    }

    private let handle: OpaquePointer
    private let sink: EventSink

    /// Creates a core instance.
    ///
    /// - Parameter onEvent: called on the **main actor** for every core
    ///   event. The closure escapes for the lifetime of this object.
    public init(onEvent: @escaping @MainActor @Sendable (Event) -> Void) {
        let sink = EventSink { event in
            DispatchQueue.main.async {
                // Safe by construction: the main queue is the main actor's
                // executor. Dispatch (rather than Task) keeps delivery in
                // strict event order.
                MainActor.assumeIsolated { onEvent(event) }
            }
        }
        self.sink = sink

        // The C callback: no captures allowed, so context arrives through
        // `userdata`. Unretained is safe because `self.sink` outlives the
        // handle: tc_app_free (in deinit) joins the dispatch thread before
        // properties are released.
        let callback: TcEventCallback = { eventPointer, userdata in
            guard let eventPointer, let userdata else { return }
            let sink = Unmanaged<EventSink>.fromOpaque(userdata).takeUnretainedValue()
            let event = eventPointer.pointee
            switch event.kind {
            case UInt32(TC_EVENT_PONG):
                sink.deliver(.pong(sequence: event.seq))
            default:
                // Unknown kinds are forward-compatibility, not errors: a
                // newer core may emit events this shell does not know yet.
                break
            }
        }

        self.handle = tc_app_new(callback, Unmanaged.passUnretained(sink).toOpaque())!
    }

    deinit {
        tc_app_free(handle)
    }

    /// Asks the core to send back ``Event/pong(sequence:)`` with the same
    /// number. Exists to verify the async event path end to end.
    public func ping(sequence: UInt64) {
        tc_app_ping(handle, sequence)
    }
}
