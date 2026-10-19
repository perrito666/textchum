import CTextchum
import Foundation

/// The core's install catalog: what is missing for a document, and the
/// ways each missing program can be installed on this machine.
public enum CoreInstall {
    /// One way to install a program, ready to run.
    public struct Method: Decodable, Equatable, Sendable {
        /// The program that does the installing: `uv`, `npm`, `go`, …
        public let installer: String
        /// `project`, `user` or `system`.
        public let scope: String
        /// Where it goes, in words.
        public let title: String
        public let command: String
        /// Where it runs.
        public let directory: String
        /// False for a system package: it needs an administrator's
        /// password, so it is handed over rather than run.
        public let runs: Bool
    }

    /// A program something needed and did not find.
    public struct Offer: Decodable, Equatable, Sendable {
        public let program: String
        /// What it was for: "python language server", "debugger", …
        public let purpose: String
        public let project: String
        /// Where it comes from, when nothing here can install it.
        public let notes: [String]
        public let methods: [Method]
    }

    /// What is missing for the document at `path`, with how each can be
    /// installed.
    public static func offers(forPath path: String) -> [Offer] {
        let json = path.withCString { tc_install_offers($0, UInt(strlen($0))) }
        guard let json else { return [] }
        defer { tc_string_free(json) }
        return (try? JSONDecoder().decode([Offer].self, from: Data(String(cString: json).utf8))) ?? []
    }

    /// Records that `program` was needed for `project` and is not there.
    public static func noteMissing(program: String, project: String, purpose: String) {
        program.withCString { programPointer in
            project.withCString { projectPointer in
                purpose.withCString { purposePointer in
                    tc_install_note_missing(
                        programPointer, UInt(strlen(programPointer)), projectPointer,
                        UInt(strlen(projectPointer)), purposePointer, UInt(strlen(purposePointer)))
                }
            }
        }
    }

    /// Forgets `program` as missing: it was just installed.
    public static func forget(program: String) {
        program.withCString { tc_install_forget($0, UInt(strlen($0))) }
    }

    /// Whether the catalog knows how `program` is installed.
    public static func knows(program: String) -> Bool {
        program.withCString { tc_install_knows($0, UInt(strlen($0))) }
    }

    /// The words that run `program` for `project`, program first; nil
    /// when it is nowhere.
    public static func resolve(program: String, project: String?) -> [String]? {
        let project = project ?? ""
        let json = program.withCString { programPointer in
            project.withCString { projectPointer in
                tc_install_resolve(
                    programPointer, UInt(strlen(programPointer)), projectPointer,
                    UInt(strlen(projectPointer)))
            }
        }
        guard let json else { return nil }
        defer { tc_string_free(json) }
        return try? JSONDecoder().decode([String].self, from: Data(String(cString: json).utf8))
    }

    /// What a program run for `project` gets on top of the editor's own
    /// environment: the project's environment active.
    public static func environment(project: String) -> [String: String] {
        let json = project.withCString { tc_install_environment($0, UInt(strlen($0))) }
        guard let json else { return [:] }
        defer { tc_string_free(json) }
        return (try? JSONDecoder().decode([String: String].self, from: Data(String(cString: json).utf8)))
            ?? [:]
    }
}
