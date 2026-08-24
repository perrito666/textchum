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
  servidor en un globo.
- **Saltar a la definición** (⌃⌘J) va al símbolo bajo el cursor — entre
  archivos, abriendo o trayendo al frente el destino según haga falta.
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

Los cambios se aplican a los servidores arrancados después; el botón
**Restart Servers Now** de la pestaña retira las instancias en marcha y
las relanza con la nueva configuración.

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

## Aún no está

- Referencias, renombrar, formatear.
- Los marcadores de fragmento (snippets) en el autocompletado se
  aplanan a texto plano.
- ⌘-clic como disparador alternativo de Saltar a la definición.
- El renderizado de Markdown en los globos de *hover* (muestran el texto
  en crudo).
- Reinicio automático de servidores caídos (una caída se informa; reabrir
  el archivo arranca una instancia nueva).
- Apagado por inactividad de instancias sin uso y un panel de estado de
  servidores.
