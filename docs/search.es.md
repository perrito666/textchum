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
lista el ámbito alfabéticamente. Los mismos filtros apilados de Buscar
en el proyecto aplican aquí — cada tipo de filtro refina las rutas
encontradas — y la línea de estado dice cuántas coincidencias podaron
los filtros.

## Buscar en el proyecto (⇧⌘F)

La consulta es una expresión regular; los resultados llegan como
`ruta:línea: texto`. ⏎ salta directamente a la línea coincidente. Los
resultados tienen tope (200) para seguir siendo instantáneos; refine el
patrón en lugar de desplazarse.

Las mayúsculas siguen la regla **smart case** que popularizó ripgrep:
una consulta toda en minúsculas coincide con cualquier caja, mientras
que una consulta con alguna mayúscula se busca tal cual. Así `todo`
encuentra `TODO`, y `TODO` solo encuentra `TODO`.

Una línea bajo los resultados dice qué hizo la búsqueda — «18 matches
in 4 files · 812 searched», «No matches in 812 files searched» o el
motivo de que no se pudiera buscar nada (un ámbito inexistente, uno
donde todo está ignorado o un patrón inválido, citado). Un resultado
vacío nunca es mudo: un patrón mal escrito o un ámbito equivocado se
anuncian en vez de parecer una ausencia de coincidencias.

## Filtros apilados

Bajo la consulta de Buscar en el proyecto, **＋ Add Filter** apila
refinamientos:

- **line contains / line excludes** — el texto de la línea coincidente;
- **file contains / file excludes** — la ruta del archivo del resultado.

Los filtros son subcadenas sin distinción de mayúsculas y se combinan con
*y*: líneas con `foo` donde también aparece `bar`, pero no en archivos
con `test` en el nombre, es la consulta `foo` más `line contains bar` más
`file excludes test`. Las exclusiones de archivo podan archivos enteros
antes siquiera de abrirlos, así que las búsquedas filtradas siguen siendo
tan rápidas como las simples.

## La pila de saltos

Cada salto — un resultado de búsqueda, Ir a la Definición, una
referencia, una entrada del esquema, un `chum` — recuerda de dónde
partió. **Go Back** (⌃⌘←) recorre esos orígenes; **Go Forward** (⌃⌘→)
los desanda. La historia se reescribe desde el punto actual: saltar a
un sitio nuevo descarta el rastro hacia adelante, exactamente como la
jumplist de vim. Las posiciones sobreviven a las ediciones como
línea/columna, así que volver tras un cambio aterriza cerca, no
perdido.

## Aún no está

- Reemplazar entre archivos.
- Conmutadores de mayúsculas/palabra completa en el panel (el propio
  patrón puede expresar ambos).
- Historial de búsquedas persistente.
