# Búsqueda

Dos maneras de encontrar cosas más allá del archivo actual, con una regla
compartida: **el ámbito es una ruta visible y editable.** Ambos paneles
muestran exactamente dónde miran — el proyecto del documento actual por
defecto — y ampliar la búsqueda es literalmente editar esa ruta (hasta
`~` o `/` si se quiere). La búsqueda nunca mira en silencio donde no se
espera.

Ambos recorridos respetan `.gitignore`, omiten archivos ocultos y limitan
el tamaño de archivo, cortesía del motor del propio ripgrep incrustado en
el núcleo — no un subproceso.

## Abrir rápidamente (⌘T)

Escriba fragmentos del nombre de archivo — `editwc` encuentra
`EditorWindowController.swift` — con coincidencia difusa y ranking al
estilo fzf. ↑/↓ mueven la selección, ⏎ abre (trayendo al frente la
ventana si el archivo ya está abierto), ⎋ cierra. Una consulta vacía
lista el ámbito alfabéticamente.

## Buscar en el proyecto (⇧⌘F)

La consulta es una expresión regular; los resultados llegan como
`ruta:línea: texto`. ⏎ salta directamente a la línea coincidente. Los
resultados tienen tope (200) para seguir siendo instantáneos; refine el
patrón en lugar de desplazarse.

## Aún no está

- Reemplazar entre archivos.
- Conmutadores de mayúsculas/palabra completa en el panel (el propio
  patrón puede expresar ambos).
- Historial de búsquedas persistente.
