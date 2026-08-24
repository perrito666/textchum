import CTextchum
import Foundation

/// One language-server finding, positioned the LSP way: zero-based line,
/// UTF-16 column.
public struct CoreDiagnostic: Codable, Equatable, Sendable {
    public let line: Int
    public let character: Int
    public let endLine: Int
    public let endCharacter: Int
    /// 1 = error, 2 = warning, 3 = information, 4 = hint.
    public let severity: Int
    public let message: String
}

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
        /// A language server published diagnostics for a file (the full
        /// current set — an empty array clears previous findings).
        case diagnostics(path: String, items: [CoreDiagnostic])
        /// A language-server instance changed state; `status` is one of
        /// starting/running/not-found/failed/exited.
        case serverStatus(server: String, root: String, status: String, message: String)
        /// A request response (internal: routed to its completion handler,
        /// never delivered to the app's event closure).
        case lspResponse(id: UInt64, json: String)
    }

    /// Retained context handed to the C callback as its `userdata`.
    private final class EventSink: @unchecked Sendable {
        let deliver: @Sendable (Event) -> Void

        init(deliver: @escaping @Sendable (Event) -> Void) {
            self.deliver = deliver
        }
    }

    /// Completion handlers for in-flight requests, keyed by request id.
    /// Register and complete both happen on the main actor (registration
    /// from the main-actor API, completion inside the main-queue delivery
    /// hop), so plain storage suffices.
    private final class ResponseRouter: @unchecked Sendable {
        private var pending: [UInt64: (String) -> Void] = [:]

        func register(_ id: UInt64, _ completion: @escaping (String) -> Void) {
            pending[id] = completion
        }

        func complete(_ id: UInt64, _ json: String) {
            pending.removeValue(forKey: id)?(json)
        }
    }

    private let handle: OpaquePointer
    private let sink: EventSink
    private let router: ResponseRouter

    /// Creates a core instance.
    ///
    /// - Parameter onEvent: called on the **main actor** for every core
    ///   event. The closure escapes for the lifetime of this object.
    public init(onEvent: @escaping @MainActor @Sendable (Event) -> Void) {
        let router = ResponseRouter()
        self.router = router
        let sink = EventSink { event in
            DispatchQueue.main.async {
                // Safe by construction: the main queue is the main actor's
                // executor. Dispatch (rather than Task) keeps delivery in
                // strict event order.
                MainActor.assumeIsolated {
                    // Request responses complete their registered handler
                    // instead of reaching the general event stream.
                    if case let .lspResponse(id, json) = event {
                        router.complete(id, json)
                    } else {
                        onEvent(event)
                    }
                }
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
            // Event strings are only valid during this call; copy now.
            let path = event.path.map { String(cString: $0) }
            let payload = event.payload.map { String(cString: $0) }
            switch event.kind {
            case UInt32(TC_EVENT_PONG):
                sink.deliver(.pong(sequence: event.seq))
            case UInt32(TC_EVENT_DIAGNOSTICS):
                guard let path, let payload else { return }
                let items = (try? JSONDecoder().decode(
                    [CoreDiagnostic].self, from: Data(payload.utf8))) ?? []
                sink.deliver(.diagnostics(path: path, items: items))
            case UInt32(TC_EVENT_SERVER_STATUS):
                sink.deliver(
                    .serverStatus(
                        server: event.server.map { String(cString: $0) } ?? "",
                        root: path ?? "",
                        status: event.status.map { String(cString: $0) } ?? "",
                        message: payload ?? ""
                    ))
            case UInt32(TC_EVENT_LSP_RESPONSE):
                sink.deliver(.lspResponse(id: event.seq, json: payload ?? "null"))
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

    // MARK: Language servers

    /// Announces an opened document; spawns its project's server instance
    /// on first use. No-op for languages without a registered server.
    public func lspDidOpen(path: String, language: String, text: String) {
        withUTF8(path) { path, pathLen in
            withUTF8(language) { language, languageLen in
                withUTF8(text) { text, textLen in
                    tc_lsp_did_open(
                        handle, path, pathLen, language, languageLen, text, textLen)
                }
            }
        }
    }

    /// Announces new document contents (full-text sync).
    public func lspDidChange(path: String, text: String) {
        withUTF8(path) { path, pathLen in
            withUTF8(text) { text, textLen in
                tc_lsp_did_change(handle, path, pathLen, text, textLen)
            }
        }
    }

    /// Requests completions at an LSP position; same contract as
    /// ``lspHover(path:line:character:completion:)``. The JSON is an LSP
    /// `CompletionItem[]` or `CompletionList`.
    @MainActor
    public func lspCompletion(
        path: String,
        line: Int,
        character: Int,
        completion: @escaping (String) -> Void
    ) {
        let id = withUTF8(path) { path, pathLen in
            tc_lsp_completion(
                handle, path, pathLen, UInt32(max(0, line)), UInt32(max(0, character)))
        }
        guard id != 0 else { return }
        router.register(id, completion)
    }

    /// Applies the configuration's `lsp` JSON to the server pool. Affects
    /// instances spawned afterwards.
    public func lspConfigure(json: String) {
        withUTF8(json) { json, jsonLen in
            tc_lsp_configure(handle, json, jsonLen)
        }
    }

    /// Shuts down every running server instance; re-announce open
    /// documents afterwards to respawn under the current configuration.
    public func lspRestartServers() {
        tc_lsp_restart_servers(handle)
    }

    /// Announces a closed document.
    public func lspDidClose(path: String) {
        withUTF8(path) { path, pathLen in
            tc_lsp_did_close(handle, path, pathLen)
        }
    }

    /// Requests hover information at an LSP position (zero-based line,
    /// UTF-16 column). The completion receives the response's `result` as
    /// JSON ("null" when the server has nothing to say), on the main
    /// actor; it is dropped silently when the document has no server.
    @MainActor
    public func lspHover(
        path: String,
        line: Int,
        character: Int,
        completion: @escaping (String) -> Void
    ) {
        let id = withUTF8(path) { path, pathLen in
            tc_lsp_hover(handle, path, pathLen, UInt32(max(0, line)), UInt32(max(0, character)))
        }
        guard id != 0 else { return }
        router.register(id, completion)
    }

    /// Requests the definition of the symbol at an LSP position; same
    /// contract as ``lspHover(path:line:character:completion:)``. The JSON
    /// is an LSP `Location`, `Location[]`, or `LocationLink[]`.
    @MainActor
    public func lspDefinition(
        path: String,
        line: Int,
        character: Int,
        completion: @escaping (String) -> Void
    ) {
        let id = withUTF8(path) { path, pathLen in
            tc_lsp_definition(
                handle, path, pathLen, UInt32(max(0, line)), UInt32(max(0, character)))
        }
        guard id != 0 else { return }
        router.register(id, completion)
    }
}

/// Runs `body` with a `(pointer, length)` view of the string's UTF-8.
private func withUTF8<R>(
    _ text: String, _ body: (UnsafePointer<CChar>?, UInt) -> R
) -> R {
    var text = text
    return text.withUTF8 { bytes in
        let pointer = bytes.baseAddress.map {
            UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
        }
        return body(pointer, UInt(bytes.count))
    }
}
