# Coloreado de sintaxis

Textchum colorea el código con
[tree-sitter](https://tree-sitter.github.io): cada documento con un
lenguaje reconocido mantiene un árbol de análisis real, actualizado de
forma incremental con cada edición, y el coloreado se calcula a partir de
ese árbol — no con expresiones regulares.

## Lenguajes

La detección es por extensión de archivo — o por nombre exacto, para
los archivos cuya identidad *es* su nombre: `Makefile` (y `*.mk`) y los
mensajes de git (`COMMIT_EDITMSG`, `MERGE_MSG`, `TAG_EDITMSG`), así que
los mensajes de commit escritos con `chum --wait` llegan coloreados.
Se ejecuta al abrir y en el primer guardado de un documento sin título.
Actualmente se reconocen: Rust, Python, Go, C, JavaScript, JSON, Bash,
Make, mensajes de commit de git, HTML, CSS, TOML, YAML, Swift, Zig y
Markdown. El subtítulo de la ventana muestra el lenguaje activo; los
archivos no reconocidos se quedan simplemente en texto plano. Las filas
del navegador llevan una pequeña insignia con el color convencional de
cada lenguaje conocido, para recorrer el árbol de un vistazo.

Las gramáticas van compiladas dentro de la aplicación, así que el
coloreado funciona sin conexión y de forma idéntica en todas partes.

## Inyecciones

Los documentos que incrustan otros lenguajes colorean el contenido
incrustado con la gramática del lenguaje incrustado:

- Los bloques de código cercados de Markdown se colorean con el lenguaje
  nombrado en el cerco (` ```rust ` y compañía), y el énfasis, los enlaces
  y el código en línea del propio Markdown provienen de una gramática
  dedicada de elementos en línea.
- Los elementos `<script>` y `<style>` de HTML se colorean como JavaScript
  y CSS.

## Cómo funciona

El reparto de responsabilidades sigue la regla arquitectónica del
proyecto:

- El **núcleo** es dueño del análisis. Cada edición alimenta el árbol con
  una descripción exacta del cambio y tree-sitter reanaliza de forma
  incremental — trabajo a escala de pulsación sin importar el tamaño del
  archivo. A petición, ejecuta la consulta de coloreado del lenguaje sobre
  un rango y responde con *tramos con estilo*: rangos más índices en una
  tabla de estilos.
- La **carcasa** es dueña de los píxeles. Los tramos se pintan como
  atributos de dibujado de TextKit — una capa solo de color que no puede
  invalidar la composición del texto, así que colorear nunca compite con
  teclear.

La tabla de estilos lleva un color por apariencia del sistema, de modo que
cambiar entre modo claro y oscuro recolorea al instante, con paletas
ajustadas a cada uno.

Los documentos muy grandes (más allá de unos pocos megabytes) omiten el
coloreado deliberadamente; el editor sigue siendo rápido a cualquier
tamaño.

La propia paleta es un tema — se incluyen siete de serie, y los temas de
la persona usuaria son archivos JSON; véanse
[los temas en la configuración](configuration.es.md).

Si un artefacto de coloreado sobreviviera a una edición, **View →
Redraw** (⌥⌘L, reasignable como `redraw`) reconstruye desde cero cada
capa visual: atributos base, colores de sintaxis, marcas de
diagnóstico y el margen.

## Aún no está

- Matices de negrita/cursiva — la capa es solo de color por ahora.
- Selección manual del lenguaje desde la interfaz para archivos con
  extensiones raras.
- Consultas limitadas a la zona visible para documentos de cientos de
  kilobytes (por ahora se colorean enteros o, pasado un tope, no se
  colorean).
