# Arquitectura

Textchum son dos programas que se encuentran en una interfaz C:

```
┌──────────────────────────────────────────────┐
│ Carcasa — Swift, AppKit                      │
│  ventanas · dibujado · entrada · menús       │
└───────────────▲──────────────┬───────────────┘
                │ llamadas C   │ callback de eventos
┌───────────────┴──────────────▼───────────────┐
│ Núcleo — Rust, libtextchum (biblioteca       │
│ estática)                                    │
│  búferes · ediciones · eventos               │
│  (pronto: sintaxis, proyectos, servidores    │
│  de lenguaje)                                │
└──────────────────────────────────────────────┘
```

El reparto de responsabilidades sigue una regla sencilla: todo lo que
responde a *«¿qué es el texto y qué sabemos de él?»* pertenece al núcleo;
todo lo que responde a *«¿cómo se ve y se siente en este sistema
operativo?»* pertenece a la carcasa. El núcleo nunca dibuja. La carcasa nunca
analiza texto.

## Por qué un núcleo compilado tras una carcasa nativa

- **Los problemas difíciles no dependen de la plataforma.** *Ropes*, análisis
  incremental, clientes de protocolos — nada de eso conoce AppKit.
  Mantenerlo en una biblioteca plana lo hace comprobable sin interfaz
  (`cargo test` cubre el núcleo sin ninguna UI a la vista) y portable a
  otras plataformas en el futuro.
- **La capa visible debe ser aburridamente nativa.** La entrada de texto en
  macOS es profunda — IME, teclas muertas, dictado, accesibilidad. Usar
  vistas AppKit reales significa heredarlo todo en lugar de reimplementarlo.
- **Una ABI en C es la frontera más ancha posible.** Swift consume cabeceras
  C de forma nativa; igual que cualquier otra cosa que algún día pueda
  alojar el núcleo.

## La regla de la fuente de verdad

El invariante más importante de todo el código: **el búfer del núcleo es
dueño del documento; lo que la interfaz retiene es una caché de
presentación.**

En concreto, en la ventana del editor actual:

1. AppKit informa de cada cambio de texto inminente — teclear, pegar,
   arrastrar, deshacer — a través de un único método delegado, como un rango
   UTF-16 más la cadena de reemplazo.
2. La carcasa aplica exactamente esa edición al búfer del núcleo *primero*.
3. Solo si el núcleo la acepta procede la vista con su propio cambio. Un
   rechazo (que indicaría un error de programación) rechaza también la
   edición de la vista, de modo que ambos lados solo pueden moverse juntos.
4. Las compilaciones de depuración verifican además la igualdad byte a byte
   de ambos lados tras cada cambio.

Las posiciones cruzan la frontera en las dos unidades que el ecosistema usa
de verdad: desplazamientos de bytes (la unidad nativa del núcleo) y unidades
UTF-16 (la unidad nativa de AppKit y de LSP). El núcleo hace todas las
conversiones; la carcasa nunca cuenta puntos de código.

## Contrato de hilos

Reglas simples, cumplidas estrictamente:

- La carcasa llama al núcleo **solo desde el hilo principal**.
- El núcleo es dueño de todos los hilos de trabajo y entrega eventos por
  **un** callback invocado desde **un** hilo de despacho dedicado — nunca
  desde el hilo del llamante, nunca de forma concurrente consigo mismo.
- El envoltorio Swift (`TextchumKit`) traslada los eventos al actor
  principal antes de que la aplicación los vea, así el código de la
  aplicación vive por completo en el actor principal.

Esto cuesta algo de paralelismo en la frontera y compra la ausencia de toda
una categoría de condiciones de carrera. El trabajo que se beneficia del
paralelismo ocurre *dentro* del núcleo, detrás de la interfaz de un solo
hilo.

## Capas del lado Swift

| Capa | Responsabilidad |
|---|---|
| `CTextchum` | La cabecera C generada, expuesta como módulo de Clang. Sin código. |
| `TextchumKit` | API Swift segura: clases con propiedad determinista, edición basada en `NSRange`, eventos tipados en el actor principal. El único lugar donde aparecen punteros. |
| `Textchum` | La aplicación: ventanas, vistas, menús. Swift corriente, sin FFI. |

De cualquier carcasa futura se espera la misma estratificación: un enlace
fino, un envoltorio idiomático seguro y, encima, la aplicación.
