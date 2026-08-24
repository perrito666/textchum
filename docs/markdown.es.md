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
- El renderizado admite CommonMark más tablas, tachado, listas de tareas
  y notas al pie, y sus estilos siguen la apariencia del sistema (o la
  configurada).

El renderizado ocurre en el núcleo (la carcasa solo posee el panel), así
que el mismo HTML alimentará más adelante otras salidas.

## Aún no está

- Sincronización de desplazamiento precisa mediante anclas de origen (la
  de hoy es proporcional, y deriva en documentos con bloques de alturas
  muy desiguales).
- Colores de sintaxis dentro de los bloques de código de la vista previa
  (el editor los colorea; la vista previa los muestra planos).
- El modo de edición híbrido/WYSIWYG — el panel de vista previa es
  deliberadamente el primero de los tres niveles de Markdown.
