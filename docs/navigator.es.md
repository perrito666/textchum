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
se resuelve en este orden:

1. el `.textchum.json` más cercano — la asignación explícita, colocada a
   mano;
2. la **raíz de control de versiones más externa** (`.git`, `.hg`,
   `.svn`): un repositorio es un solo proyecto sin importar cuántos
   manifiestos anidados contenga — un paquete Python en una subcarpeta
   pertenece al repositorio, y los repositorios anidados se resuelven al
   más externo;
3. fuera del control de versiones, el archivo de construcción/manifiesto
   más cercano (`Cargo.toml`, `go.mod`, `package.json`,
   `pyproject.toml`, `Package.swift`, `build.zig`, `Makefile`, …).

Los archivos fuera de todo proyecto se reúnen bajo **Other**. La regla
del paso 2 — gana el repositorio — puede relajarse por proyecto: el
interruptor **Manifest projects** de
[Ajustes → Projects](configuration.es.md) vuelve a dividir una raíz por
sus manifiestos de lenguaje, para repositorios que en realidad son
varios proyectos disfrazados de uno.

Las filas muestran solo el nombre del archivo — hasta que dos archivos
abiertos comparten uno: entonces cada uno muestra justo la cola de ruta
necesaria para distinguirlos (los títulos de las pestañas hacen lo
mismo). El botón en la parte superior de la lista — o View → Toggle
Path Display (⌥⌘T) — cambia todas las
filas a su ruta desde la raíz del proyecto mientras está activo;
deliberadamente no se recuerda entre arranques — es un vistazo rápido,
no un modo.

Un clic derecho sobre una fila de la lista o una entrada del árbol
ofrece la ubicación del archivo en todas sus formas útiles: el nombre a
secas, la ruta relativa a la raíz del proyecto, la ruta absoluta y —
dentro de un repositorio git con remoto — la URL del archivo en su
forja, hablando con naturalidad las formas de URL de GitHub, GitLab y
Forgejo. Los mismos elementos actúan sobre la pestaña frontal desde
**File → Copy Path**.

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

Las carpetas expandidas son estado compartido: abra una carpeta en una
pestaña y estará abierta en todas (y en cualquier ventana que muestre el
mismo proyecto).

Los archivos ocultos no se listan.

## Aún no está

- Acciones de renombrar / mostrar en Finder sobre las entradas del árbol.
- Respetar `.gitignore` en el árbol.
- Una asignación manual de «este archivo pertenece a aquel proyecto» para
  cuando la heurística de marcadores se equivoque.
