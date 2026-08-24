# Documentos

Un *búfer* es texto crudo; un *documento* es un búfer más todo lo que lo
convierte en un archivo: un historial de deshacer, un indicador de cambios
sin guardar, una ruta y una codificación. Las ventanas del editor trabajan
siempre con documentos.

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

## Aún no está

- Detectar cambios externos en archivos abiertos (si el archivo se edita en
  otro sitio, Textchum todavía no lo nota).
- Codificaciones más allá de UTF-8 y Latin-1.
- Reabrir ventanas y documentos de la sesión anterior.
