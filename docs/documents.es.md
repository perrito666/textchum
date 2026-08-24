# Documentos

Un *búfer* es texto crudo; un *documento* es un búfer más todo lo que lo
convierte en un archivo: un historial de deshacer, un indicador de cambios
sin guardar, una ruta y una codificación. Las ventanas del editor trabajan
siempre con documentos.

## Escritura

Retorno aplica sangría automática: la línea nueva hereda el espacio en
blanco inicial de la actual, y baja un nivel más cuando la línea termina
(antes del cursor) con un abridor — `{`, `[`, `(` o `:`. El nivel extra
habla el dialecto del propio documento: tabuladores en un archivo
sangrado con tabuladores, espacios al ancho de tabulación configurado en
caso contrario. Una línea sin nada que heredar recibe un salto de línea
normal, así que la función es invisible hasta que ayuda.

## Deshacer y rehacer

El historial de deshacer vive en el núcleo, no en el `NSUndoManager` de
AppKit. Cada edición se registra como una operación invertible; deshacer
extrae el registro más reciente, aplica su inversa e informa del cambio
resultante a la ventana, que lo reproduce en pantalla. Como el historial
está detrás de la misma interfaz que todo lo demás, no puede perderse
ninguna edición: no existe un segundo camino hacia el texto.

Los registros se fusionan para que deshacer avance en pasos a escala
humana:

- **Las rachas de tecleo** se fusionan: inserciones consecutivas, cada una
  comenzando exactamente donde terminó la anterior, se convierten en un solo
  paso de deshacer.
- **Las rachas de borrado** se fusionan igual, tanto con retroceso como con
  suprimir.
- Un **salto de línea** termina la racha por cualquiera de sus lados, así
  que deshacer se detiene con granularidad de línea.
- **Mover el cursor** (clic, flechas) termina la racha actual; la siguiente
  tecla inicia un paso nuevo.

Las operaciones compuestas se registran como grupos explícitos: Reemplazar
todo reescribe cada coincidencia pero se deshace como un solo paso, y una
recarga desde disco (más abajo) también es un solo paso.

## Buscar y reemplazar

**⌘F** abre la barra de búsqueda nativa (**⌥⌘F** con el campo de
reemplazo, **⌘G** / **⇧⌘G** coincidencia siguiente y anterior, **⌘E** busca
la selección). El menú de opciones de la barra ofrece coincidencia por
subcadena, palabra completa y **expresión regular**. Los reemplazos son
ediciones normales: pasan por el núcleo, entran en el historial de
deshacer — Reemplazar todo como un solo paso — y marcan el documento como
modificado igual que el tecleo.

## Cambios externos

Textchum vigila el archivo de cada documento abierto. Si otro programa lo
cambia:

- un documento **limpio** sigue al disco en silencio — la ventana muestra
  simplemente el contenido nuevo;
- un documento **modificado** pregunta: conservar los cambios sin guardar o
  recargar desde disco. Recargar descarta el búfer en favor del archivo,
  pero la recarga es en sí misma un paso de deshacer, así que ⌘Z devuelve
  su versión (y vuelve a marcar el documento como modificado, pues entonces
  difiere del disco).

Un archivo que desaparece del disco se deja en paz: el búfer permanece y
guardar recrea el archivo.

**Revertir a lo guardado** (menú File, ⌥⌘R, reasignable como
`revertToSaved`) es la versión manual de la misma recarga: descartar el
búfer y aceptar lo que diga el disco, con una confirmación cuando hay
cambios sin guardar (y un Deshacer para recuperarlos). Existe para el
raro cambio externo que el vigilante no ve — flujos de borrar y
reemplazar como un git checkout.

## Estado de cambios sin guardar

Un documento conoce el punto exacto de su historial en el que se guardó por
última vez, así que *modificado* significa «el estado actual difiere del
guardado» — no «hubo una edición en algún momento». Editar y luego deshacer
hasta el punto de guardado deja un documento limpio, y el botón de cerrar de
la ventana pierde su punto en consecuencia. Si nuevas ediciones vuelven
inalcanzable el estado guardado (se deshizo más allá de él y se escribió
otra cosa), el documento cuenta como modificado hasta el próximo guardado,
como debe ser.

Cerrar una ventana con cambios, o salir con ventanas modificadas abiertas,
hace la pregunta habitual: guardar, no guardar o cancelar.

## Archivos y codificaciones

Textchum descodifica al abrir y recodifica al guardar:

- El **UTF-8** válido se carga como UTF-8. Un BOM inicial se elimina en
  memoria, se recuerda y se vuelve a escribir al guardar.
- Cualquier otra cosa se descodifica como **ISO-8859-1** (Latin-1), que
  asigna un carácter a cada byte y por tanto no puede fallar. Al guardar se
  recodifica a Latin-1; si una edición introdujo caracteres que Latin-1 no
  puede contener, el guardado promociona el archivo a UTF-8 en silencio —
  nada puede perderse en esa dirección — y el subtítulo de la ventana
  refleja la nueva codificación.

Los finales de línea nunca se normalizan: lo que se leyó es lo que se
escribe, sea `\n` o `\r\n`.

La codificación actual está siempre visible en el subtítulo de la ventana,
junto al tamaño del documento.

## Los guardados son atómicos

Un guardado escribe el documento completo en un archivo temporal dentro del
directorio de destino, lo vuelca a disco y lo renombra sobre el destino. Un
fallo a mitad de guardado no puede dejar nunca un archivo truncado, y otros
programas que observen el archivo ven el contenido viejo o el nuevo — nunca
una mezcla.

## Restauración de sesión

Relanzar Textchum reabre los archivos que estaban abiertos, cada uno con
su posición de cursor y de desplazamiento, trayendo al frente el que se
estaba usando. El estado es un archivo JSON plano (`session.json`, junto
a la configuración), escrito continuamente — no solo al salir —, así que
un fallo pierde como mucho un instante de posición, nunca la lista de
archivos. Los archivos que ya no existen se omiten.

Para arrancar sin memoria (útil persiguiendo un error): lanzar con
`--fresh`, mantener ⇧ mientras arranca la aplicación o borrar
`session.json` — cualquiera de las tres es un reinicio completo.

## Aún no está

- Codificaciones más allá de UTF-8 y Latin-1.
