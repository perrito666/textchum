# Language setups

Working recipes for the usual suspects: what to install for each
language's server and formatters, and the configuration that wires
them up. Every snippet goes in `config.json` (see
[Configuration](configuration.md)) or, equivalently, through Settings ▸
Language Servers — these are the hand-edited forms because they are
easier to copy. Servers listed here match the built-in registry, so
installing the tool is usually enough; the `lsp` entries below are
only needed when you want a different server than the default.

Install lines assume [Homebrew](https://brew.sh) on macOS; on Linux,
your package manager or the language's own installer does the same
job.

## Python

```bash
brew install pyright ruff black
```

(or `npm install -g pyright`, `pip install ruff black`.)

Pyright is the default server. Ruff fixes and Black formats on every
save:

```json
{
  "preprocessors": {
    "defaults": { "python": ["ruff check --fix-only -", "black -"] }
  }
}
```

Prefer `python-lsp-server`? Install it (`pip install python-lsp-server`)
and point the language at it:

```json
{ "lsp": { "defaults": { "python": "pylsp" } } }
```

## Go

```bash
brew install go gopls
```

`gopls` is found automatically. `gofmt` ships with Go itself:

```json
{ "preprocessors": { "defaults": { "go": ["gofmt"] } } }
```

Swap in `goimports` (`go install golang.org/x/tools/cmd/goimports@latest`)
to also manage imports on save.

## Rust

```bash
rustup component add rust-analyzer rustfmt
```

`rust-analyzer` is the default server. `rustfmt` reads stdin when
called plainly:

```json
{ "preprocessors": { "defaults": { "rust": ["rustfmt"] } } }
```

## JavaScript

```bash
npm install -g typescript typescript-language-server prettier
```

`typescript-language-server` is the default server. Prettier needs a
filename hint to pick its parser — any name with the right extension
works:

```json
{
  "preprocessors": {
    "defaults": { "javascript": ["prettier --stdin-filepath file.js"] }
  }
}
```

## C

```bash
brew install llvm
```

`clangd` and `clang-format` both come with LLVM (Xcode's command-line
tools also carry a `clangd`). `clang-format` reads stdin by default
and picks up the project's `.clang-format`:

```json
{ "preprocessors": { "defaults": { "c": ["clang-format"] } } }
```

## Swift

`sourcekit-lsp` ships with Xcode and is found automatically.
`swift-format` comes with recent toolchains:

```json
{ "preprocessors": { "defaults": { "swift": ["swift format"] } } }
```

## Shell

```bash
brew install bash-language-server shfmt shellcheck
```

```json
{ "preprocessors": { "defaults": { "bash": ["shfmt"] } } }
```

## JSON / YAML / Markdown

Prettier handles all three; Markdown also gets the built-in preview
and prose spell checking without any setup:

```json
{
  "preprocessors": {
    "defaults": {
      "json": ["prettier --stdin-filepath file.json"],
      "yaml": ["prettier --stdin-filepath file.yaml"],
      "markdown": ["prettier --stdin-filepath file.md"]
    }
  }
}
```

## Checking a setup

Open a file of the language and watch the window subtitle: problem
counts appear when the server answers. When nothing happens, the
[Language servers](language-servers.md) page covers the debug log
(`~/Library/Logs/Textchum/lsp.log`), the PATH rules, and the ctags
fallback for projects without a server. Preprocessor failures always
surface as an alert naming the command and its stderr — a chain can
never eat a save silently.
