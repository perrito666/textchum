# Configuración

Los ajustes de Textchum siguen un principio: **la interfaz gráfica es la
forma cómoda de cambiarlos, y un archivo JSON plano es la salida de
emergencia siempre disponible.** Hay exactamente un almacén — el archivo —
y la ventana de Ajustes lo lee y lo escribe; nada vive solo dentro de la
aplicación.

## La ventana de Ajustes

**Textchum → Settings…** (⌘,) edita los ajustes reconocidos:

- **Apariencia** — seguir al sistema (cambiando en vivo cuando macOS lo
  hace) o forzar claro u oscuro.
- **Abrir archivos en** — pestañas de la ventana actual (el valor por
  defecto) o ventanas separadas. Con ventanas separadas, el navegador de
  cada ventana lista solo los documentos de su propio grupo de pestañas.
- **Tipografía** — cualquier familia de ancho fijo instalada en el sistema,
  o la tipografía monoespaciada de la plataforma.
- **Tamaño de letra** — de 6 a 72 puntos.
- **Ancho de tabulación** — de 1 a 16 columnas.
- **Mostrar números de línea** — el margen, también conmutables por
  sesión con View → Toggle Line Numbers (⇧⌘L).

Cada cambio se aplica de inmediato a las ventanas abiertas del editor y se
escribe a disco en el mismo momento. No hay botón de Aplicar o Guardar que
olvidar.

## El archivo

Los ajustes viven en:

```
~/Library/Application Support/Textchum/config.json
```

Un archivo editado a mano podría verse así:

```json
{
  "appearance": "dark",
  "editor": {
    "font_family": "JetBrains Mono",
    "font_size": 13,
    "tab_width": 4
  }
}
```

`appearance` acepta `"system"`, `"light"` u `"dark"`; si se omite (el
valor por defecto), se sigue al sistema.

Todo es opcional: un archivo, una sección o una clave ausentes significan
simplemente el valor por defecto. Las escrituras son atómicas (archivo
temporal más renombrado), como toda escritura que hace Textchum.

Dos garantías hacen segura la edición a mano:

- **Las claves desconocidas sobreviven.** La ventana de ajustes reescribe
  solo las claves que le pertenecen. Cualquier otra cosa en el archivo —
  anotaciones propias, claves de una versión más nueva — se conserva
  literalmente en cada guardado.
- **Los archivos rotos nunca se pisotean.** Si el archivo no se puede
  analizar, Textchum arranca con los ajustes por defecto, lo avisa una vez
  al iniciarse y deja el archivo exactamente como estaba para que pueda
  arreglarse en cualquier editor — incluido el propio Textchum. Si se
  cambia un ajuste desde la interfaz mientras el archivo está roto, el
  original inanalizable se copia primero a `config.json.bak` y solo
  entonces se reemplaza.

Los valores fuera de rango o con tipo incorrecto no cuentan como rotura: un
`font_size` de `4000` se recorta al rango válido, un `font_family` de `42`
se ignora y el resto del archivo funciona con normalidad.

## Proyectos

La pestaña Projects decide dónde empieza y termina un proyecto — el
límite por el que agrupa el navegador y con el que el pool de servidores
de lenguaje identifica sus instancias. Ambos interruptores existen dos
veces: como valor por defecto para todos los proyectos y por raíz de
proyecto. Una fila añadida con el campo de ruta (que completa nombres de
directorio al escribir y lleva un botón Browse…) anula los valores por
defecto solo para esa raíz.

- **Manifest projects** — normalmente gana el repositorio más externo:
  abrir un archivo en cualquier punto dentro de un repositorio convierte
  al repositorio en el proyecto, por muchos `Cargo.toml` o
  `pyproject.toml` que haya en medio. Activarlo vuelve a dividir una
  raíz por manifiestos de lenguaje, de modo que los módulos anidados son
  proyectos propios.
- **Recursive config** — hace que los ajustes por proyecto de una raíz
  (sus comandos de servidor de lenguaje y estos mismos interruptores) se
  apliquen a los proyectos anidados dentro de ella, con prioridad para
  el ancestro más cercano. Útil en monorepos: una configuración arriba,
  muchos proyectos debajo.
- **Ctags fallback** — responde Ir a la Definición desde un índice de
  Universal Ctags cuando no hay servidor de lenguaje disponible; véase
  [servidores de lenguaje](language-servers.es.md).

En el archivo, esto vive en una sección `workspace`:

```json
{
  "workspace": {
    "manifest_projects": false,
    "recursive_config": false,
    "ctags_fallback": false,
    "projects": {
      "/Users/you/code/monorepo": {
        "manifest_projects": true,
        "recursive_config": true
      }
    }
  }
}
```

## Atajos de teclado

Los atajos de los menús se reasignan mediante una sección `keys` editada
a mano (sin interfaz todavía): un objeto de nombres de acción a
especificaciones `modificadores+tecla`, aplicado al arrancar.

```json
{
  "keys": {
    "openQuickly": "cmd+p",
    "goToBlockEnd": "ctrl+alt+down",
    "findInProject": "cmd+shift+g"
  }
}
```

Modificadores: `cmd`, `shift`, `alt`, `ctrl`. Teclas: un carácter, o
`up`/`down`/`left`/`right`/`return`/`escape`/`space`/`tab`/`delete`.
Entre las acciones: `new`, `open`, `openQuickly`, `save`, `saveAs`,
`close`, `undo`, `redo`, `find`, `findAndReplace`, `findNext`,
`findPrevious`, `useSelectionForFind`, `findInProject`,
`jumpToDefinition`, `goToBlockStart`, `goToBlockEnd`,
`toggleNavigator`, `togglePreview`, `toggleLineNumbers`, `settings` —
un nombre desconocido se registra junto a la lista completa. Ir al
inicio/fin de bloque (⌃⌥↑/⌃⌥↓ por defecto) salta sobre el bloque
sintáctico multilínea más interno alrededor del cursor, cortesía del
mismo árbol que alimenta el coloreado.

## Aún no está

- Textchum todavía no vigila el archivo mientras se ejecuta; los cambios
  hechos en otro editor se aplican en el siguiente arranque.
- Ajustes del editor por proyecto (tipografía, ancho de tabulación) —
  los proyectos ya tienen sus propios ajustes de detección y de
  servidores de lenguaje, pero no estos.
