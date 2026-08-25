# Recettes par langage

Des recettes qui marchent pour les suspects habituels : quoi installer
pour le serveur et les formateurs de chaque langage, et la
configuration qui les relie. Chaque extrait va dans `config.json`
(voir [Configuration](configuration.md)) ou, de façon équivalente,
dans Réglages ▸ Serveurs de langage — les formes éditées à la main
sont montrées ici parce qu'elles se copient plus facilement. Les
serveurs listés correspondent au registre intégré : installer l'outil
suffit en général ; les entrées `lsp` ci-dessous ne servent que pour
choisir un serveur différent de celui par défaut.

Les lignes d'installation supposent [Homebrew](https://brew.sh) sur
macOS ; sous Linux, le gestionnaire de paquets ou l'installateur du
langage font le même travail.

## Python

```bash
brew install pyright ruff black
```

(ou `npm install -g pyright`, `pip install ruff black`.)

Pyright est le serveur par défaut. Ruff corrige et Black formate à
chaque sauvegarde :

```json
{
  "preprocessors": {
    "defaults": { "python": ["ruff check --fix-only -", "black -"] }
  }
}
```

Vous préférez `python-lsp-server` ? Installez-le
(`pip install python-lsp-server`) et pointez le langage dessus :

```json
{ "lsp": { "defaults": { "python": "pylsp" } } }
```

## Go

```bash
brew install go gopls
```

`gopls` est trouvé automatiquement. `gofmt` vient avec Go lui-même :

```json
{ "preprocessors": { "defaults": { "go": ["gofmt"] } } }
```

Passez à `goimports`
(`go install golang.org/x/tools/cmd/goimports@latest`) pour gérer
aussi les imports à la sauvegarde.

## Rust

```bash
rustup component add rust-analyzer rustfmt
```

`rust-analyzer` est le serveur par défaut. `rustfmt` lit stdin quand
on l'appelle sans arguments :

```json
{ "preprocessors": { "defaults": { "rust": ["rustfmt"] } } }
```

## JavaScript

```bash
npm install -g typescript typescript-language-server prettier
```

`typescript-language-server` est le serveur par défaut. Prettier a
besoin d'un indice de nom de fichier pour choisir son parseur —
n'importe quel nom avec la bonne extension convient :

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

`clangd` et `clang-format` viennent tous deux avec LLVM (les outils en
ligne de commande de Xcode embarquent aussi un `clangd`).
`clang-format` lit stdin par défaut et respecte le `.clang-format` du
projet :

```json
{ "preprocessors": { "defaults": { "c": ["clang-format"] } } }
```

## Swift

`sourcekit-lsp` vient avec Xcode et est trouvé automatiquement.
`swift-format` accompagne les toolchains récentes :

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

Prettier couvre les trois ; Markdown a en plus l'aperçu intégré et la
correction orthographique de la prose sans rien configurer :

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

## Vérifier une recette

Ouvrez un fichier du langage et regardez le sous-titre de la fenêtre :
les compteurs de problèmes apparaissent quand le serveur répond. Si
rien ne se passe, la page
[Serveurs de langage](language-servers.md) couvre le journal de
débogage (`~/Library/Logs/Textchum/lsp.log`), les règles de PATH et le
recours à ctags pour les projets sans serveur. Un échec de
préprocesseur s'affiche toujours en alerte avec la commande et son
stderr — une chaîne ne peut jamais avaler une sauvegarde en silence.
