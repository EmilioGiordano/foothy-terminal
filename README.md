# foot-terminal — investigación y diseño

Documentación consolidada sobre **foot** (el emulador de terminal Wayland que uso
a diario bajo Niri) y el diseño de una terminal propia para Windows: rápida y
ligera como foot, pero con multiplexado tipo Warp (splits y tabs).

Elaborado el 2026-08-19, sobre el código fuente real de foot en el commit `85655c7`
(rama `master`), no sobre documentación de segunda mano.

## Índice

| Documento | Contenido |
|---|---|
| [docs/01-como-funciona-foot.md](docs/01-como-funciona-foot.md) | **Cómo funciona foot por dentro.** Arquitectura, bucle de eventos, estructura del grid, parser VT, render multihilo, modo servidor. Con referencias a archivo y línea del código clonado. |
| [docs/02-diseno-terminal-windows.md](docs/02-diseno-terminal-windows.md) | **El documento de diseño.** Arquitectura propuesta en 3 capas, reglas de rendimiento, elección de stack, plan por fases, decisiones de scope. |
| [docs/03-referencias.md](docs/03-referencias.md) | **Repos y enlaces útiles.** foot, fcft, Alacritty, WezTerm, Ghostty/libghostty, Windows Terminal, ConPTY, especificaciones VT. |
| [docs/04-mediciones-locales.md](docs/04-mediciones-locales.md) | **Números medidos en esta máquina**, más los benchmarks oficiales de foot. Incluye un hallazgo de configuración. |

## El repo de foot

```
https://codeberg.org/dnkl/foot        # el terminal      — MIT, C
https://codeberg.org/dnkl/fcft        # su motor de fuentes — MIT, C
```

Autor: **Daniel Eklöf** (`dnkl`). Licencia **MIT** en ambos: se puede leer, copiar
y reutilizar incluso en producto propietario.

## Código fuente clonado

```
upstream/foot/    # ~53.000 líneas de C, commit 85655c7
upstream/fcft/    # la librería de carga de fuentes y rasterizado de glifos
```

Para actualizar:

```sh
cd upstream/foot && git pull
```

## Resumen en cinco puntos

1. **foot no es de Niri.** Son proyectos independientes y sin relación formal.
   Niri no trae terminal; su configuración de ejemplo ni siquiera usa foot (usa
   `alacritty`). foot es simplemente el compañero de facto del mundo Wayland
   minimalista.

2. **foot no usa GPU.** Rasteriza glifos con FreeType vía `fcft` y los compone
   con `pixman` sobre un buffer de memoria compartida. Ese es justamente el
   motivo de que arranque tan rápido: nunca negocia un contexto con el driver
   gráfico.

3. **Los splits no son lo que hace pesada a una terminal.** Un pane extra son
   unos MB de grid y scrollback. Warp pesa 191 MB por su renderer GPU propio, su
   modelo de *blocks* y la IA integrada — no por los panes.

4. **`foot --server` ya es el 80% de la arquitectura multi-pane.** Un proceso
   hospedando N terminales que comparten el caché de glifos. Lo único que le
   falta es un árbol de layout que las meta en una sola ventana. Que no lo tenga
   es una decisión de scope de su autor, no una limitación técnica.

5. **La lección central de foot es de producto, no de código:** lo que decides
   *no* soportar es la feature. Sin X11, sin tabs, sin splits, sin ligaduras.
