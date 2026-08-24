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

- Autocompletado, *hover*, ir a definición, referencias, renombrar,
  formatear — los diagnósticos llegaron primero porque la validación es
  la promesa central del producto.
- Reinicio automático de servidores caídos (una caída se informa; reabrir
  el archivo arranca una instancia nueva).
- Apagado por inactividad de instancias sin uso y un panel de estado de
  servidores.
- Configuración de servidores propia (un `servers.json` con las reglas de
  salida de emergencia de la configuración).
