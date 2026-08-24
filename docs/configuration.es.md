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

## Aún no está

- Textchum todavía no vigila el archivo mientras se ejecuta; los cambios
  hechos en otro editor se aplican en el siguiente arranque.
- Ajustes por proyecto.
