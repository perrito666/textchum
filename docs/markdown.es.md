# Markdown

Markdown es ciudadano de primera: el mismo archivo recibe
[coloreado](highlighting.md) tree-sitter en el editor — incluidos los
bloques de código cercados coloreados en su propio lenguaje — y una
**vista previa en vivo** a su lado.

## La vista previa

Abrir un documento Markdown abre automáticamente el panel de vista
previa, a la derecha del código fuente. **View → Toggle Markdown
Preview** (⌥⌘P) lo oculta y lo muestra.

- La vista previa se actualiza mientras se escribe, parcheada en su
  sitio — sin recargas, sin parpadeo, sin perder la posición de
  desplazamiento.
- Desplazar cualquiera de los dos paneles arrastra al otro.
- Un enlace se abre en el navegador al pulsarlo; el panel se queda en
  el documento. Un enlace al propio documento desplaza la vista previa
  hasta él.
- Pulsar la vista previa con el botón derecho ofrece **Guardar como
  PDF…** — la página renderizada, escrita directa a un archivo, sin
  herramientas externas.
- El renderizado admite CommonMark más tablas, tachado, listas de tareas
  y notas al pie, y sus estilos siguen la apariencia del sistema (o la
  configurada).

El renderizado ocurre en el núcleo (la carcasa solo posee el panel), así
que el mismo HTML alimentará más adelante otras salidas.

## Hugo

Las entradas escritas para [Hugo](https://gohugo.io) son Markdown con
dos añadidos, y Textchum lee ambos sin necesidad de tener Hugo
instalado.

El **front matter** — TOML entre `+++`, YAML entre `---` — se colorea
como el lenguaje que realmente es, se mantiene fuera de la prosa que
lee el corrector (un slug no es una falta) y se muestra en la vista
previa como un pequeño bloque de metadatos en vez de un párrafo de
signos.

Los **shortcodes** — `{{< figure src="…" >}}` y
`{{% notice %}}…{{% /notice %}}` — se colorean como las llamadas que
son, el corrector los salta, y la vista previa los muestra como un
marcador con su nombre. Nunca se ejecutan: hacerlo requeriría el motor
de plantillas de Hugo y los layouts de tu sitio, así que un marcador
es lo honesto. El cuerpo de un `{{% … %}}` emparejado se sigue
renderizando como Markdown, igual que hace Hugo.

El **esquema** (⇧⌘O) lista los encabezados de una entrada aunque no
haya servidor de lenguaje, anidados por profundidad. Los encabezados
dentro de bloques de código o del front matter no se confunden con
estructura.

Por último, los archivos bajo un directorio `layouts/` se tratan como
**plantillas de Go** en vez de HTML plano: el marcado se colorea como
HTML y las acciones `{{ … }}` destacan sobre él.

El front matter en JSON (la forma con llaves) todavía no se reconoce;
TOML y YAML cubren lo que Hugo escribe por defecto.

## Aún no está

- Sincronización de desplazamiento precisa mediante anclas de origen (la
  de hoy es proporcional, y deriva en documentos con bloques de alturas
  muy desiguales).
- Colores de sintaxis dentro de los bloques de código de la vista previa
  (el editor los colorea; la vista previa los muestra planos).
- El modo de edición híbrido/WYSIWYG — el panel de vista previa es
  deliberadamente el primero de los tres niveles de Markdown.
