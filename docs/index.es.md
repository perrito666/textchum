# Textchum

Textchum es un editor de texto para macOS en el espíritu de TextMate:
nativo, rápido y centrado en una sola tarea — **editar y validar una gran
variedad de tipos de archivo** — en lugar de ser un IDE. No hay botón de
ejecutar, ni depurador, ni un mercado de plugins en el horizonte; lo que hay
(o habrá) es coloreado de sintaxis para muchos lenguajes y validación basada
en servidores de lenguaje que respeta los límites de cada proyecto.

## Cómo está construido

Textchum se divide en dos mitades:

- **El núcleo** (`libtextchum`), escrito en Rust, es dueño de todo lo
  relativo al texto: búferes, ediciones y el flujo de eventos que mantiene
  informada a la interfaz. Se compila como biblioteca estática con una
  interfaz C plana y no sabe nada de macOS.
- **La carcasa** (*shell*), escrita en Swift con AppKit, es dueña de todo lo
  relativo a la plataforma: ventanas, dibujado, entrada y menús. Nunca
  mantiene estado propio del documento — cada edición pasa por el núcleo.

Esta división mantiene la lógica interesante portable y comprobable sin
interfaz gráfica, mientras que la capa visible para la persona usuaria es
completamente nativa. La [página de arquitectura](architecture.md) explica el
razonamiento y las reglas de la frontera.

## Estado actual

Textchum es joven. Lo que existe y funciona hoy:

- Un núcleo en Rust que expone búferes de texto basados en *ropes* a través
  de una ABI en C, con edición por desplazamientos de bytes y por unidades
  UTF-16 (esta última coincide con cómo AppKit y el Language Server Protocol
  direccionan el texto).
- Un canal de eventos asíncrono desde los hilos de trabajo del núcleo hacia
  la interfaz, con un contrato estricto de entrega en un solo hilo.
- Una aplicación macOS mínima: una ventana, una vista de texto editable,
  sincronizada con un búfer del núcleo mediante un protocolo que impide que
  ambos lados diverjan.
- Una prueba de humo sin interfaz que ejercita el ciclo completo
  Swift ↔ núcleo, usada por la integración continua y por humanos con prisa.

Lo próximo, en orden aproximado: gestión real de documentos (abrir, guardar,
codificaciones, deshacer), coloreado de sintaxis, servidores de lenguaje por
proyecto y vista previa de Markdown.

## Por dónde seguir

- [Primeros pasos](getting-started.md) — compilar y ejecutar Textchum desde
  el código fuente.
- [Arquitectura](architecture.md) — la división núcleo/carcasa y sus reglas.
- [La frontera C](ffi.md) — convenciones de la interfaz entre ambos.
