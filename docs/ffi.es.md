# La frontera C

La carcasa y el núcleo se encuentran en una única cabecera C, `textchum.h`.
Se genera a partir del código Rust con
[cbindgen](https://github.com/mozilla/cbindgen) en cada compilación del
núcleo, y se versiona en el repositorio para que las herramientas del lado
Swift funcionen sin una cadena de herramientas de Rust. La CI falla si una
compilación deja la cabecera desactualizada, así que nunca puede divergir
del código.

## Convenciones

Cada función de la interfaz sigue el mismo pequeño conjunto de reglas.

**Manejadores opacos.** Los tipos del núcleo (`TcApp`, `TcBuffer`) son
estructuras opacas; quien llama retiene punteros, los devuelve en cada
llamada y los libera con la función `tc_*_free` correspondiente. Quien llama
nunca reserva memoria en nombre del núcleo.

**UTF-8 de entrada, longitudes explícitas.** Las cadenas que *entran* al
núcleo son pares `(puntero, longitud)` de bytes UTF-8 — sin terminadores
nulos obligatorios, sin más codificación que UTF-8. Las cadenas que el
núcleo *devuelve* son UTF-8 terminadas en nulo y de su propiedad; se liberan
con `tc_string_free`.

**Dos unidades de posición.** Las funciones direccionan el texto en
desplazamientos de bytes UTF-8 (la unidad nativa del núcleo) o en unidades
UTF-16 (sufijo `_utf16`), porque es lo que cuentan `NSRange` y el Language
Server Protocol. El núcleo hace la conversión; quien llama usa la unidad que
tenga de forma natural.

**Fallo transaccional.** Las llamadas falibles devuelven `bool`. `false`
significa que la entrada fue validada, rechazada y **nada cambió** — un
desplazamiento fuera de rango, una posición en mitad de un carácter, UTF-8
inválido. Quien llama siempre puede tratar el fallo como «la operación no
ocurrió».

**Los pánicos no cruzan.** Cada punto de entrada captura los pánicos de Rust
y los convierte en el valor de fallo de la función. Un error del núcleo no
puede desenrollarse hacia marcos de pila de Swift.

**Un hilo de entrada, un hilo de salida.** Las llamadas al núcleo deben
venir de un solo hilo. Los eventos fluyen en sentido contrario por el
callback registrado con `tc_app_new`, invocado en un único hilo de despacho
propiedad del núcleo. La tarea del callback es trasladar el evento al hilo
de interfaz de la carcasa; `TextchumKit` hace exactamente eso y nada más.

## El canal de eventos

Cierta información se origina dentro del núcleo (hoy: respuestas *pong*
usadas para verificar el canal; próximamente: diagnósticos de servidores de
lenguaje, invalidaciones de coloreado). Llega a la carcasa como un
`TcEvent` — un discriminante `kind` más la carga del evento — entregado al
callback registrado.

Las carcasas deben tolerar valores de `kind` desconocidos: que un núcleo más
nuevo emita un evento que una carcasa más vieja no entiende es
compatibilidad hacia adelante, no un error.

`tc_app_free` bloquea hasta que los eventos en cola se hayan entregado y
garantiza que el callback no se invoca nunca después, que es lo que hace que
el desmontaje sea seguro de escribir del lado de la carcasa.

## Superficie actual

| Función | Propósito |
|---|---|
| `tc_version` | Versión del núcleo como cadena estática. |
| `tc_app_new` / `tc_app_free` | Crear/destruir una instancia del núcleo y su canal de eventos. |
| `tc_app_ping` | Pedir un *pong* asíncrono; ejercita la ruta de eventos. |
| `tc_buffer_new` / `tc_buffer_free` | Crear/destruir un búfer de texto. |
| `tc_buffer_insert` | Insertar UTF-8 en un desplazamiento de bytes. |
| `tc_buffer_delete` | Borrar un rango de bytes. |
| `tc_buffer_replace_utf16` | Reemplazar un rango de unidades UTF-16 — la forma de una edición de AppKit. |
| `tc_buffer_text` | Copiar el contenido completo. |
| `tc_buffer_len_bytes` / `tc_buffer_len_utf16` | Longitudes en ambas unidades. |
| `tc_document_new` / `tc_document_open` / `tc_document_free` | Crear un documento (vacío o desde un archivo) y destruirlo. |
| `tc_document_replace_utf16` | Editar un documento registrando el historial de deshacer. |
| `tc_document_undo` / `tc_document_redo` | Recorrer el historial; un parámetro de salida informa de la edición que la carcasa debe reproducir. |
| `tc_document_break_undo_group` | Terminar la racha actual de fusión de deshacer. |
| `tc_document_save` / `tc_document_save_as` | Guardados atómicos; los fallos rellenan un parámetro de salida opcional con un mensaje. |
| `tc_document_text` / `tc_document_len_bytes` / `tc_document_len_utf16` | Contenido y longitudes. |
| `tc_document_is_dirty` / `tc_document_can_undo` / `tc_document_can_redo` | Consultas de estado. |
| `tc_document_path` / `tc_document_encoding_name` | Identidad del archivo y codificación. |
| `tc_string_free` | Liberar una cadena devuelta por el núcleo. |

Las operaciones de archivo falibles siguen una convención más: devuelven su
valor de fallo y, cuando quien llama pasó un parámetro de salida no nulo,
almacenan allí un mensaje UTF-8 legible (que se libera con
`tc_string_free`) — la carcasa lo muestra tal cual en la alerta.

La superficie es deliberadamente pequeña y crece solo cuando una
característica de la carcasa lo necesita. Los datos masivos (futuro: tramos
de coloreado, diagnósticos) cruzarán como estructuras compactas o cargas
serializadas, no como una llamada por elemento.

## Cómo lo consume Swift

El objetivo `CTextchum` envuelve la cabecera como módulo de Clang, de modo
que Swift la importa como cualquier biblioteca. `TextchumKit` traduce después
las llamadas crudas a Swift idiomático — clases con propiedad basada en
`deinit`, parámetros `NSRange`, errores lanzados para operaciones rechazadas
y eventos tipados entregados en el actor principal. El código de la
aplicación nunca toca un puntero.
