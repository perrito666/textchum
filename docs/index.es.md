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
- Documentos sobre los búferes: abrir y guardar con detección de
  codificación y escrituras atómicas, deshacer/rehacer con fusión de tecleo
  y seguimiento de cambios anclado al último guardado — véase
  [Documentos](documents.md).
- Configuración respaldada en JSON con una ventana de ajustes gráfica que
  escribe directamente en el archivo: las ediciones a mano sobreviven y los
  archivos rotos recurren a los valores por defecto y se respaldan en lugar
  de pisarse — véase [Configuración](configuration.md).
- Un canal de eventos asíncrono desde los hilos de trabajo del núcleo hacia
  la interfaz, con un contrato estricto de entrega en un solo hilo.
- Un editor macOS con varias ventanas (y pestañas nativas), paneles de
  abrir/guardar, avisos de guardado al cerrar, buscar y reemplazar con
  expresiones regulares y vigilancia en vivo de los archivos abiertos ante
  cambios de otros programas; cada vista de texto se mantiene sincronizada
  con su documento del núcleo mediante un protocolo que impide que ambos
  lados diverjan.
- Una prueba de humo sin interfaz que ejercita el ciclo completo
  Swift ↔ núcleo — edición, deshacer, guardado, reapertura, eventos —,
  usada por la integración continua y por humanos con prisa.

Lo próximo, en orden aproximado: coloreado de sintaxis, servidores de
lenguaje por proyecto y vista previa de Markdown.

## Por dónde seguir

- [Primeros pasos](getting-started.md) — compilar y ejecutar Textchum desde
  el código fuente.
- [Arquitectura](architecture.md) — la división núcleo/carcasa y sus reglas.
- [Documentos](documents.md) — deshacer, estado de cambios, codificaciones,
  guardados atómicos.
- [Configuración](configuration.md) — la ventana de ajustes y su archivo
  JSON.
- [La frontera C](ffi.md) — convenciones de la interfaz entre ambos.
