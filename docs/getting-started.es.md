# Primeros pasos

Por ahora Textchum se compila y ejecuta solo en macOS.

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
| `make docs` | Genera este sitio de documentación en `site/`. |
| `make clean` | Elimina todos los productos de compilación. |

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
