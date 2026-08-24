# El navegador

Cada ventana del editor lleva un cajón de navegación a su izquierda
(se alterna con **⌘0**, o View → Toggle Navigator). Tiene dos paneles
apilados.

## Búferes abiertos, agrupados por proyecto

El panel superior lista los documentos abiertos del **grupo de pestañas de
esta ventana** — los archivos abiertos como pestañas comparten una lista,
mientras que las ventanas separadas mantienen mundos separados (que los
archivos se abran como pestañas o ventanas es un
[ajuste](configuration.md)). Los documentos se agrupan por el **proyecto**
al que pertenecen. El proyecto de un archivo
es el directorio ancestro más cercano que parece una raíz de proyecto: un
directorio de control de versiones (`.git`, `.hg`, `.svn`) o un archivo de
construcción/manifiesto (`Cargo.toml`, `go.mod`, `package.json`,
`pyproject.toml`, `Package.swift`, `build.zig`, `Makefile`, …). Gana el
más cercano: en un monorepo, un archivo dentro de un *crate* con su propio
`Cargo.toml` pertenece a ese *crate*, no a la raíz del repositorio. Los
archivos fuera de todo proyecto se reúnen bajo **Other**.

Esta es la misma noción de «proyecto» que usa el resto de Textchum (y por
la que se delimitarán los servidores de lenguaje), así que el cajón hace
también de detector de verdades: si un archivo aparece agrupado en un
sitio sorprendente, así es exactamente como lo ve también el resto de la
aplicación.

El documento de la ventana actual va en negrita; los documentos con
cambios sin guardar muestran un punto. Al hacer clic en un documento, su
ventana pasa al frente.

## El árbol del proyecto

El panel inferior muestra el árbol de carpetas del proyecto del documento
actual, desde su raíz. Al hacer clic en un archivo se abre — o su ventana
pasa al frente si ya está abierto. Los documentos sin proyecto (el grupo
**Other**) no muestran árbol.

Los archivos ocultos no se listan.

## Aún no está

- Acciones de renombrar / mostrar en Finder sobre las entradas del árbol.
- Respetar `.gitignore` en el árbol.
- Una asignación manual de «este archivo pertenece a aquel proyecto» para
  cuando la heurística de marcadores se equivoque.
