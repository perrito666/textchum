import CTextchum
import Foundation

/// The core's project records, as Swift values.
///
/// A record says what each file of a project remembers about itself:
/// how many views it is shown in, the dividers between them, what is
/// folded, what language it was told it is, and where each view was
/// looking. The format, the atomic write and the sweep are the core's.
public enum CoreProjectState {
    /// What one file remembers.
    public struct FileState: Equatable {
        public var views: Int
        public var dividers: [Double]
        /// Folded stretches, as first and last line, both zero-based.
        public var folds: [(start: Int, end: Int)]
        public var language: String?
        public var places: [Place]

        public struct Place: Equatable {
            public var caret: Int
            public var scroll: Double
            /// The first character shown, in UTF-16 units.
            public var top: Int

            public init(caret: Int = 0, scroll: Double = 0, top: Int = 0) {
                self.caret = caret
                self.scroll = scroll
                self.top = top
            }
        }

        public init(
            views: Int = 1,
            dividers: [Double] = [],
            folds: [(start: Int, end: Int)] = [],
            language: String? = nil,
            places: [Place] = []
        ) {
            self.views = views
            self.dividers = dividers
            self.folds = folds
            self.language = language
            self.places = places
        }

        public static func == (a: FileState, b: FileState) -> Bool {
            a.views == b.views && a.dividers == b.dividers && a.language == b.language
                && a.places == b.places
                && a.folds.count == b.folds.count
                && zip(a.folds, b.folds).allSatisfy { $0 == $1 }
        }

        /// Whether this is worth writing down at all.
        public var isEmpty: Bool {
            views <= 1 && dividers.isEmpty && folds.isEmpty && language == nil
                && places.allSatisfy { $0.caret == 0 && $0.scroll == 0 }
        }
    }

    /// One record, as the cleanup window lists them.
    public struct Record: Identifiable {
        public var root: String
        public var path: String
        public var bytes: Int
        public var updated: Date
        public var missing: Bool
        public var files: Int

        public var id: String { path }
    }

    public static func fileState(
        root: String, directory: String, inProject: Bool, path: String
    ) -> FileState? {
        let json = root.withCString { rootPointer in
            directory.withCString { dirPointer in
                path.withCString { pathPointer in
                    tc_project_file_state(
                        rootPointer, UInt(strlen(rootPointer)),
                        dirPointer, UInt(strlen(dirPointer)),
                        inProject,
                        pathPointer, UInt(strlen(pathPointer)))
                }
            }
        }
        guard let json else { return nil }
        defer { tc_string_free(json) }
        let text = String(cString: json)
        guard let data = text.data(using: .utf8),
            let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
            !object.isEmpty
        else { return nil }
        return read(object)
    }

    @discardableResult
    public static func setFileState(
        _ state: FileState, root: String, directory: String, inProject: Bool, path: String
    ) -> Bool {
        let json = write(state)
        return root.withCString { rootPointer in
            directory.withCString { dirPointer in
                path.withCString { pathPointer in
                    json.withCString { jsonPointer in
                        tc_project_set_file_state(
                            rootPointer, UInt(strlen(rootPointer)),
                            dirPointer, UInt(strlen(dirPointer)),
                            inProject,
                            pathPointer, UInt(strlen(pathPointer)),
                            jsonPointer, UInt(strlen(jsonPointer)))
                    }
                }
            }
        }
    }

    public static func records(directory: String) -> [Record] {
        let json = directory.withCString { pointer in
            tc_project_records(pointer, UInt(strlen(pointer)))
        }
        guard let json else { return [] }
        defer { tc_string_free(json) }
        let text = String(cString: json)
        guard let data = text.data(using: .utf8),
            let items = (try? JSONSerialization.jsonObject(with: data)) as? [[String: Any]]
        else { return [] }
        return items.compactMap { item in
            guard let root = item["root"] as? String, let path = item["path"] as? String
            else { return nil }
            return Record(
                root: root,
                path: path,
                bytes: item["bytes"] as? Int ?? 0,
                updated: Date(timeIntervalSince1970: item["updated"] as? Double ?? 0),
                missing: item["missing"] as? Bool ?? false,
                files: item["files"] as? Int ?? 0)
        }
    }

    @discardableResult
    public static func sweep(directory: String, keepDays: UInt64) -> Int {
        Int(
            directory.withCString { pointer in
                tc_project_sweep(pointer, UInt(strlen(pointer)), keepDays)
            })
    }

    @discardableResult
    public static func forget(recordAt path: String) -> Bool {
        path.withCString { pointer in tc_project_forget(pointer, UInt(strlen(pointer))) }
    }

    // MARK: JSON

    private static func read(_ object: [String: Any]) -> FileState {
        FileState(
            views: object["views"] as? Int ?? 1,
            dividers: object["dividers"] as? [Double] ?? [],
            folds: (object["folds"] as? [[Int]] ?? []).compactMap { pair in
                pair.count == 2 ? (pair[0], pair[1]) : nil
            },
            language: object["language"] as? String,
            places: (object["places"] as? [[String: Any]] ?? []).map { place in
                FileState.Place(
                    caret: place["caret"] as? Int ?? 0,
                    scroll: place["scroll"] as? Double ?? 0,
                    top: place["top"] as? Int ?? 0)
            })
    }

    private static func write(_ state: FileState) -> String {
        var object: [String: Any] = ["views": max(1, state.views)]
        if !state.dividers.isEmpty { object["dividers"] = state.dividers }
        if !state.folds.isEmpty { object["folds"] = state.folds.map { [$0.start, $0.end] } }
        if let language = state.language { object["language"] = language }
        if !state.places.isEmpty {
            object["places"] = state.places.map {
                ["caret": $0.caret, "scroll": $0.scroll, "top": $0.top]
            }
        }
        guard let data = try? JSONSerialization.data(withJSONObject: object),
            let text = String(data: data, encoding: .utf8)
        else { return "{}" }
        return text
    }
}
