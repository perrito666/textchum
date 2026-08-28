# Servidores de lenguaje

Textchum valida el código mediante el
[Language Server Protocol](https://microsoft.github.io/language-server-protocol/),
con un comportamiento definitorio: **una instancia de servidor por
proyecto**.

## Una instancia por proyecto

Los procesos de servidor se identifican por *(servidor, raíz del
proyecto)*, usando la misma noción de proyecto que
[el navegador](navigator.md): el directorio ancestro más cercano con un
marcador de raíz. Abra archivos de dos proyectos Rust distintos y
correrán dos procesos `rust-analyzer` independientes, cada uno
inicializado con su propia raíz, cada uno viendo solo los archivos de su
proyecto. Las fugas entre proyectos — diagnósticos de un espacio de
trabajo colándose en otro, un índice construido sobre todo el directorio
personal — no pueden ocurrir por construcción.

Los archivos fuera de todo proyecto reciben una instancia por directorio,
así que los archivos sueltos tampoco se suman al espacio de trabajo de
nadie.

## Lo que se ve

- Los hallazgos llegan mientras se escribe (enviados en lotes con
  *debounce*) y marcan el texto afectado: rojo para errores, naranja para
  avisos, azul para notas.
- El subtítulo de la ventana los cuenta («2 errors, 1 warning»).
- **Autocompletado al escribir**: las sugerencias aparecen tras
  caracteres de identificador y `.`, filtradas mientras se sigue
  escribiendo — ↑/↓ para elegir, ⏎ o ⇥ para aceptar, ⎋ para descartar,
  ⌃Espacio para pedirlas explícitamente.
- Dejar el ratón sobre un símbolo muestra la documentación **hover** del
  servidor en un globo, con el Markdown que envían los servidores ya
  renderizado — bloques de código en monoespaciada, énfasis y código en
  línea con su estilo. Solo se dispara sobre identificadores (nunca
  sobre espacios ni comentarios), se puede apagar en Vista ▸
  Documentación al pasar (o en Ajustes), y **Mostrar documentación del
  símbolo** (⌃⌘H) la pide para el símbolo bajo el cursor a demanda —
  incluso con el hover del ratón apagado.
- **Saltar a la definición** (⌃⌘J, o ⌘-clic) va al símbolo bajo el
  cursor — entre archivos, abriendo o trayendo al frente el destino
  según haga falta.
- **Buscar referencias** (⇧⌘R) lista cada uso del símbolo bajo el
  cursor en un panel flotante — ↑/↓ para moverse, ⏎ para saltar.
  Primero el código y después las pruebas, cada parte bajo un
  encabezado con su cuenta: qué llama a esto es la pregunta, y qué lo
  comprueba es lo siguiente. Qué archivos son pruebas es una
  convención y no un hecho —un directorio `tests`, un
  `parser_test.go`, un `Button.test.ts`, un `ParserTests.swift`—, así
  que la regla es prudente y `latest.rs` no es una prueba. Un
  `#[cfg(test)] mod tests` de Rust dentro de un archivo corriente
  aparece como código, que es lo que dice su ruta. Si todo cae de un
  lado, no hay encabezados.
- **Formatear documento** (⌥⇧⌘F) pregunta primero al servidor y cae a
  la cadena de preprocesadores de guardado — así el formateo funciona
  en documentos sin título y en lenguajes sin servidor, siempre que
  haya una cadena configurada.
- **Renombrar símbolo…** (⌃⌘R) renombra en todo el espacio de trabajo:
  las ventanas abiertas se editan en el sitio (el deshacer funciona por
  ventana) y los archivos que nadie tiene abiertos se reescriben en
  disco.
- **Formatear documento** (⌥⇧⌘F) reformatea a través del servidor,
  conservando tabuladores si el documento sangra con tabuladores y
  espacios en caso contrario.
- **Esquema del documento** (⇧⌘O) lista los símbolos del archivo — el
  anidamiento se muestra con sangría, filtrable de forma difusa — y ⏎
  salta a la selección.
- Un servidor ausente se informa una sola vez, con el comando que lo
  instala; todo lo demás del editor sigue funcionando sin él.

## Servidores

Textchum encuentra los servidores en el `PATH` — no los instala:

| Lenguaje | Servidor | Instalación |
|---|---|---|
| Rust | rust-analyzer | `rustup component add rust-analyzer` |
| Python | pyright | `npm install -g pyright` |
| Go | gopls | `go install golang.org/x/tools/gopls@latest` |
| C | clangd | Xcode CLT, o `brew install llvm` |
| JavaScript | typescript-language-server | `npm install -g typescript-language-server typescript` |
| Swift | sourcekit-lsp | viene con la cadena de herramientas de Xcode |
| Zig | zls | `brew install zls` |
| Bash | bash-language-server | `npm install -g bash-language-server` |

## Elegir los servidores

Settings → Language Servers permite decidir qué comando sirve a un
lenguaje — para todos los proyectos (un *valor por defecto*) o para una
raíz de proyecto concreta. Las entradas de proyecto ganan a los valores
por defecto; los lenguajes sin entrada usan la tabla anterior. Las
entradas viven en `config.json` bajo `"lsp"`, con las garantías de
edición a mano habituales del archivo:

```json
{
  "lsp": {
    "defaults": {"python": "pylsp"},
    "projects": {"/work/projA": {"python": "pyright-langserver --stdio"}}
  }
}
```

El comando de una entrada se edita en el sitio — corregir una errata o
añadir un `--stdio` que faltaba se hace en la propia fila, con ⏎ o
haciendo clic fuera, sin borrar y volver a crear. Los cambios se
aplican a los servidores arrancados después; el botón **Restart Servers
Now** de la pestaña retira las instancias en marcha y las relanza con
la nueva configuración.

## Cuando no hay servidor

Dos redes de seguridad cubren el caso sin servidor:

- **El respaldo con ctags.** Con **Ctags fallback** activado en
  Ajustes → Projects (como valor por defecto o por proyecto, igual que
  todos los indicadores de proyecto), Ir a la Definición se responde
  desde un índice de [Universal Ctags](https://ctags.io) del proyecto
  siempre que no haya servidor de lenguaje disponible — y también cuando
  un servidor en marcha no tiene respuesta. El índice se construye en el
  primer uso y se refresca mientras se sigue saltando; ctags conoce
  nombres, no semántica, así que es un respaldo, no un reemplazo. Debe
  ser *Universal* Ctags (`brew install universal-ctags`): el `ctags` que
  macOS trae en `/usr/bin` es otro programa, mucho más antiguo, que no
  puede emitir el índice JSON que esto lee. Textchum mira más allá de
  ese para encontrar un Universal Ctags real en el `PATH`.
- **El registro de depuración.** Cada decisión en el camino de «archivo
  abierto» a «servidor en marcha» — la raíz de proyecto resuelta, qué
  servidor se eligió y por qué, los fallos de arranque con el `PATH`
  exacto consultado y cada transición de estado — se añade a:

  ```
  ~/Library/Logs/Textchum/lsp.log
  ```

  La salida de error propia de cada servidor (stderr) también se
  captura ahí, de modo que un servidor que sale durante el arranque
  deja su queja registrada — un comando sin su indicador de transporte
  (el `--stdio` de pyright, por ejemplo) se diagnostica de un vistazo,
  y el registro avisa directamente cuando un comando personalizado
  omite argumentos que el registro incorporado sabe necesarios. Cuando
  un proyecto se queda misteriosamente sin soporte de lenguaje, este
  archivo nombra la pieza que falta.

Una causa clásica merece nota aparte: las aplicaciones lanzadas desde el
Finder heredaban el `PATH` mínimo de macOS, que no contiene ninguno de
los lugares donde realmente viven los servidores de lenguaje (Homebrew,
npm, cargo, go). Textchum ahora adopta al arrancar el `PATH` de la shell
de inicio de sesión — más algunos directorios de herramientas
convencionales — de modo que un servidor que funciona desde la terminal
funciona también desde el Dock.

## Por debajo

El cliente vive en el núcleo, tras la misma frontera que todo lo demás:
JSON-RPC sobre stdio, un apretón de manos de inicialización antes de
cualquier tráfico de documentos y sincronización de documento completo
(la sincronización incremental es una optimización futura). Los mensajes
del servidor se procesan fuera del hilo de interfaz y llegan a ella por
el canal único de eventos del núcleo; un proceso de servidor colgado
recibe un período de gracia acotado al cerrar y después se mata, así que
salir de Textchum nunca puede quedarse colgado por un servidor que se
porta mal. Toda la ruta del protocolo se ejercita en la CI contra un
servidor guionizado.

Las instancias también se cuidan solas: un servidor que **cae** a mitad
de sesión se reinicia automáticamente con retroceso (1 → 2 → 4 → 8
segundos; cuatro fallos seguidos y se queda abajo hasta un reinicio o
un cambio de configuración), y una instancia que **ningún documento
abierto ha necesitado en cinco minutos** se apaga — la siguiente
apertura arranca una fresca.

- Los snippets del autocompletado se expanden y se recorren. El
  primer marcador vuelve seleccionado, así que escribir lo reemplaza;
  ⇥ pasa al siguiente y ⇧⇥ al anterior; un marcador escrito más de una
  vez copia lo que se teclea en el otro. Llegar al final, pulsar ⎋ o
  hacer clic fuera devuelve las teclas.
- **Vista ▸ Estado de servidores** lista las instancias en ejecución y
  las transiciones recientes de la sesión, refrescado en vivo, con un
  puntero al registro completo.
