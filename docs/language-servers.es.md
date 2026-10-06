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

El archivo de una biblioteca es la excepción: un crate del registro de
cargo, las fuentes de la biblioteca estándar, un paquete en
`site-packages` o `node_modules`, una cabecera de un SDK. Saltar a la
definición cae en uno a menudo, y lo sirve el servidor del proyecto del
que se venía — que ya conoce el archivo como dependencia —, no un
servidor arrancado sobre el directorio de la biblioteca, que leería el
crate como un proyecto propio y se pondría a compilarlo. El archivo
conserva además el árbol y los ajustes del proyecto desde el que se
llegó a él.

Los proyectos anidados en otro — los miembros de un workspace de uv o de
Cargo, una vez que **manifest projects** los hizo proyectos — son el
único caso con elección. Cuando el proyecto exterior tiene **recursive
config**, toman sus entradas de servidor y los atiende *su* instancia:
un servidor arriba, iniciado allí, que ve a todos los miembros y el
entorno que comparten. Con **separate nested servers** además, conservan
las entradas y cada uno las corre por su cuenta, en su propia carpeta,
para miembros que llevan cada uno su entorno. Un proyecto anidado con
entrada propia es siempre independiente. Ambos interruptores existen por
proyecto y por defecto, en Settings ▸ Projects y junto a cada entrada de
proyecto en Settings ▸ Language Servers; véase
[configuración](configuration.md#proyectos).

## Lo que se ve

- Los hallazgos llegan mientras se escribe (enviados en lotes con
  *debounce*) y encuadran el texto afectado en su color: rojo para errores, naranja para
  avisos, azul para notas. Una marca se queda sobre el código que señala
  mientras se edita encima o alrededor, hasta que el servidor vuelve a
  informar; un *lint* que solo corre al guardar (el de cargo, por
  ejemplo) se refresca con el siguiente guardado.
- El subtítulo de la ventana los cuenta («2 errors, 1 warning»).
- **Autocompletado al escribir**: las sugerencias aparecen tras
  caracteres de identificador y `.`, filtradas mientras se sigue
  escribiendo — ↑/↓ para elegir, ⏎ o ⇥ para aceptar, ⎋ para descartar,
  ⌃Espacio para pedirlas explícitamente. Un elemento aceptado sustituye
  el tramo que nombra el servidor (un autocompletado *postfix* se traga
  el receptor y el punto) y trae consigo sus ediciones adicionales, un
  *import* por ejemplo.
- **Show Callers** (menú Edit y menú contextual; acción `showCallers`)
  lista cada lugar donde se llama a la función bajo el cursor, en la
  lista que usa Find References. Las referencias responden «dónde está
  escrito este nombre»; esto responde «qué la ejecuta», y deja fuera la
  declaración, los *imports* y las menciones.
- **Go to Symbol in Project…** (⌥⌘T, Ctrl+Alt+O en Linux; acción
  `projectSymbols`) encuentra una función, un tipo o una constante en
  cualquier parte del proyecto por parte de su nombre, tal como los
  conoce el servidor del documento en primer plano, y salta a su
  declaración.
- **Se muestran los tipos inferidos y los nombres de los parámetros.**
  Los tipos que un servidor deduce — de un `let` sin anotación, del
  argumento de una clausura — y los nombres de los parámetros a los que
  se pasa cada argumento aparecen atenuados dentro de la línea, donde
  el servidor los coloca: `let d = make();` se lee `let d: Drinker =
  make();`, y `pair(d, 2)` se lee `pair(who: d, n: 2)`. No son parte
  del texto: el cursor pasa por encima, no se pueden seleccionar ni
  copiar, y se quedan con su línea mientras se escribe y se vuelven a
  pedir cuando la escritura hace una pausa. Settings ▸ General ▸ «Show
  the types the language server inferred» los apaga
  (`editor.inlay_hints`). Todavía no: en Linux las pistas se reúnen al
  final de la línea, cada tipo con su nombre, y los nombres de
  parámetro se dejan fuera, porque la vista de texto de GTK no puede
  abrir un hueco dentro de una línea.
 Sin nada seleccionado, un
  cursor que descansa sobre un nombre marca dónde más se usa ese
  símbolo, tal como lo conoce el servidor: una variable que lo oculta
  con el mismo nombre se deja en paz. Seleccionar una palabra sigue
  marcando el mismo texto, con servidor o sin él. Ambos siguen el
  ajuste de marcar ocurrencias.
- **La llamada que se escribe dice qué recibe.** Un paréntesis de
  apertura o una coma muestra la firma de la función junto al cursor,
  con el parámetro en curso en negrita; el paréntesis de cierre, ⎋ o
  salir de la línea la guarda.
- Dejar el ratón sobre un símbolo muestra la documentación **hover** del
  servidor en un globo, con el Markdown que envían los servidores ya
  renderizado — bloques de código en monoespaciada, énfasis y código en
  línea con su estilo. Solo se dispara sobre identificadores (nunca
  sobre espacios ni comentarios), sigue abierto mientras el puntero se
  mueve dentro del símbolo que explica, y se puede apagar en Vista ▸
  Documentación al pasar (o en Ajustes). Ajustes ▸ General también
  puede pedir que se mantenga una tecla — ⇧, ⌃, ⌥ o ⌘ — para mostrarlo,
  de modo que la documentación aparezca cuando se pide y no donde
  repose el puntero; pulsar la tecla con el puntero ya sobre un símbolo
  también la pide. **Mostrar documentación del símbolo** (⌃⌘H) la pide
  para el símbolo bajo el cursor a demanda — incluso con el hover del
  ratón apagado.
- **El globo se puede usar.** Se queda mientras el puntero está sobre
  el texto al que se refiere y mientras está dentro del globo, y se
  cierra un momento después de que el puntero deja ambos, así que se
  puede llegar a él. Su texto se puede seleccionar y copiar, y la
  documentación más larga que el globo se desplaza desde su inicio.
- **Saltar a la definición** (⌃⌘J, o ⌘-clic) va al símbolo bajo el
  cursor — entre archivos, abriendo o trayendo al frente el destino
  según haga falta. Sobre la definición no tiene adónde ir, así que
  responde la pregunta que queda: quién usa esto. Un uso es un salto,
  varios abren la lista, y un símbolo al que nadie se refiere lo dice.
  Un servidor que contesta con varias definiciones —una declaración y
  su implementación— las ofrece igual. El atajo de buscar referencias
  sigue como estaba.
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
- **Una línea marcada se puede leer.** Al posar el puntero sobre un
  subrayado aparece lo que dijo el servidor, y **Mostrar diagnóstico de
  la línea** (⌃⌘E, Ctrl+Alt+E en Linux) dice lo mismo para la línea del
  cursor —el cursor suele estar al final de la línea que se arregla y
  no dentro de la marca, así que responde por la línea—. El mensaje
  nombra su gravedad, porque un subrayado solo dice que algo va mal y
  un aviso no debería leerse como un error. Sin ida y vuelta: el
  hallazgo ya está a mano.
- **Diagnósticos…** (⇧⌘E, Ctrl+Shift+E en Linux) lista todos los
  hallazgos del documento en el orden en que aparecen —el orden en que
  se arreglan y el que muestra el margen—, con la gravedad en cada
  fila. ⏎ salta, y el salto entra en la pila de vuelta atrás.
- **Acciones de código…** (⌘., Ctrl+. en Linux) pregunta qué puede
  hacer el servidor con el sitio donde está el cursor —importar este
  nombre, añadir la rama que falta, quitar la variable sin usar— y
  lista lo que llega, con la sugerencia del propio servidor marcada.
  Los hallazgos bajo el cursor viajan con la petición tal como los
  publicó el servidor, con `code` y `data` incluidos: así reconoce el
  servidor lo que él mismo encontró, y uno reconstruido no le dice
  nada. Una acción que el servidor mandó sin su edición se le devuelve
  para que la termine antes de aplicarla, y una que trae un comando en
  vez de una edición la ejecuta el servidor.
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

Todavía no: Show Callers lista quién llama a una función, no a qué
llama ella. El coloreado viene solo de la gramática; los *semantic
tokens* de un servidor no se usan.

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
| C++ | clangd | Xcode CLT, or `brew install llvm` |
| TypeScript | typescript-language-server | `npm install -g typescript-language-server typescript` |
| Java | jdtls | `brew install jdtls` |
| C# | csharp-ls | `dotnet tool install --global csharp-ls` |
| Ruby | ruby-lsp | `gem install ruby-lsp` |
| Lua | lua-language-server | `brew install lua-language-server` |
| Haskell | haskell-language-server | `ghcup install hls` |
| OCaml | ocamllsp | `opam install ocaml-lsp-server` |
| Scala | metals | `cs install metals` |
| Nix | nil | `nix profile install nixpkgs#nil` |
| CMake | cmake-language-server | `uv tool install cmake-language-server` |
| JSON | vscode-json-language-server | `npm install -g vscode-langservers-extracted` |
| HTML | vscode-html-language-server | `npm install -g vscode-langservers-extracted` |
| CSS | vscode-css-language-server | `npm install -g vscode-langservers-extracted` |
| YAML | yaml-language-server | `npm install -g yaml-language-server` |
| TOML | taplo | `brew install taplo` |
| Markdown | marksman | `brew install marksman` |
| Elixir | elixir-ls | `brew install elixir-ls` |
| PHP | intelephense | `npm install -g intelephense` |
| Dockerfile | docker-langserver | `npm install -g dockerfile-language-server-nodejs` |
| XML | lemminx | `brew install lemminx` |
| SQL | sqls | `go install github.com/sqls-server/sqls@latest` |
| R | languageserver | `R -e 'install.packages("languageserver")'` |

Las plantillas de Go también las atiende `gopls`, C++ lo atiende
`clangd`, y TypeScript y TSX los servidores de JavaScript. Varios
lenguajes tienen más de un servidor registrado: Python cuenta con
`pyright`, `basedpyright`, `pylsp`, `ruff`, `jedi`, `ty` y `pyrefly`;
JavaScript, TypeScript y TSX con `typescript-language-server`, `vtsls`,
`deno` y `biome`; Ruby con `ruby-lsp`, `solargraph` y `rubocop`, este
último un *linter* para correr junto a uno de los otros dos; PHP con
`intelephense` y `phpactor`.

La tabla nombra el que se usa cuando la configuración no dice nada; a
los demás se los pide por identificador, y `lsp.servers` acepta uno que
el editor no conozca.

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

El campo de lenguaje ofrece los lenguajes que esta compilación conoce y
sigue aceptando cualquier texto: se puede configurar un lenguaje antes
de que exista una gramática para él, y la entrada sigue sirviendo
cuando llega.

### Más de un servidor para un lenguaje

Un lenguaje suele ser un comprobador de tipos y un *linter*, y cada uno
es un servidor por sí mismo. Una entrada los lista con ` ;; ` entre
ellos:

```json
{ "lsp": { "defaults": { "python": "pyright ;; ruff" } } }
```

Todos reciben el documento y sus cambios, y sus hallazgos se muestran
juntos. Una petición — *hover*, autocompletado, ir a la definición,
formato — va al primer servidor listado que dice ofrecerla, así que el
orden es el orden de preferencia. Una acción de código se pide al
servidor que informó del hallazgo bajo el cursor. Si uno no está
instalado, los demás corren igual.

Todavía no: el autocompletado viene de un servidor, no se combina el de
todos.

### Definir un servidor que el editor no conoce

`lsp.servers` guarda entradas con la misma forma que usa la tabla
incorporada, así que se puede añadir un servidor sin tocar el código, y
redefinir uno ya conocido reutilizando su identificador:

```json
{
  "lsp": {
    "servers": {
      "basedpyright": {
        "command": "{project}/.venv/bin/basedpyright-langserver",
        "args": ["--stdio"],
        "languages": ["python"],
        "install": "uv tool install basedpyright"
      }
    },
    "defaults": {"python": "basedpyright"}
  }
}
```

`command` es obligatorio; el resto se puede omitir. La tabla incorporada
sigue disponible junto a estas entradas, así que una configuración que no
dice nada tiene servidores igualmente, y una versión que aprende uno
nuevo lo ofrece sin reescribir la configuración. Definir un servidor no
cambia cuál usa un lenguaje por omisión: eso lo decide `lsp.defaults`.

### Configuración para un servidor

Los servidores aceptan configuración — qué *lints* corren, cuán
estricto es el comprobador de tipos — y `lsp.settings` es donde va: un
objeto por id de servidor, con las claves por sección tal como el
servidor las pide.

```json
{
  "lsp": {
    "settings": {
      "rust-analyzer": {"rust-analyzer": {"check": {"command": "clippy"}}},
      "gopls": {"gopls": {"staticcheck": true}},
      "pyright": {"python": {"analysis": {"typeCheckingMode": "strict"}}}
    },
    "init_options": {
      "typescript-language-server": {"preferences": {"quotePreference": "single"}}
    }
  }
}
```

El objeto se envía al servidor una vez que arrancó y se entrega por
sección cuando el servidor pregunta, que entre las dos es como los
servidores leen su configuración. Es la forma de la tabla `settings` de
una configuración de nvim-lspconfig, así que una se puede traer tal
cual. `lsp.init_options` contiene lo que un servidor lee de
`initializationOptions`. Una línea de comando escrita a mano no tiene
id; su configuración se busca bajo el nombre del comando que ejecuta.

Un servidor lee su configuración al arrancar. Aplicar un
[perfil de lenguaje](configuration.md#perfiles-de-lenguaje) reinicia
los servidores; tras una edición a mano, Settings ▸ Language Servers
tiene el botón que lo hace.

Settings ▸ Language Servers las edita en **Server settings**: un objeto
JSON por servidor, para todos los proyectos o para una raíz. El objeto
de un proyecto vive en `lsp.project_settings.<raíz>.<servidor>` y, como
toda entrada por proyecto, sustituye a la de por defecto en vez de
sumarse a ella. Un texto que no es un objeto JSON no se guarda, y el
campo lo dice.

Todavía no: `lsp.init_options` se edita a mano, y no es por proyecto.

### Nombrar un servidor y apuntar a uno dentro del proyecto

La entrada de un lenguaje acepta el identificador de un servidor que el
editor conoce o una línea de órdenes.

Un identificador trae consigo los argumentos del servidor. Un lenguaje
con más de un servidor registrado usa el primero salvo que la
configuración nombre otro.

Una línea de órdenes se ejecuta tal cual, con dos sustituciones:

- `{project}` — la raíz del proyecto con la que se indexa la instancia.
- `{home}` — el directorio personal del usuario.

```json
{"lsp": {"defaults":
  {"python": "{project}/.venv/bin/basedpyright-langserver --stdio"}}}
```

Es lo que necesita un repositorio que lleva sus propias herramientas: un
entorno virtual, una entrada de `node_modules/.bin`, un servidor incluido
en el propio repositorio. La sustitución ocurre por argumento después de
dividir la línea, así que una ruta con espacios sigue siendo un
argumento.

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
