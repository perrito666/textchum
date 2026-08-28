# Un recorrido

Cada pantalla que tiene Textchum, en el orden en que las irías
encontrando, en los dos entornos: la app de macOS a la izquierda y la
de GTK a la derecha. Las capturas vienen de un proyecto ficticio
pequeño — *Harbor*, un gestor de puertos que existe solo para que
estas imágenes tengan algo honesto que mostrar — y cada una sigue tu
ajuste de tema claro u oscuro.

Los dos entornos son el mismo editor sobre el mismo núcleo, así que
las imágenes se diferencian sobre todo en lo que aporta la
plataforma: el marco de la ventana, dónde se dibuja un panel y qué
tipografía entrega el sistema.

## La ventana

Un documento por ventana, pestañas por defecto, y un cajón navegador
con dos mitades: arriba los búferes abiertos agrupados por proyecto,
abajo el árbol de archivos de ese proyecto.

<div class="shots" markdown>
<figure markdown>
[![La ventana del editor: barra lateral con búferes abiertos y el árbol del proyecto, un archivo Rust con coloreado y números de línea (macOS)](images/editor.png#only-light)](images/editor.png)
[![La ventana del editor: barra lateral con búferes abiertos y el árbol del proyecto, un archivo Rust con coloreado y números de línea (macOS)](images/editor-dark.png#only-dark)](images/editor-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![La ventana del editor: barra lateral con búferes abiertos y el árbol del proyecto, un archivo Rust con coloreado y números de línea (Linux)](images/editor-gtk.png#only-light)](images/editor-gtk.png)
[![La ventana del editor: barra lateral con búferes abiertos y el árbol del proyecto, un archivo Rust con coloreado y números de línea (Linux)](images/editor-gtk-dark.png#only-dark)](images/editor-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

La barra de título lleva los datos del documento — codificación,
tamaño, lenguaje y el número de problemas en cuanto el servidor de
lenguaje opina. El árbol acompaña: cambiar de pestaña despliega la
ruta del archivo actual y lo resalta.

## Servidores de lenguaje

Los diagnósticos llegan como marcas tintadas en el texto y un contador
en la barra de título. Nada del editor espera al servidor: se conecta
cuando puede, y lo dice cuando no.

<div class="shots" markdown>
<figure markdown>
[![Un aviso del servidor de lenguaje marcado en el texto y contado en la barra de título (macOS)](images/diagnostics.png#only-light)](images/diagnostics.png)
[![Un aviso del servidor de lenguaje marcado en el texto y contado en la barra de título (macOS)](images/diagnostics-dark.png#only-dark)](images/diagnostics-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![Un aviso del servidor de lenguaje marcado en el texto y contado en la barra de título (Linux)](images/diagnostics-gtk.png#only-light)](images/diagnostics-gtk.png)
[![Un aviso del servidor de lenguaje marcado en el texto y contado en la barra de título (Linux)](images/diagnostics-gtk-dark.png#only-dark)](images/diagnostics-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

Dejar el puntero sobre un símbolo muestra la documentación del
servidor, con el Markdown que envía ya renderizado — bloques de código
en monoespaciada, énfasis con estilo. ⌃⌘H la pide para el símbolo bajo
el cursor, lo que funciona incluso con el hover del ratón apagado.

<div class="shots" markdown>
<figure markdown>
[![Documentación al pasar sobre una función, con firma y prosa renderizadas (macOS)](images/hover.png#only-light)](images/hover.png)
[![Documentación al pasar sobre una función, con firma y prosa renderizadas (macOS)](images/hover-dark.png#only-dark)](images/hover-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![Documentación al pasar sobre una función, con firma y prosa renderizadas (Linux)](images/hover-gtk.png#only-light)](images/hover-gtk.png)
[![Documentación al pasar sobre una función, con firma y prosa renderizadas (Linux)](images/hover-gtk-dark.png#only-dark)](images/hover-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

**Ir a la línea** (⌘L, Ctrl+L en Linux) acepta un número, o el
`src/main.rs:412:8` entero pegado desde un registro de compilación —
el nombre del archivo y el ruido sobrante se ignoran, la línea queda
centrada, y Atrás vuelve a donde se interrumpió la lectura.

Una **barra de cambios** baja por la izquierda del margen y dice qué
líneas difieren del archivo tal como está en git: una franja verde para
una línea nueva, azul para una que dice otra cosa, y una cuña roja en
el límite donde se borraron líneas —las líneas borradas no ocupan alto,
así que una franja no tendría nada que cubrir—. Sigue al búfer y no al
archivo en disco, así que acierta antes de guardar, y se recalcula
cuando la escritura se detiene. Un archivo sin versión confirmada, o
fuera de un repositorio, no lleva marcas en vez de dar todas sus líneas
por nuevas.

**Autoría de la línea** (⌃⌘B, Ctrl+Alt+B en Linux) le pregunta a git
quién tocó por última vez la línea bajo el cursor: el commit, el autor
y cuándo la escribió, el asunto y el cuerpo del mensaje —donde suele
estar el porqué— y el nombre que tenía el archivo entonces si se ha
renombrado desde. El commit queda a un botón del portapapeles, que es
para lo que sirve la respuesta. Una línea escrita desde el último
commit lo dice, en vez de tomar prestado un autor ajeno.

Pregunta con el texto del búfer y no con el archivo en disco, así que
una edición sin guardar por encima del cursor no puede desplazar la
respuesta a la línea vecina.

El autocompletado aparece al escribir tras caracteres de identificador
y `.`; ↑/↓ eligen, ⏎ o ⇥ aceptan, ⎋ descarta. Un snippet llega con su
primer marcador seleccionado, así que escribir lo reemplaza; ⇥ pasa al
siguiente marcador y ⇧⇥ al anterior, y uno escrito dos veces se copia
mientras se teclea. El último ⇥ deja el cursor donde el snippet lo
pidió y devuelve las teclas.

<div class="shots" markdown>
<figure markdown>
[![El desplegable de autocompletado listando miembros con sus tipos (macOS)](images/completion.png#only-light)](images/completion.png)
[![El desplegable de autocompletado listando miembros con sus tipos (macOS)](images/completion-dark.png#only-dark)](images/completion-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![El desplegable de autocompletado listando miembros con sus tipos (Linux)](images/completion-gtk.png#only-light)](images/completion-gtk.png)
[![El desplegable de autocompletado listando miembros con sus tipos (Linux)](images/completion-gtk-dark.png#only-dark)](images/completion-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

**⇧⌘O** lista los símbolos del archivo, filtrables desde el teclado.

<div class="shots" markdown>
<figure markdown>
[![El panel de esquema del documento, con una estructura y sus métodos (macOS)](images/outline.png#only-light)](images/outline.png)
[![El panel de esquema del documento, con una estructura y sus métodos (macOS)](images/outline-dark.png#only-dark)](images/outline-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![El panel de esquema del documento, con una estructura y sus métodos (Linux)](images/outline-gtk.png#only-light)](images/outline-gtk.png)
[![El panel de esquema del documento, con una estructura y sus métodos (Linux)](images/outline-gtk-dark.png#only-dark)](images/outline-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

**Vista ▸ Estado de servidores** responde a «¿está vivo mi servidor?» —
qué corre y dónde, y las transiciones recientes de la sesión,
refrescado en vivo.

<div class="shots" markdown>
<figure markdown>
[![El panel de estado de servidores con una instancia en ejecución y sus transiciones (macOS)](images/server-status.png#only-light)](images/server-status.png)
[![El panel de estado de servidores con una instancia en ejecución y sus transiciones (macOS)](images/server-status-dark.png#only-dark)](images/server-status-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![El panel de estado de servidores con una instancia en ejecución y sus transiciones (Linux)](images/server-status-gtk.png#only-light)](images/server-status-gtk.png)
[![El panel de estado de servidores con una instancia en ejecución y sus transiciones (Linux)](images/server-status-gtk-dark.png#only-dark)](images/server-status-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

## Encontrar cosas

**⌘T** abre archivos por nombre difuso dentro del proyecto. El ámbito
se recorre una vez y se filtra en memoria, así que escribir es
instantáneo; la línea de estado dice cuántos de cuántos archivos
coinciden y qué hace cada tecla — **⏎ busca, ⌘⏎ abre**, para que
afinar una consulta nunca abra un archivo por accidente.

<div class="shots" markdown>
<figure markdown>
[![Abrir rápidamente: una consulta difusa, una ruta coincidente y la línea de estado con las teclas (macOS)](images/open-quickly.png#only-light)](images/open-quickly.png)
[![Abrir rápidamente: una consulta difusa, una ruta coincidente y la línea de estado con las teclas (macOS)](images/open-quickly-dark.png#only-dark)](images/open-quickly-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![Abrir rápidamente: una consulta difusa, una ruta coincidente y la línea de estado con las teclas (Linux)](images/open-quickly-gtk.png#only-light)](images/open-quickly-gtk.png)
[![Abrir rápidamente: una consulta difusa, una ruta coincidente y la línea de estado con las teclas (Linux)](images/open-quickly-gtk-dark.png#only-dark)](images/open-quickly-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

**⇧⌘F** busca en el contenido con una expresión regular, con filtros
apilados que refinan por texto de línea o por ruta. La línea de estado
siempre dice qué hizo la búsqueda.

<div class="shots" markdown>
<figure markdown>
[![Buscar en el proyecto: resultados de regex con un filtro de archivo aplicado (macOS)](images/find-in-project.png#only-light)](images/find-in-project.png)
[![Buscar en el proyecto: resultados de regex con un filtro de archivo aplicado (macOS)](images/find-in-project-dark.png#only-dark)](images/find-in-project-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![Buscar en el proyecto: resultados de regex con un filtro de archivo aplicado (Linux)](images/find-in-project-gtk.png#only-light)](images/find-in-project-gtk.png)
[![Buscar en el proyecto: resultados de regex con un filtro de archivo aplicado (Linux)](images/find-in-project-gtk-dark.png#only-dark)](images/find-in-project-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

**⇧⌘P** es la paleta de comandos: cada acción de menú, buscable de
forma difusa, con su atajo al lado.

<div class="shots" markdown>
<figure markdown>
[![La paleta de comandos listando acciones de menú y sus atajos (macOS)](images/palette.png#only-light)](images/palette.png)
[![La paleta de comandos listando acciones de menú y sus atajos (macOS)](images/palette-dark.png#only-dark)](images/palette-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![La paleta de comandos listando acciones de menú y sus atajos (Linux)](images/palette-gtk.png#only-light)](images/palette-gtk.png)
[![La paleta de comandos listando acciones de menú y sus atajos (Linux)](images/palette-gtk-dark.png#only-dark)](images/palette-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

## Markdown y prosa

Los documentos Markdown abren con una vista previa viva junto al
texto, y el corrector ortográfico de prosa — apagado hasta que elijas
un diccionario — marca las faltas en púrpura, distinto de los
diagnósticos. En código solo mira los comentarios; los identificadores
nunca se marcan.

<div class="shots" markdown>
<figure markdown>
[![Un documento Markdown con su vista previa al lado (macOS)](images/preview.png#only-light)](images/preview.png)
[![Un documento Markdown con su vista previa al lado (macOS)](images/preview-dark.png#only-dark)](images/preview-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![Un documento Markdown con su vista previa al lado (Linux)](images/preview-gtk.png#only-light)](images/preview-gtk.png)
[![Un documento Markdown con su vista previa al lado (Linux)](images/preview-gtk-dark.png#only-dark)](images/preview-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

<div class="shots" markdown>
<figure markdown>
[![Faltas marcadas en la prosa, con la vista previa al lado (macOS)](images/spell-check.png#only-light)](images/spell-check.png)
[![Faltas marcadas en la prosa, con la vista previa al lado (macOS)](images/spell-check-dark.png#only-dark)](images/spell-check-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![Faltas marcadas en la prosa, con la vista previa al lado (Linux)](images/spell-check-gtk.png#only-light)](images/spell-check-gtk.png)
[![Faltas marcadas en la prosa, con la vista previa al lado (Linux)](images/spell-check-gtk-dark.png#only-dark)](images/spell-check-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

## Ajustes

Los ajustes son un archivo JSON plano que la ventana edita; el archivo
es la escotilla de escape, y está vigilado, así que una edición en
otro editor se aplica al instante.

<div class="shots" markdown>
<figure markdown>
[![Ajustes, pestaña General: apariencia, tema, ubicación, tipografía y los interruptores del editor (macOS)](images/settings-general.png#only-light)](images/settings-general.png)
[![Ajustes, pestaña General: apariencia, tema, ubicación, tipografía y los interruptores del editor (macOS)](images/settings-general-dark.png#only-dark)](images/settings-general-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![Ajustes, pestaña General: apariencia, tema, ubicación, tipografía y los interruptores del editor (Linux)](images/settings-general-gtk.png#only-light)](images/settings-general-gtk.png)
[![Ajustes, pestaña General: apariencia, tema, ubicación, tipografía y los interruptores del editor (Linux)](images/settings-general-gtk-dark.png#only-dark)](images/settings-general-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

**Proyectos** decide cómo se encuentran las raíces, qué oculta el árbol
y qué ajustes del editor sobrescribe una raíz.

<div class="shots" markdown>
<figure markdown>
[![Ajustes, pestaña Proyectos: detección, patrones ocultos y sobrescrituras por proyecto (macOS)](images/settings-projects.png#only-light)](images/settings-projects.png)
[![Ajustes, pestaña Proyectos: detección, patrones ocultos y sobrescrituras por proyecto (macOS)](images/settings-projects-dark.png#only-dark)](images/settings-projects-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![Ajustes, pestaña Proyectos: detección, patrones ocultos y sobrescrituras por proyecto (Linux)](images/settings-projects-gtk.png#only-light)](images/settings-projects-gtk.png)
[![Ajustes, pestaña Proyectos: detección, patrones ocultos y sobrescrituras por proyecto (Linux)](images/settings-projects-gtk-dark.png#only-dark)](images/settings-projects-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

Los nombres ocultos son patrones glob, editados uno por línea, con un
menú que añade un preajuste con nombre en un clic.

<div class="shots" markdown>
<figure markdown>
[![El editor de ocultos abierto como globo, un patrón por línea, con el menú de preajustes (macOS)](images/hide-globs.png#only-light)](images/hide-globs.png)
[![El editor de ocultos abierto como globo, un patrón por línea, con el menú de preajustes (macOS)](images/hide-globs-dark.png#only-dark)](images/hide-globs-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![El editor de ocultos abierto como globo, un patrón por línea, con el menú de preajustes (Linux)](images/hide-globs-gtk.png#only-light)](images/hide-globs-gtk.png)
[![El editor de ocultos abierto como globo, un patrón por línea, con el menú de preajustes (Linux)](images/hide-globs-gtk-dark.png#only-dark)](images/hide-globs-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

**Presets** edita esos conjuntos con nombre del mismo modo. Empiezan
como integrados; edita cualquiera y tu lista toma el mando, así que el
que borres sigue borrado hasta que restaures los integrados. Esta
pantalla y la siguiente no tienen imagen del entorno GTK porque allí
no son pantallas: los presets están dentro de Projects, y los
preprocesadores dentro de Language Servers.

![Ajustes, pestaña Presets: conjuntos glob con nombre, cada uno
editable un patrón por línea](images/settings-presets.png#only-light)
![Ajustes, pestaña Presets: conjuntos glob con nombre, cada uno
editable un patrón por línea](images/settings-presets-dark.png#only-dark)

**Servidores de lenguaje** sobrescribe qué comando sirve un lenguaje,
para todos los proyectos o para una raíz.

<div class="shots" markdown>
<figure markdown>
[![Ajustes, pestaña Servidores de lenguaje: comandos por defecto y por proyecto (macOS)](images/settings-servers.png#only-light)](images/settings-servers.png)
[![Ajustes, pestaña Servidores de lenguaje: comandos por defecto y por proyecto (macOS)](images/settings-servers-dark.png#only-dark)](images/settings-servers-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![Ajustes, pestaña Servidores de lenguaje: comandos por defecto y por proyecto (Linux)](images/settings-servers-gtk.png#only-light)](images/settings-servers-gtk.png)
[![Ajustes, pestaña Servidores de lenguaje: comandos por defecto y por proyecto (Linux)](images/settings-servers-gtk-dark.png#only-dark)](images/settings-servers-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

**Preprocesadores** ejecuta formateadores antes de cada guardado: un
comando por línea, cada uno leyendo el documento por la entrada
estándar y devolviéndolo por la salida estándar.

![Ajustes, pestaña Preprocesadores: cadenas de comandos por
lenguaje](images/settings-preprocessors.png#only-light)
![Ajustes, pestaña Preprocesadores: cadenas de comandos por
lenguaje](images/settings-preprocessors-dark.png#only-dark)

## Cosas pequeñas

**⇧⌘N** empieza un documento nuevo en el lenguaje que elijas, filtrado
desde el teclado, para que el coloreado funcione antes del primer
guardado.

<div class="shots" markdown>
<figure markdown>
[![El selector Nuevo con formato, filtrando la lista de lenguajes (macOS)](images/new-with-format.png#only-light)](images/new-with-format.png)
[![El selector Nuevo con formato, filtrando la lista de lenguajes (macOS)](images/new-with-format-dark.png#only-dark)](images/new-with-format-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![El selector Nuevo con formato, filtrando la lista de lenguajes (Linux)](images/new-with-format-gtk.png#only-light)](images/new-with-format-gtk.png)
[![El selector Nuevo con formato, filtrando la lista de lenguajes (Linux)](images/new-with-format-gtk-dark.png#only-dark)](images/new-with-format-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>

Y el panel Acerca de dice qué compilación estás usando — una versión
real, incluso para una compilación local.

<div class="shots" markdown>
<figure markdown>
[![El panel Acerca de con la versión, el autor, el repositorio y la licencia (macOS)](images/about.png#only-light)](images/about.png)
[![El panel Acerca de con la versión, el autor, el repositorio y la licencia (macOS)](images/about-dark.png#only-dark)](images/about-dark.png)
<figcaption>macOS</figcaption>
</figure>
<figure markdown>
[![El panel Acerca de con la versión, el autor, el repositorio y la licencia (Linux)](images/about-gtk.png#only-light)](images/about-gtk.png)
[![El panel Acerca de con la versión, el autor, el repositorio y la licencia (Linux)](images/about-gtk-dark.png#only-dark)](images/about-gtk-dark.png)
<figcaption>Linux</figcaption>
</figure>
</div>
