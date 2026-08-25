# Recetas por lenguaje

Recetas que funcionan para los sospechosos habituales: qué instalar
para el servidor y los formateadores de cada lenguaje, y la
configuración que los conecta. Cada fragmento va en `config.json`
(ver [Configuración](configuration.md)) o, equivalentemente, en
Ajustes ▸ Servidores de lenguaje — aquí se muestran las formas
editadas a mano porque son más fáciles de copiar. Los servidores
listados coinciden con el registro integrado, así que instalar la
herramienta suele bastar; las entradas `lsp` de abajo solo hacen falta
cuando se quiere un servidor distinto del predeterminado.

Las líneas de instalación asumen [Homebrew](https://brew.sh) en macOS;
en Linux, el gestor de paquetes o el instalador del propio lenguaje
hacen el mismo trabajo.

## Python

```bash
brew install pyright ruff black
```

(o `npm install -g pyright`, `pip install ruff black`.)

Pyright es el servidor predeterminado. Ruff corrige y Black formatea
en cada guardado:

```json
{
  "preprocessors": {
    "defaults": { "python": ["ruff check --fix-only -", "black -"] }
  }
}
```

¿Prefieres `python-lsp-server`? Instálalo
(`pip install python-lsp-server`) y apunta el lenguaje hacia él:

```json
{ "lsp": { "defaults": { "python": "pylsp" } } }
```

## Go

```bash
brew install go gopls
```

`gopls` se encuentra automáticamente. `gofmt` viene con el propio Go:

```json
{ "preprocessors": { "defaults": { "go": ["gofmt"] } } }
```

Cambia a `goimports`
(`go install golang.org/x/tools/cmd/goimports@latest`) para gestionar
también los imports al guardar.

## Rust

```bash
rustup component add rust-analyzer rustfmt
```

`rust-analyzer` es el servidor predeterminado. `rustfmt` lee stdin al
llamarlo sin argumentos:

```json
{ "preprocessors": { "defaults": { "rust": ["rustfmt"] } } }
```

## JavaScript

```bash
npm install -g typescript typescript-language-server prettier
```

`typescript-language-server` es el servidor predeterminado. Prettier
elige su parser por el nombre del archivo, así que pásale el real —
`{filename}` se expande a él:

```json
{
  "preprocessors": {
    "defaults": { "javascript": ["prettier --stdin-filepath {filename}"] }
  }
}
```

## C

```bash
brew install llvm
```

`clangd` y `clang-format` vienen ambos con LLVM (las herramientas de
línea de comandos de Xcode también traen un `clangd`). `clang-format`
lee stdin por defecto y respeta el `.clang-format` del proyecto:

```json
{ "preprocessors": { "defaults": { "c": ["clang-format"] } } }
```

## Swift

`sourcekit-lsp` viene con Xcode y se encuentra automáticamente.
`swift-format` viene con las toolchains recientes:

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

Prettier cubre los tres; Markdown además tiene la vista previa
integrada y la corrección ortográfica de prosa sin configurar nada:

```json
{
  "preprocessors": {
    "defaults": {
      "json": ["prettier --stdin-filepath {filename}"],
      "yaml": ["prettier --stdin-filepath {filename}"],
      "markdown": ["prettier --stdin-filepath {filename}"]
    }
  }
}
```

## Comprobar una receta

Abre un archivo del lenguaje y mira el subtítulo de la ventana: los
contadores de problemas aparecen cuando el servidor responde. Si no
pasa nada, la página de
[Servidores de lenguaje](language-servers.md) cubre el registro de
depuración (`~/Library/Logs/Textchum/lsp.log`), las reglas de PATH y
el respaldo con ctags para proyectos sin servidor. Los fallos de los
preprocesadores siempre salen como una alerta con el comando y su
stderr — una cadena nunca puede tragarse un guardado en silencio.
