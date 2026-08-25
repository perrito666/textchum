# Primeros pasos

La plataforma de origen de Textchum es macOS; una carcasa experimental
para Linux sobre el mismo núcleo también compila — véase
[Linux](#linux-experimental) más abajo.

## Requisitos

- macOS 14 o más reciente.
- La cadena de herramientas de Swift, versión 6 o superior. Basta con las
  Xcode Command Line Tools (`xcode-select --install`); no hace falta la
  aplicación Xcode completa.
- Una cadena de herramientas de Rust (estable). Lo más sencillo es
  instalarla con [rustup](https://rustup.rs).
- `make`, incluido en las Command Line Tools.

## Compilar y ejecutar

```sh
git clone https://github.com/perrito666/textchum
cd textchum
make run
```

`make run` compila el núcleo en Rust como biblioteca estática, genera la
cabecera C, compila la aplicación Swift contra ella y lanza el editor.

Otros objetivos útiles:

| Objetivo | Qué hace |
|---|---|
| `make build` | Compila núcleo y aplicación sin lanzarla. |
| `make test` | Ejecuta la batería de pruebas de Rust. |
| `make smoke` | Compila todo y ejecuta la prueba de humo sin interfaz. |
| `make check` | Todo lo que ejecuta la CI: pruebas, prueba de humo y verificación de la cabecera. |
| `make app` | Construye un `Textchum.app` de doble clic (con icono) en `dist/`. |
| `make docs` | Genera este sitio de documentación en `site/`. |
| `make clean` | Elimina todos los productos de compilación. |

¿Sin ganas de compilar? Cada etiqueta `v*` publica una
[release de GitHub](https://github.com/perrito666/textchum/releases) con
un zip de `Textchum.app` listo para usar y tarballs Linux de
`textchum-gtk` (x86_64 y arm64), cada uno con su SHA-256. La aplicación
no está firmada, así que en el primer arranque haga clic derecho sobre
ella y elija Abrir.

## Estructura del repositorio

```
textchum/
├── core/                    espacio de trabajo de Rust
│   ├── textchum-core/         el núcleo del editor (búferes, eventos)
│   └── textchum-ffi/          ABI en C sobre el núcleo; genera textchum.h
├── macos/                   paquete Swift
│   └── Sources/
│       ├── CTextchum/         la cabecera C generada como módulo de Clang
│       ├── TextchumKit/       envoltorio Swift seguro sobre la interfaz C
│       └── Textchum/          la aplicación AppKit
├── docs/                    esta documentación (MkDocs)
└── Makefile                 el punto de entrada para todas las tareas
```

## El comando `chum`

**Textchum → Install chum Command…** (o, desde un checkout,
`make install-cli` — respeta `PREFIX`, por defecto `/usr/local`)
instala un pequeño comando de terminal que habla con la aplicación en
marcha; la vía del menú pide derechos de administración solo cuando
`/usr/local/bin` los necesita:

```sh
chum notas.md                # abrir (pestaña o ventana según sus ajustes)
chum +42 src/main.rs         # abrir con el cursor en la línea 42
chum -w grande.md            # forzar una ventana separada
chum -t a.rs +7 b.rs         # varios archivos, pestañas, uno con línea
chum --wait borrador.md      # bloquea hasta cerrar la ventana
```

`--wait` es lo que necesitan las herramientas que lanzan un editor y
leen el archivo después — guarde, cierre la ventana y quien llamó
continúa:

```sh
git config --global core.editor "chum --wait"
```

Cerrar sin guardar deja el archivo intacto, lo que git lee como un
commit abortado — el mismo gesto que `:q!`. Si Textchum se cierra (o no
está), los chum en espera se liberan en lugar de quedar colgados.

Funciona mediante el esquema de URL `textchum://`, así que el paquete de
la aplicación (`make app`) debe haberse lanzado al menos una vez para
registrarlo.

## Linux (experimental)

El mismo núcleo impulsa una carcasa nativa GTK4/libadwaita, enlazada
como crate de Rust en lugar de a través de la cabecera C (allí ambos
lados son Rust). Es más joven que la aplicación de macOS pero ya no es
un juguete: pestañas (AdwTabView) que enfocan en vez de duplicar, un
árbol de archivos del proyecto en barra lateral (F9), edición y
deshacer propiedad del núcleo, coloreado tree-sitter desde la tabla de
temas compartida, búsqueda en el archivo (Ctrl+F), búsqueda en el proyecto
(Ctrl+Shift+F, regex con smart case, filtros apilables de línea y
archivo, y línea de estado que dice qué hizo), apertura difusa (Ctrl+P), una lista de archivos abiertos
agrupada por proyecto sobre el árbol, el pool de servidores de
lenguaje conectado (diagnósticos subrayados con recuento de problemas,
autocompletado al escribir, hover, salto a la definición con F12,
problemas de servidor como avisos), un panel de vista previa de
Markdown en vivo (Ctrl+Alt+P) y una ventana de preferencias
(Ctrl+,) sobre el mismo contrato de `config.json` — apariencia, tema,
ajustes del editor y servidores de lenguaje — valores por defecto,
anulaciones por proyecto y los interruptores de espacio de trabajo —,
guardado en
`~/.config/textchum/config.json`.

```sh
sudo apt install libgtk-4-dev libadwaita-1-dev libgtksourceview-5-dev \
  libwebkitgtk-6.0-dev libsoup-3.0-dev
cargo build --release --manifest-path linux/Cargo.toml
linux/target/release/textchum-gtk notas.md
```

La CI lo compila y ejecuta su prueba de humo sin pantalla en cada push.

## Generar la documentación

La documentación es un sitio [MkDocs](https://www.mkdocs.org) con el tema
Material, publicada en inglés, español y francés. Es completamente estática:
el directorio `site/` generado puede servirse con cualquier servidor web.

```sh
python3 -m venv .docs-venv
.docs-venv/bin/pip install -r docs/requirements.txt
.docs-venv/bin/mkdocs serve    # vista previa con recarga en localhost:8000
.docs-venv/bin/mkdocs build    # sitio estático en site/
```

`make docs` envuelve estos mismos pasos.

## Resolución de problemas

- **Errores de `xcodebuild` sobre una "command line tools instance"** — es
  inofensivo; Textchum no usa `xcodebuild`. Compile con `make` (que invoca
  `swift build`).
- **El enlazador no encuentra `-ltextchum`** — el núcleo en Rust aún no se ha
  compilado. Ejecute `make core` (o cualquier objetivo de `make` que lo
  incluya) antes de invocar `swift build` a mano.
