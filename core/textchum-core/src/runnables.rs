//! What in a file can be run: its `main`, each test, each benchmark —
//! and the command line that runs just that one.
//!
//! Read from the syntax tree, so it needs no language server and works
//! the same in both shells: a Rust `#[test]` function, a Go `TestXxx`
//! in a `_test.go` file, a pytest `test_` function or `Test` class
//! method, a `main`. Each gets the command its own tool takes to run
//! exactly that one, in the directory that tool expects to be run
//! from. Other languages have none yet.

use std::path::{Path, PathBuf};

use ropey::Rope;
use tree_sitter::Node;

/// One thing that can be run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Runnable {
    /// The zero-based lines it spans: the mark goes on the first.
    pub line: usize,
    pub end_line: usize,
    /// `main`, `test` or `bench`.
    pub kind: &'static str,
    /// What it is called, the way its tool names it: `tests::adds`,
    /// `TestAdd`, `test_x.py::test_adds`.
    pub name: String,
    pub command: String,
    pub cwd: PathBuf,
}

/// The runnables of a file in `language` at `path`, from its syntax
/// tree, in the order they appear.
pub fn find(language: &str, path: &Path, root: Node<'_>, rope: &Rope) -> Vec<Runnable> {
    let text = |node: Node<'_>| rope.byte_slice(node.start_byte()..node.end_byte()).to_string();
    match language {
        "rust" => rust(path, root, &text),
        "go" => go(path, root, &text),
        "python" => python(path, root, &text),
        _ => Vec::new(),
    }
}

/// A word safe on a shell's command line, quoted when it needs to be.
pub fn quote(word: &str) -> String {
    if !word.is_empty() && word.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:=+,@".contains(c)) {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

fn span(node: Node<'_>) -> (usize, usize) {
    (node.start_position().row, node.end_position().row)
}

fn children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

/// The directory above `path` that holds `manifest`, nearest first.
fn holding(path: &Path, manifest: &str) -> Option<PathBuf> {
    path.ancestors().skip(1).find(|dir| dir.join(manifest).is_file()).map(Path::to_owned)
}

// MARK: Rust

fn rust(path: &Path, root: Node<'_>, text: &dyn Fn(Node<'_>) -> String) -> Vec<Runnable> {
    let Some(package) = holding(path, "Cargo.toml") else { return Vec::new() };
    let Ok(relative) = path.strip_prefix(&package) else { return Vec::new() };
    let parts: Vec<String> = relative.iter().map(|part| part.to_string_lossy().into_owned()).collect();
    let stem = |name: &str| name.trim_end_matches(".rs").to_owned();
    // What the file is to cargo: the target it belongs to, and the
    // module path of its top level within that target.
    let (target, module): (Option<String>, Vec<String>) = match parts.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["src", "main.rs"] | ["src", "lib.rs"] => (None, vec![]),
        ["src", "bin", file] => (Some(format!("--bin {}", quote(&stem(file)))), vec![]),
        ["src", "bin", name, "main.rs"] => (Some(format!("--bin {}", quote(name))), vec![]),
        ["examples", file] => (Some(format!("--example {}", quote(&stem(file)))), vec![]),
        ["tests", file] => (Some(format!("--test {}", quote(&stem(file)))), vec![]),
        ["src", ref inner @ ..] => {
            let mut module: Vec<String> = inner.iter().map(|part| stem(part)).collect();
            if module.last().map(String::as_str) == Some("mod") {
                module.pop();
            }
            (None, module)
        }
        _ => return Vec::new(),
    };
    let mut found = Vec::new();
    collect_rust(root, &module, root, text, &mut |node, kind, path_name| {
        let (line, end_line) = span(node);
        let command = match kind {
            "main" => match &target {
                Some(target) if !target.starts_with("--test") => format!("cargo run {target}"),
                _ => "cargo run".to_owned(),
            },
            "bench" => format!("cargo bench -- {} --exact", quote(&path_name)),
            _ => match &target {
                Some(target) if target.starts_with("--test") => {
                    format!("cargo test {target} -- {} --exact", quote(&path_name))
                }
                Some(target) => format!("cargo test {target} -- {} --exact", quote(&path_name)),
                None => format!("cargo test -- {} --exact", quote(&path_name)),
            },
        };
        found.push(Runnable { line, end_line, kind, name: path_name, command, cwd: package.clone() });
    });
    found
}

fn collect_rust(
    node: Node<'_>,
    module: &[String],
    root: Node<'_>,
    text: &dyn Fn(Node<'_>) -> String,
    found: &mut dyn FnMut(Node<'_>, &'static str, String),
) {
    let items = children(node);
    for (at, item) in items.iter().enumerate() {
        match item.kind() {
            "function_item" => {
                let Some(name) = item.child_by_field_name("name").map(|name| text(name)) else { continue };
                // The attributes right above it.
                let attributes: Vec<String> = items[..at]
                    .iter()
                    .rev()
                    .take_while(|before| matches!(before.kind(), "attribute_item" | "line_comment" | "block_comment"))
                    .filter(|before| before.kind() == "attribute_item")
                    .map(|attribute| text(*attribute))
                    .collect();
                let path_name = module.iter().cloned().chain([name.clone()]).collect::<Vec<_>>().join("::");
                if attributes.iter().any(|attribute| attribute_is(attribute, "test")) {
                    found(*item, "test", path_name);
                } else if attributes.iter().any(|attribute| attribute_is(attribute, "bench")) {
                    found(*item, "bench", path_name);
                } else if name == "main" && node == root && module.is_empty() {
                    found(*item, "main", "main".to_owned());
                }
            }
            "mod_item" => {
                let (Some(name), Some(body)) = (item.child_by_field_name("name"), item.child_by_field_name("body"))
                else {
                    continue;
                };
                let inner: Vec<String> = module.iter().cloned().chain([text(name)]).collect();
                collect_rust(body, &inner, root, text, found);
            }
            _ => {}
        }
    }
}

/// Whether an attribute is `#[name]` or `#[something::name(...)]`:
/// `#[test]`, `#[tokio::test]`, `#[tokio::test(flavor = "multi_thread")]`.
fn attribute_is(attribute: &str, name: &str) -> bool {
    let inside = attribute.trim_start_matches("#[").trim_start_matches("#![");
    let path: String = inside.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':').collect();
    path == name || path.ends_with(&format!("::{name}"))
}

// MARK: Go

fn go(path: &Path, root: Node<'_>, text: &dyn Fn(Node<'_>) -> String) -> Vec<Runnable> {
    let Some(dir) = path.parent() else { return Vec::new() };
    let file = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let is_test_file = file.ends_with("_test.go");
    let is_main_package = children(root).iter().any(|node| {
        node.kind() == "package_clause" && children(*node).first().is_some_and(|name| text(*name) == "main")
    });
    // Go's own rule: Test, or Test followed by anything but a lowercase
    // letter; the same for benchmarks.
    let named = |name: &str, prefix: &str| {
        name.strip_prefix(prefix).is_some_and(|rest| !rest.starts_with(|c: char| c.is_lowercase()))
    };
    let mut found = Vec::new();
    for node in children(root) {
        if node.kind() != "function_declaration" {
            continue;
        }
        let Some(name) = node.child_by_field_name("name").map(|name| text(name)) else { continue };
        let (line, end_line) = span(node);
        let runnable = |kind: &'static str, command: String| Runnable {
            line,
            end_line,
            kind,
            name: name.clone(),
            command,
            cwd: dir.to_path_buf(),
        };
        if is_test_file && named(&name, "Test") {
            found.push(runnable("test", format!("go test -run {} .", quote(&format!("^{name}$")))));
        } else if is_test_file && named(&name, "Benchmark") {
            found.push(runnable("bench", format!("go test -run '^$' -bench {} .", quote(&format!("^{name}$")))));
        } else if !is_test_file && is_main_package && name == "main" {
            found.push(runnable("main", "go run .".to_owned()));
        }
    }
    found
}

// MARK: Python

/// A definition and what it defines, through its decorators.
fn definition(node: Node<'_>) -> Option<(Node<'_>, Node<'_>)> {
    let inner = if node.kind() == "decorated_definition" { node.child_by_field_name("definition")? } else { node };
    Some((node, inner))
}

fn python(path: &Path, root: Node<'_>, text: &dyn Fn(Node<'_>) -> String) -> Vec<Runnable> {
    // pytest is run from the project, which is where its configuration
    // and the imports it resolves are.
    let cwd = crate::workspace::project_root_for(path)
        .or_else(|| path.parent().map(Path::to_owned))
        .unwrap_or_default();
    let relative = path.strip_prefix(&cwd).unwrap_or(path).to_string_lossy().into_owned();
    let file = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let is_test_file = (file.starts_with("test_") || file.ends_with("_test.py")) && file.ends_with(".py");
    let mut found = Vec::new();
    let test = |outer: Node<'_>, id: String, found: &mut Vec<Runnable>| {
        let (line, end_line) = span(outer);
        let id = format!("{relative}::{id}");
        let command = format!("python3 -m pytest {}", quote(&id));
        found.push(Runnable { line, end_line, kind: "test", name: id, command, cwd: cwd.clone() });
    };
    for node in children(root) {
        if is_test_file {
            let Some((outer, inner)) = definition(node) else { continue };
            let Some(name) = inner.child_by_field_name("name").map(|name| text(name)) else { continue };
            match inner.kind() {
                "function_definition" if name.starts_with("test") => test(outer, name, &mut found),
                "class_definition" if name.starts_with("Test") => {
                    let Some(body) = inner.child_by_field_name("body") else { continue };
                    for member in children(body) {
                        let Some((outer, method)) = definition(member) else { continue };
                        let Some(method_name) = method.child_by_field_name("name").map(|name| text(name)) else {
                            continue;
                        };
                        if method.kind() == "function_definition" && method_name.starts_with("test") {
                            test(outer, format!("{name}::{method_name}"), &mut found);
                        }
                    }
                }
                _ => {}
            }
        } else if node.kind() == "if_statement" {
            let condition = node.child_by_field_name("condition").map(|condition| text(condition)).unwrap_or_default();
            let squeezed: String = condition.chars().filter(|c| !c.is_whitespace()).collect();
            if squeezed == "__name__==\"__main__\"" || squeezed == "__name__=='__main__'" {
                let (line, end_line) = span(node);
                found.push(Runnable {
                    line,
                    end_line,
                    kind: "main",
                    name: relative.clone(),
                    command: format!("python3 {}", quote(&relative)),
                    cwd: cwd.clone(),
                });
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::SyntaxState;

    fn runnables(language: &str, path: &Path, source: &str) -> Vec<Runnable> {
        let rope = Rope::from_str(source);
        let state = SyntaxState::new(crate::syntax::languages::by_name(language).unwrap(), &rope).unwrap();
        state.runnables(path, &rope)
    }

    fn project(name: &str, manifest: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("textchum-runnables-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(manifest), "").unwrap();
        dir
    }

    #[test]
    fn rust_tests_get_their_module_path_and_main_its_run() {
        let package = project("rust", "Cargo.toml");
        let source = "fn main() {}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn adds() {}\n\n    // a note\n    #[tokio::test(flavor = \"multi_thread\")]\n    async fn waits() {}\n\n    fn helper() {}\n}\n";
        let found = runnables("rust", &package.join("src/main.rs"), source);
        let summary: Vec<(&str, usize, &str)> =
            found.iter().map(|runnable| (runnable.kind, runnable.line, runnable.command.as_str())).collect();
        assert_eq!(
            summary,
            [
                ("main", 0, "cargo run"),
                ("test", 5, "cargo test -- tests::adds --exact"),
                ("test", 9, "cargo test -- tests::waits --exact"),
            ]
        );
        assert_eq!(found[0].cwd, package);
        // A module file, and an integration test.
        let module = runnables("rust", &package.join("src/net/mod.rs"), "#[test]\nfn binds() {}\n");
        assert_eq!(module[0].command, "cargo test -- net::binds --exact");
        let integration = runnables("rust", &package.join("tests/api.rs"), "#[test]\nfn serves() {}\n");
        assert_eq!(integration[0].command, "cargo test --test api -- serves --exact");
        let _ = std::fs::remove_dir_all(&package);
    }

    #[test]
    fn go_tests_benchmarks_and_main() {
        let module = project("go", "go.mod");
        let source = "package shop\n\nfunc TestSell(t *testing.T) {}\nfunc Testable() {}\nfunc BenchmarkSell(b *testing.B) {}\n";
        let found = runnables("go", &module.join("shop_test.go"), source);
        let commands: Vec<&str> = found.iter().map(|runnable| runnable.command.as_str()).collect();
        assert_eq!(commands, ["go test -run '^TestSell$' .", "go test -run '^$' -bench '^BenchmarkSell$' ."]);
        let main = runnables("go", &module.join("main.go"), "package main\n\nfunc main() {}\n");
        assert_eq!((main[0].kind, main[0].command.as_str()), ("main", "go run ."));
        let _ = std::fs::remove_dir_all(&module);
    }

    #[test]
    fn pytest_functions_and_class_methods_and_a_main_block() {
        let root = project("python", "pyproject.toml");
        std::fs::create_dir_all(root.join("tests")).unwrap();
        let source = "import pytest\n\ndef test_adds():\n    pass\n\n@pytest.mark.slow\ndef test_waits():\n    pass\n\nclass TestShop:\n    def test_sells(self):\n        pass\n\n    def helper(self):\n        pass\n";
        let found = runnables("python", &root.join("tests/test_shop.py"), source);
        let commands: Vec<&str> = found.iter().map(|runnable| runnable.command.as_str()).collect();
        assert_eq!(
            commands,
            [
                "python3 -m pytest tests/test_shop.py::test_adds",
                "python3 -m pytest tests/test_shop.py::test_waits",
                "python3 -m pytest tests/test_shop.py::TestShop::test_sells",
            ]
        );
        assert_eq!(found[1].line, 5, "the mark goes on the decorator, where the definition starts");
        let main = runnables("python", &root.join("tool.py"), "def run():\n    pass\n\nif __name__ == '__main__':\n    run()\n");
        assert_eq!((main[0].kind, main[0].command.as_str(), main[0].line), ("main", "python3 tool.py", 3));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn words_are_quoted_only_when_they_need_it() {
        assert_eq!(quote("tests::adds"), "tests::adds");
        assert_eq!(quote("a b"), "'a b'");
        assert_eq!(quote("it's"), r"'it'\''s'");
    }
}
