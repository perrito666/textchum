import CTextchum
import Foundation

/// One language-server finding, positioned the LSP way: zero-based line,
/// UTF-16 column.
///
/// A finding read back from a document (``CoreDocument/diagnostics``)
/// also carries `start` and `end`, its range in UTF-16 units of the
/// text as it is now. One straight from a server has not been placed
/// yet, and reads as zero there.
public struct CoreDiagnostic: Codable, Equatable, Sendable {
    public let line: Int
    public let character: Int
    public let endLine: Int
    public let endCharacter: Int
    /// 1 = error, 2 = warning, 3 = information, 4 = hint.
    public let severity: Int
    public let message: String
    public let start: Int
    public let end: Int

    public init(
        line: Int, character: Int, endLine: Int, endCharacter: Int,
        severity: Int, message: String, start: Int = 0, end: Int = 0
    ) {
        self.line = line
        self.character = character
        self.endLine = endLine
        self.endCharacter = endCharacter
        self.severity = severity
        self.message = message
        self.start = start
        self.end = end
    }

    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        line = try values.decode(Int.self, forKey: .line)
        character = try values.decode(Int.self, forKey: .character)
        endLine = try values.decode(Int.self, forKey: .endLine)
        endCharacter = try values.decode(Int.self, forKey: .endCharacter)
        severity = try values.decode(Int.self, forKey: .severity)
        message = try values.decode(String.self, forKey: .message)
        start = try values.decodeIfPresent(Int.self, forKey: .start) ?? 0
        end = try values.decodeIfPresent(Int.self, forKey: .end) ?? 0
    }
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

    /// The pool's live instances as (server id, project root) pairs.
    public func lspRunning() -> [(server: String, root: String)] {
        guard let cString = tc_lsp_running(handle) else { return [] }
        defer { tc_string_free(cString) }
        return String(cString: cString)
            .split(separator: "\n")
            .compactMap { line in
                let halves = line.split(separator: "\u{1f}", maxSplits: 1)
                guard halves.count == 2 else { return nil }
                return (String(halves[0]), String(halves[1]))
            }
    }

    /// Shuts down every running server instance; re-announce open
    /// documents afterwards to respawn under the current configuration.
    public func lspRestartServers() {
        tc_lsp_restart_servers(handle)
    }

    /// Forgets one crashed (server, root) instance; re-announce the
    /// affected documents afterwards to spawn a replacement.
    public func lspRetire(server: String, root: String) {
        withUTF8(server) { server, serverLen in
            withUTF8(root) { root, rootLen in
                tc_lsp_retire(handle, server, serverLen, root, rootLen)
            }
        }
    }

    /// Announces a closed document.
    public func lspDidClose(path: String) {
        withUTF8(path) { path, pathLen in
            tc_lsp_did_close(handle, path, pathLen)
        }
    }

    /// Tells the document's server it was written to disk, which is
    /// when servers that check on save run.
    public func lspDidSave(path: String) {
        withUTF8(path) { path, pathLen in
            tc_lsp_did_save(handle, path, pathLen)
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

    /// Requests the other uses, within the document, of the symbol at
    /// an LSP position; same contract as
    /// ``lspHover(path:line:character:completion:)``. The JSON is an LSP
    /// `DocumentHighlight[]`.
    @MainActor
    public func lspDocumentHighlight(
        path: String,
        line: Int,
        character: Int,
        completion: @escaping (String) -> Void
    ) {
        let id = withUTF8(path) { path, pathLen in
            tc_lsp_document_highlight(
                handle, path, pathLen, UInt32(max(0, line)), UInt32(max(0, character)))
        }
        guard id != 0 else { return }
        router.register(id, completion)
    }

    /// What asking the server to format on the spot came to.
    public enum FormatAnswer: Equatable, Sendable {
        case formatted(String)
        /// Nothing was formatted, and why: no server that formats, or
        /// none that answered in time. A reason to carry on with the
        /// text as it is.
        case skipped(String)
    }

    /// Has the document's server format `text` and waits for the
    /// result, for the `@format` link of a save chain. Blocks for up to
    /// `wait` seconds.
    @MainActor
    public func lspFormatNow(path: String, text: String, tabSize: Int, wait: TimeInterval = 2)
        -> FormatAnswer
    {
        let answer = withUTF8(path) { path, pathLen in
            withUTF8(text) { text, textLen in
                tc_lsp_format_now(
                    handle, path, pathLen, text, textLen, UInt32(max(1, tabSize)),
                    UInt32(max(0, wait) * 1000))
            }
        }
        guard let answer else { return .skipped("the language-server pool is not available") }
        defer { tc_string_free(answer) }
        let parsed =
            (try? JSONSerialization.jsonObject(with: Data(String(cString: answer).utf8)))
            as? [String: Any]
        if let formatted = parsed?["text"] as? String { return .formatted(formatted) }
        return .skipped(parsed?["error"] as? String ?? "no answer")
    }

    /// Requests the signature of the call an LSP position is inside;
    /// same contract as ``lspHover(path:line:character:completion:)``.
    /// ``CoreSignature/active(fromResultJSON:)`` reduces the answer.
    @MainActor
    public func lspSignatureHelp(
        path: String,
        line: Int,
        character: Int,
        completion: @escaping (String) -> Void
    ) {
        let id = withUTF8(path) { path, pathLen in
            tc_lsp_signature_help(
                handle, path, pathLen, UInt32(max(0, line)), UInt32(max(0, character)))
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

    /// Requests the document's symbol tree; same contract as
    /// ``lspHover(path:line:character:completion:)``. The JSON is an LSP
    /// `DocumentSymbol[]` (hierarchical) or `SymbolInformation[]` (flat).
    @MainActor
    public func lspDocumentSymbols(
        path: String,
        completion: @escaping (String) -> Void
    ) {
        let id = withUTF8(path) { path, pathLen in
            tc_lsp_document_symbols(handle, path, pathLen)
        }
        guard id != 0 else { return }
        router.register(id, completion)
    }

    /// Requests every reference to the symbol at an LSP position
    /// (declaration included); same contract as
    /// ``lspHover(path:line:character:completion:)``. The JSON is an LSP
    /// `Location[]`.
    @MainActor
    public func lspReferences(
        path: String,
        line: Int,
        character: Int,
        completion: @escaping (String) -> Void
    ) {
        let id = withUTF8(path) { path, pathLen in
            tc_lsp_references(
                handle, path, pathLen, UInt32(max(0, line)), UInt32(max(0, character)))
        }
        guard id != 0 else { return }
        router.register(id, completion)
    }

    /// Requests a workspace-wide rename of the symbol at an LSP position;
    /// same contract as ``lspHover(path:line:character:completion:)``.
    /// The JSON is an LSP `WorkspaceEdit`.
    @MainActor
    public func lspRename(
        path: String,
        line: Int,
        character: Int,
        newName: String,
        completion: @escaping (String) -> Void
    ) {
        let id = withUTF8(path) { path, pathLen in
            withUTF8(newName) { name, nameLen in
                tc_lsp_rename(
                    handle, path, pathLen, UInt32(max(0, line)), UInt32(max(0, character)),
                    name, nameLen)
            }
        }
        guard id != 0 else { return }
        router.register(id, completion)
    }

    /// Requests the code actions offered at an LSP position — the quick
    /// fixes and refactorings a server has for it.
    ///
    /// The findings under the caret go with the request, as the server
    /// itself published them — the core keeps them, so the caller does
    /// not have to carry them back. That is what turns the answer into
    /// quick fixes rather than the refactorings the range allows.
    @MainActor
    public func lspCodeAction(
        path: String,
        line: Int,
        character: Int,
        completion: @escaping (String) -> Void
    ) {
        let id = withUTF8(path) { path, pathLen in
            tc_lsp_code_action(
                handle, path, pathLen, UInt32(max(0, line)), UInt32(max(0, character)))
        }
        guard id != 0 else { return }
        router.register(id, completion)
    }

    /// Sends a code action back to have its edit filled in.
    @MainActor
    public func lspResolveCodeAction(
        path: String,
        actionJSON: String,
        completion: @escaping (String) -> Void
    ) {
        let id = withUTF8(path) { path, pathLen in
            withUTF8(actionJSON) { action, actionLen in
                tc_lsp_resolve_code_action(handle, path, pathLen, action, actionLen)
            }
        }
        guard id != 0 else { return }
        router.register(id, completion)
    }

    /// Runs a command a code action carried instead of an edit.
    @MainActor
    public func lspExecuteCommand(
        path: String,
        command: String,
        argumentsJSON: String,
        completion: @escaping (String) -> Void
    ) {
        let id = withUTF8(path) { path, pathLen in
            withUTF8(command) { name, nameLen in
                withUTF8(argumentsJSON) { arguments, argumentsLen in
                    tc_lsp_execute_command(
                        handle, path, pathLen, name, nameLen, arguments, argumentsLen)
                }
            }
        }
        guard id != 0 else { return }
        router.register(id, completion)
    }

    /// Requests whole-document formatting; same contract as
    /// ``lspHover(path:line:character:completion:)``. The JSON is an LSP
    /// `TextEdit[]`.
    @MainActor
    public func lspFormatting(
        path: String,
        tabSize: Int,
        insertSpaces: Bool,
        completion: @escaping (String) -> Void
    ) {
        let id = withUTF8(path) { path, pathLen in
            tc_lsp_formatting(
                handle, path, pathLen, UInt32(max(1, tabSize)), insertSpaces)
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

/// The language servers Textchum knows how to talk to, and whether they
/// are actually on this machine. A settings screen that lists only what
/// has been overridden cannot say what there is to configure, nor why a
/// freshly installed server did nothing.
public enum CoreLSPRegistry {
    public struct Server: Decodable {
        public let id: String
        /// The full command line, arguments included.
        public let command: String
        public let languages: [String]
        public let installHint: String
    }

    public static let all: [Server] = {
        guard let cString = tc_lsp_registry_json() else { return [] }
        defer { tc_string_free(cString) }
        let json = String(cString: cString)
        return (try? JSONDecoder().decode([Server].self, from: Data(json.utf8))) ?? []
    }()

    /// Whether a command would start: an absolute path that exists, or a
    /// bare name found on `PATH`. Only the first word is looked at,
    /// because that is what the pool runs.
    public static func isInstalled(_ command: String) -> Bool {
        var command = command
        return command.withUTF8 { bytes in
            tc_lsp_executable_exists(
                bytes.baseAddress.map {
                    UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                },
                UInt(bytes.count)
            )
        }
    }
}

/// The signature of the call being typed, reduced by the core to the
/// line to show and the stretch of it that is the current parameter.
public struct CoreSignature: Decodable, Equatable, Sendable {
    public let label: String
    public let start: Int?
    public let end: Int?
    public let documentation: String

    /// The active parameter's range in `label`, in UTF-16 units.
    public var parameterRange: NSRange? {
        guard let start, let end, end >= start else { return nil }
        return NSRange(location: start, length: end - start)
    }

    /// The signature a `signatureHelp` result means; nil when the
    /// server had nothing to show.
    public static func active(fromResultJSON json: String) -> CoreSignature? {
        var json = json
        let reduced = json.withUTF8 { bytes in
            tc_signature_active_json(
                bytes.baseAddress.map {
                    UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                },
                UInt(bytes.count))
        }
        guard let reduced else { return nil }
        defer { tc_string_free(reduced) }
        return try? JSONDecoder().decode(
            CoreSignature.self, from: Data(String(cString: reduced).utf8))
    }
}
