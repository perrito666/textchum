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
- **Tema** — la paleta de sintaxis; véase [Temas](#temas) más abajo.
- **Iconos de archivo** — un paquete de iconos de VS Code para el
  árbol de archivos; ver [Iconos de archivo](#iconos-de-archivo).
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
    "tab_width": 4,
    "hover": false
  }
}
```

`appearance` acepta `"system"`, `"light"` u `"dark"`; si se omite (el
valor por defecto), se sigue al sistema. `editor.hover` apaga el globo
de documentación al reposar el ratón (`true`, el valor por defecto, lo
mantiene encendido). `editor.new_files_in` coloca los documentos
nuevos en una `"tab"` del grupo de la ventana frontal (el valor por
defecto) o en una `"window"` propia. `editor.mark_occurrences` (`true`
por defecto) marca los demás lugares donde aparece la palabra
seleccionada; `editor.occurrences_case_sensitive` y
`editor.occurrences_whole_word` deciden qué cuenta como uno, ambos
`true` por defecto.

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

### En otro sitio, por una ejecución

`--data-dir <ruta>` guarda todo lo que Textchum posee bajo un único
directorio durante esa ejecución —la configuración, los temas, los
paquetes de iconos, la sesión y el registro del servidor de lenguaje—
en vez de en los sitios de siempre:

```bash
Textchum --data-dir ~/perfil-de-pruebas
```

Es un perfil entero hecho para la ocasión y desechado después, sin que
el de verdad se abra nunca; `make playground` lo usa, y también
cualquier otra cosa que no deba tocar tus ajustes. En Linux una
ejecución con perfil propio es su propio proceso, ya que entregar los
archivos a una instancia ya en marcha los abriría en el perfil de esa
instancia.

`--config <ruta>` es la versión estrecha: apunta a un archivo de
configuración, y la sesión lo acompaña.

## Temas

El selector **Theme** de la pestaña General elige la paleta de
sintaxis. Se incluyen siete de serie: **Textchum** (el predeterminado),
**Textchum High Contrast**, **Graphite** (uno apagado casi monocromo) y
los clásicos — **Molokai**, **Solarized**, **Dracula** y **Gruvbox**.
Cada tema lleva una paleta clara y una oscura en un mismo archivo, de
modo que un tema sirve para ambos modos de apariencia (los clásicos
nacidos oscuros emparejan su paleta canónica con una clara ajustada en
contraste; Solarized y Gruvbox usan sus paletas claras genuinas).

Los temas propios son archivos JSON en:

```
~/Library/Application Support/Textchum/themes/
```

seleccionados por nombre de archivo (sin `.json`); un archivo con el
nombre de un tema incorporado lo reemplaza. **Textchum → Open Themes
Folder** abre (y crea) este directorio. La forma más rápida de
empezar uno es generar un arranque completo — cada nombre de captura
con estilo, relleno con la paleta por defecto — y solo cambiar colores:

```bash
Textchum --emit-theme ~/Library/Application\ Support/Textchum/themes/Mio.json
```

Las entradas asocian nombres de captura de tree-sitter a estilos:

```json
{
  "name": "Mio",
  "styles": {
    "keyword": {"light": "#AD3DA4", "dark": "#FC5FA3", "bold": true},
    "comment": {"light": "#707F8C", "dark": "#7F8C98", "italic": true}
  }
}
```

Los colores son `#RRGGBB` o `#RRGGBBAA`. Todo lo omitido — un color,
un indicador, una captura entera — conserva el valor de la paleta por
defecto, así que un tema solo necesita decir lo que cambia. Las reglas
de salida de emergencia son las de la configuración: un tema que no se
puede analizar cae al predeterminado con un único aviso y nunca se
sobrescribe, y las claves desconocidas sobreviven. Los archivos de tema
se leen al arrancar y al cambiar la selección.

### Importar uno de otro editor

**Textchum → Importar tema** trae los colores de VS Code o TextMate.
Elige un archivo de tema, o una carpeta con varios — un directorio de
extensión de VS Code (su `package.json` dice qué aporta) o un bundle de
TextMate (sus temas están en `Themes/`). Se importa todo lo que haya, y
el primero queda puesto.

Los dos editores describen el color por **ámbito de TextMate**, así que
importar es traducir ámbitos a los nombres de captura de Textchum:
`entity.name.function` pasa a ser `function`, y `keyword.control.loop`
pasa a ser `repeat`. Los ámbitos se detienen donde las capturas siguen
—ningún tema colorea `if` distinto de `while`— así que una captura que
el origen nunca nombró toma el color de aquella de la que es un caso
particular, en cualquier dirección: un tema que dice `keyword` colorea
todas las clases de palabra clave, y uno que sólo dice
`constant.numeric` colorea toda la familia de constantes.

Dos cosas conviene saber antes de que los colores parezcan mal:

- **Un tema llena una apariencia.** Los dos editores escriben un tema
  para fondo claro o para fondo oscuro; los de Textchum llevan ambos.
  La importación llena el lado que el origen declara y deja el otro con
  la paleta por omisión, y dice cuál llenó. Importar un tema oscuro con
  el editor en apariencia clara no cambia nada visible.
- **Los ámbitos sin destino se nombran.** Todo lo que el origen coloreó
  y a lo que ninguna captura responde se lista al terminar. Esos
  colores quedan sin usar.

Lo que sale es un archivo de tema corriente en la carpeta de temas,
editable como cualquier otro.

## Iconos de archivo

El árbol de archivos dibuja un icono por fila. Sin un paquete es el que
ofrezca el escritorio para el tipo del archivo, que distingue Python de
Markdown y no llega mucho más lejos — y nunca ha oído hablar de un
archivo llamado `Dockerfile`.

**Ajustes → General → Iconos de archivo** acepta un **paquete de
iconos de VS Code**. Los paquetes ya vistos están en la lista,
separados entre los importados aquí y los abiertos donde están;
*System icons* es la vuelta atrás.

**Import…** copia el paquete a la carpeta propia de Textchum —
`~/Library/Application Support/Textchum/icons/` en macOS,
`~/.local/share/textchum/icons/` en Linux— así que mover o borrar el
original no se lleva los iconos. **Open…** apunta a un paquete donde
está y lo recuerda, que es lo adecuado para uno que mantienes tú. Ambos
aceptan el archivo JSON del tema de iconos o la carpeta de extensión
que lo contiene (su `package.json` dice cuál es). **Delete** borra un
paquete importado; uno abierto desde otro sitio es de quien lo puso
ahí, así que solo se puede quitar de la lista.

La elección es una ruta en `config.json`, y los paquetes abiertos desde
otro sitio se recuerdan a su lado:

```json
{
  "icon_pack": "~/packs/material-icon-theme/dist/material-icons.json",
  "icon_packs": ["~/packs/material-icon-theme/dist/material-icons.json"]
}
```

Un paquete cuya carpeta ya no está desaparece de la lista en vez de
quedarse ahí para fallar al elegirlo. Un paquete que no se puede leer
se avisa una vez y el árbol conserva los iconos del sistema.

La búsqueda sigue la de VS Code, de lo más específico a lo menos:

1. El nombre entero del archivo (`Dockerfile`, `cargo.toml`), en
   minúsculas.
2. La extensión más larga que coincida: `component.test.ts` prueba
   `test.ts` antes que `ts`.
3. El lenguaje que Textchum decidió que es el archivo — que es también
   como llega al icono un lenguaje fijado a mano en **Propiedades del
   archivo**.
4. El valor por omisión del propio paquete.

La sección `light` de un paquete sustituye cualquiera de esos sobre
fondo claro, una búsqueda a la vez, así que un paquete que solo redibuja
unos pocos conserva el resto.

Quedan fuera dos cosas. **Los iconos de carpeta**: el árbol dibuja los
suyos. **Los iconos por tipografía** —las definiciones `fontCharacter`
que usan Seti y sus descendientes— necesitan la tipografía instalada y
un texto donde va una imagen; un paquete que no tenga otra cosa se
rechaza dando esa razón, en vez de cargarse para no dibujar nada.

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

## Preprocesadores de guardado

Los formateadores y correctores pueden ejecutarse automáticamente antes
de cada guardado, por lenguaje — para todos los proyectos o para una
raíz concreta, exactamente como los servidores de lenguaje. Cada
entrada es una cadena: un comando por línea, en orden, donde cada
comando lee el documento por la entrada estándar y escribe el documento
completo por la salida estándar (la convención `-` que siguen casi
todos los formateadores). Si un eslabón falla — salida distinta de
cero, salida vacía, o más de diez segundos colgado — no se aplica nada,
se muestra el error (con el stderr de la herramienta) y el guardado
pregunta si continuar sin procesar.

```json
{
  "preprocessors": {
    "defaults": {
      "python": ["ruff check --fix -", "black -"],
      "go": ["gofmt"]
    },
    "projects": {
      "/work/site": { "javascript": ["prettier --stdin-filepath {filename}"] }
    }
  }
}
```

`{path}` y `{filename}` en cualquier parte de un comando se expanden
a la ruta absoluta del documento y a su nombre — para herramientas que
leen stdin pero deducen su comportamiento del nombre, como el
`--stdin-filepath` de Prettier. Un documento sin título ofrece
`Untitled` más la extensión de su lenguaje.

Una entrada de proyecto reemplaza la cadena por defecto para ese
lenguaje, nunca se añade a ella. La ventana de Ajustes edita esta misma
sección bajo Servidores de lenguaje, y **Edición ▸ Ejecutar
preprocesadores** (⌃⌥⌘F, nombre de acción `runPreprocessors`) ejecuta
la cadena a demanda sin guardar — formatear con tus herramientas en
vez del formateador del servidor. El resultado llega como una sola
edición, así que ⌘Z lo deshace.

## Corrección ortográfica

La prosa usa el corrector del sistema — los mismos diccionarios que
comparte toda app del Mac — acotado a donde la prosa vive de verdad:
los comentarios en el código, y el documento entero en Markdown,
mensajes de commit de git y texto plano. Los identificadores y las
cadenas literales nunca se marcan. Las faltas llevan un tinte púrpura,
distinto del rojo/naranja/azul de los diagnósticos.

Elige el idioma en Ajustes ▸ General ▸ «Spell check prose» — Apagado
(el valor por defecto), automático por contenido, o un diccionario
concreto — o pon `editor.spell` a mano: `"auto"` o un identificador
como `"es"` o `"en_US"`. Los diccionarios disponibles son los
habilitados en Ajustes del Sistema ▸ Teclado ▸ Entrada de texto.

```json
{ "editor": { "spell": "auto" } }
```

Pueden aplicarse varios diccionarios a la vez: nómbralos separados por
comas. Una palabra que cualquiera de ellos conozca está bien escrita,
que es lo que necesita un texto que cambia de idioma a mitad de
párrafo:

```json
{ "editor": { "spell": "en_US, es_ES" } }
```

`editor.spell_words` es tu propia lista: nombres de proyecto, siglas y
todo aquello que ningún diccionario trae. Al hacer clic derecho sobre
una palabra marcada aparecen las sugerencias, **Añadir al diccionario**,
que escribe la palabra aquí, e **Ignorar**, que la acepta hasta cerrar
el editor. La lista también se edita en los ajustes.

```json
{ "editor": { "spell_words": ["SBX", "Textchum"] } }
```

En Linux los mismos ajustes usan hunspell: instala `hunspell` más un
paquete de diccionario (`hunspell-es`, `hunspell-en-us`, …) y las
marcas aparecen; `"auto"` sigue a `$LANG`, y los diccionarios que
hunspell encuentra se listan junto al campo en las preferencias.

## Autoguardado

Desactivado por defecto. `editor.autosave` son segundos; el reloj se
reinicia con cada pulsación, así que el guardado ocurre cuando dejas de
escribir y no en mitad de una frase.

```json
{ "editor": { "autosave": 30 } }
```

Dos cosas que a propósito no hace. Nunca guarda un documento sin
nombre: no hay dónde ponerlo, e inventarle uno no le corresponde al
editor. Y no ejecuta los preprocesadores de guardado: un formateador
que te reordena la línea que estás escribiendo no es un favor, así que
eso se queda en los guardados explícitos.

## Atajos de teclado

Ajustes ▸ Teclado los tiene: un perfil, y cada comando con el atajo al
que responde, editable ahí mismo.

**Perfiles.** La gente llega de otro editor con sus atajos en los
dedos, así que los tres por los que se los conoce vienen con la
compilación — Visual Studio Code, Sublime Text e IntelliJ IDEA. Un
perfil nombra los comandos que mueve y deja el resto en paz, así que
elegir uno cambia aquello por lo que ese editor es conocido y nada
más. `keys_profile` guarda la elección; vacío son los atajos propios
de Textchum.

Cambiar un atajo sobre un perfil conserva el perfil: el cambio es una
anulación, y **Reset changes** las descarta todas. **Save as profile**
convierte lo que está en vigor en un perfil propio — la manera de
modificar un preajuste, que viene con la compilación. Los perfiles
guardados viven en `key_profiles`, y uno que reutilice un nombre
incluido lo reemplaza.

El archivo escribe las anulaciones como una sección `keys`: un objeto
de nombres de acción a especificaciones `modificadores+tecla`, aplicado
sobre el perfil.

```json
{
  "keys": {
    "openQuickly": "cmd+p",
    "goToBlockEnd": "ctrl+alt+down",
    "findInProject": "cmd+shift+g"
  }
}
```

Modificadores: `cmd`, `shift`, `alt`, `ctrl` — `cmd` es Command en
macOS y Ctrl en Linux, así que un perfil significa lo mismo en ambos.
Teclas: un carácter, de `f1` a `f20`, o
`up`/`down`/`left`/`right`/`return`/`escape`/`space`/`tab`/`delete`.
Entre las acciones: `new`, `open`, `openQuickly`, `save`, `saveAs`,
`close`, `undo`, `redo`, `find`, `findAndReplace`, `findNext`,
`findPrevious`, `useSelectionForFind`, `findInProject`,
`jumpToDefinition`, `findReferences`, `codeActions`, `renameSymbol`, `formatDocument`,
`runPreprocessors`,
`documentOutline`, `goBack`, `goForward`,
`blameLine`, `goToLine`,
`goToBlockStart`, `goToBlockEnd`,
`toggleNavigator`, `togglePreview`, `toggleLineNumbers`,
`toggleHover`, `showHover`, `serverStatus`, `newWithFormat`, `revealInTree`, `reopenClosed`,
`togglePathDisplay`, `redraw`, `commandPalette`, `settings` —
un nombre desconocido se registra junto a la lista completa. Ir al
inicio/fin de bloque (⌃⌥↑/⌃⌥↓ por defecto) salta sobre el bloque
sintáctico multilínea más interno alrededor del cursor, cortesía del
mismo árbol que alimenta el coloreado. Y cuando un atajo se escapa de
la memoria por completo, la **paleta de comandos** (⇧⌘P) busca de forma
difusa cualquier acción de menú por su nombre y ejecuta la selección.

## Recarga en vivo

El archivo se vigila mientras Textchum corre: edita `config.json` en
otro editor y el cambio se aplica en cuanto aterriza — apariencia,
tema, tipografías, atajos, tabla de servidores, todo, incluida la
ventana de Ajustes si está abierta. Los guardados de la propia app se
reconocen y se ignoran, y un archivo que momentáneamente no parsea cae
a los valores por defecto sin ser sobrescrito, igual que al arrancar.

## Ajustes del editor por proyecto

Una raíz de proyecto puede sobrescribir la tipografía, su tamaño y el
ancho de tabulación para todas las ventanas dentro de ella — las filas
de la pestaña Proyectos llevan los tres campos (vacío significa
«heredar el valor general»), y el archivo lo escribe como un objeto
`editor` en la entrada del workspace:

```json
{
  "workspace": {
    "projects": {
      "/work/legacy": { "editor": { "tab_width": 8, "font_size": 12 } }
    }
  }
}
```

## Añadir un proyecto

La pestaña Proyectos lista las raíces de los documentos abiertos, así
que un proyecto se añade eligiéndolo y no escribiendo su ruta.
**Copiar ajustes de** parte de uno ya configurado — sus servidores,
comandos de guardado, indicadores y ajustes del editor, todo — que es
lo que necesita un segundo servicio con la misma disposición. La misma
opción está en la fila de cada proyecto, para copiar sobre uno que ya
existe.

Un campo de anulación vacío muestra lo que hereda, así que una casilla
en blanco dice qué se aplica en vez de dejarte ir a mirar.

Una raíz configurada cuyo directorio ya no está se marca como *missing*,
y **Remove missing** olvida todas: nada volverá a coincidir con esas
entradas.

## Lenguajes que la compilación no conoce

El coloreado de Textchum viene de gramáticas tree-sitter compiladas en
el binario. Un lenguaje que no trae se puede nombrar en `languages`,
con la gramática como biblioteca compilada y su consulta de resaltado
como archivo:

```json
{
  "languages": {
    "dockerfile": {
      "grammar": "~/.local/share/textchum/grammars/libtree-sitter-dockerfile.dylib",
      "highlights": "~/.local/share/textchum/grammars/dockerfile/highlights.scm",
      "extensions": ["dockerfile"],
      "filenames": ["Dockerfile", "Containerfile"]
    }
  }
}
```

`aliases`, `filenames` e `injections` son opcionales, y `symbol`
también: el constructor es `tree_sitter_<nombre>` salvo que se nombre,
con guiones y puntos vueltos guiones bajos. Una gramática hecha para
otro tree-sitter se rechaza por su número de ABI en lugar de confiar en
ella y romper, y un nombre que la compilación ya conoce queda
reemplazado por el configurado: así se arregla una gramática vieja sin
esperar una versión nueva.

Para construir una, desde el repositorio de la gramática:

```bash
cc -O2 -fPIC -shared -I src -o libtree-sitter-NOMBRE.dylib src/parser.c src/scanner.c
```

(`.so` en Linux, y sin `src/scanner.c` cuando la gramática no lo trae.)
Una entrada que no se puede cargar cuesta ese lenguaje y nada más: el
editor dice qué pasó y sigue.

## El idioma de la interfaz

La interfaz habla inglés, español o francés; **Interface language** en
Preferencias elige uno, y *System* sigue al de la máquina. El cambio se
aplica en el siguiente arranque.

Los catálogos son gettext y viven en el núcleo, así que los dos shells
y el núcleo dicen lo mismo con las mismas palabras. La fuente es
`core/textchum-core/i18n/<idioma>.po` —el formato que hablan quienes
traducen y sus herramientas— y la compilación produce el `.mo` que lee
el editor. Una frase sin traducir se lee como lo que dice en inglés en
lugar de como una clave que falta.

Un catálogo propio va en el perfil y se lee en lugar del incluido:

```bash
msgfmt -o ~/.config/textchum/translations/es.mo mi-es.po
```

- `~/Library/Application Support/Textchum/translations/<idioma>.mo`
- `~/.config/textchum/translations/<idioma>.mo`

Un archivo con el nombre de un idioma que la compilación no trae se lee
igual, así que un cuarto idioma es un catálogo y no una versión nueva.

Para trabajar las traducciones del repositorio, `scripts/i18n.sh`
extrae las cadenas y las mezcla en cada catálogo como manda gettext:
`xgettext` las encuentra, `msgmerge` conserva lo traducido y marca como
dudoso lo que cambió en inglés, y `msgfmt` comprueba el resultado.
`make check` ejecuta `scripts/i18n.sh --check`, así que una cadena
nueva sin traducir rompe la compilación en vez de salir en inglés.

## Registros de proyecto

Un archivo recuerda cómo está dividido, dónde miraba cada vista, qué
está plegado y qué se le dijo que era cuando su nombre no lo dice. Eso
son datos del archivo, así que viven con el proyecto en lugar de en
`config.json`: un registro por raíz de proyecto, JSON como todo lo
demás.

```json
{
  "version": 1,
  "root": "/work/engine",
  "files": {
    "src/parser.rs": {
      "views": 2,
      "dividers": [0.45],
      "folds": [[12, 48]],
      "language": "rust",
      "places": [{"caret": 812, "scroll": 240.0}]
    }
  }
}
```

Los registros se guardan en el perfil, junto a la sesión y los temas,
así que una ejecución apuntada a un perfil de prueba escribe los suyos.
**Keep each project's state with the checkout** pone el registro en
`<root>/.tchum`, para una disposición que viaje con el clon; la opción
es global, porque una respuesta por proyecto tendría que anotarse
centralmente para poder encontrarse.

La limpieza corre al arrancar en un hilo propio: olvida los registros
de proyectos que ya no están y los que no se escriben desde hace más
que la ventana (90 días por defecto; cero los conserva hasta que se
borren a mano). **Forget records at launch** la apaga, y **Manage…**
junto a la carpeta de registros lista lo que hay, de qué trata cada
uno y cuándo se escribió, para olvidarlos de a uno o de una pasada.

## Aún no está

- Nada por el momento — apunta la próxima molestia cuando aparezca.
