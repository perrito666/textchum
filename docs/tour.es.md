# Un recorrido por la app de macOS

Cada pantalla que tiene Textchum, en el orden en que las irías
encontrando. Las capturas vienen de un proyecto ficticio pequeño —
*Harbor*, un gestor de puertos que existe solo para que estas imágenes
tengan algo honesto que mostrar.

## La ventana

Un documento por ventana, pestañas por defecto, y un cajón navegador
con dos mitades: arriba los búferes abiertos agrupados por proyecto,
abajo el árbol de archivos de ese proyecto.

![La ventana del editor: barra lateral con búferes abiertos y el árbol
del proyecto, un archivo Rust con coloreado y números de
línea](images/editor.png)

La barra de título lleva los datos del documento — codificación,
tamaño, lenguaje y el número de problemas en cuanto el servidor de
lenguaje opina. El árbol acompaña: cambiar de pestaña despliega la
ruta del archivo actual y lo resalta.

## Servidores de lenguaje

Los diagnósticos llegan como marcas tintadas en el texto y un contador
en la barra de título. Nada del editor espera al servidor: se conecta
cuando puede, y lo dice cuando no.

![Un aviso del servidor de lenguaje marcado en el texto y contado en la
barra de título](images/diagnostics.png)

Dejar el puntero sobre un símbolo muestra la documentación del
servidor, con el Markdown que envía ya renderizado — bloques de código
en monoespaciada, énfasis con estilo. ⌃⌘H la pide para el símbolo bajo
el cursor, lo que funciona incluso con el hover del ratón apagado.

![Documentación al pasar sobre una función, con firma y prosa
renderizadas](images/hover.png)

El autocompletado aparece al escribir tras caracteres de identificador
y `.`; ↑/↓ eligen, ⏎ o ⇥ aceptan, ⎋ descarta. Un snippet llega con su
primer marcador seleccionado, así que escribir lo reemplaza.

![El desplegable de autocompletado listando miembros con sus
tipos](images/completion.png)

**⇧⌘O** lista los símbolos del archivo, filtrables desde el teclado.

![El panel de esquema del documento, con una estructura y sus
métodos](images/outline.png)

**Vista ▸ Estado de servidores** responde a «¿está vivo mi servidor?» —
qué corre y dónde, y las transiciones recientes de la sesión,
refrescado en vivo.

![El panel de estado de servidores con una instancia en ejecución y sus
transiciones](images/server-status.png)

## Encontrar cosas

**⌘T** abre archivos por nombre difuso dentro del proyecto. El ámbito
se recorre una vez y se filtra en memoria, así que escribir es
instantáneo; la línea de estado dice cuántos de cuántos archivos
coinciden y qué hace cada tecla — **⏎ busca, ⌘⏎ abre**, para que
afinar una consulta nunca abra un archivo por accidente.

![Abrir rápidamente: una consulta difusa, una ruta coincidente y la
línea de estado con las teclas](images/open-quickly.png)

**⇧⌘F** busca en el contenido con una expresión regular, con filtros
apilados que refinan por texto de línea o por ruta. La línea de estado
siempre dice qué hizo la búsqueda.

![Buscar en el proyecto: resultados de regex con un filtro de archivo
aplicado](images/find-in-project.png)

**⇧⌘P** es la paleta de comandos: cada acción de menú, buscable de
forma difusa, con su atajo al lado.

![La paleta de comandos listando acciones de menú y sus
atajos](images/palette.png)

## Markdown y prosa

Los documentos Markdown abren con una vista previa viva junto al
texto, y el corrector ortográfico de prosa — apagado hasta que elijas
un diccionario — marca las faltas en púrpura, distinto de los
diagnósticos. En código solo mira los comentarios; los identificadores
nunca se marcan.

![Un documento Markdown con su vista previa al
lado](images/preview.png)

![Faltas marcadas en la prosa, con la vista previa
al lado](images/spell-check.png)

## Ajustes

Los ajustes son un archivo JSON plano que la ventana edita; el archivo
es la escotilla de escape, y está vigilado, así que una edición en
otro editor se aplica al instante.

![Ajustes, pestaña General: apariencia, tema, ubicación, tipografía y
los interruptores del editor](images/settings-general.png)

**Proyectos** decide cómo se encuentran las raíces, qué oculta el árbol
y qué ajustes del editor sobrescribe una raíz.

![Ajustes, pestaña Proyectos: detección, patrones ocultos y
sobrescrituras por proyecto](images/settings-projects.png)

Los nombres ocultos son patrones glob, editados uno por línea, con un
menú que añade un preajuste con nombre en un clic.

![El editor de ocultos abierto como globo, un patrón por línea, con el
menú de preajustes](images/hide-globs.png)

**Presets** edita esos conjuntos con nombre del mismo modo. Empiezan
como integrados; edita cualquiera y tu lista toma el mando, así que el
que borres sigue borrado hasta que restaures los integrados.

![Ajustes, pestaña Presets: conjuntos glob con nombre, cada uno
editable un patrón por línea](images/settings-presets.png)

**Servidores de lenguaje** sobrescribe qué comando sirve un lenguaje,
para todos los proyectos o para una raíz.

![Ajustes, pestaña Servidores de lenguaje: comandos por defecto y por
proyecto](images/settings-servers.png)

**Preprocesadores** ejecuta formateadores antes de cada guardado: un
comando por línea, cada uno leyendo el documento por la entrada
estándar y devolviéndolo por la salida estándar.

![Ajustes, pestaña Preprocesadores: cadenas de comandos por
lenguaje](images/settings-preprocessors.png)

## Cosas pequeñas

**⇧⌘N** empieza un documento nuevo en el lenguaje que elijas, filtrado
desde el teclado, para que el coloreado funcione antes del primer
guardado.

![El selector Nuevo con formato, filtrando la lista de
lenguajes](images/new-with-format.png)

Y el panel Acerca de dice qué compilación estás usando — una versión
real, incluso para una compilación local.

![El panel Acerca de con la versión, el autor, el repositorio y la
licencia](images/about.png)
