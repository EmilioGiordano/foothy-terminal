# Referencias

## El repo de foot (lo principal)

| Qué | URL | Licencia |
|---|---|---|
| **foot** — el emulador de terminal | **https://codeberg.org/dnkl/foot** | **MIT** |
| **fcft** — su motor de fuentes/glifos | **https://codeberg.org/dnkl/fcft** | **MIT** |
| tllist — lista enlazada typed, del mismo autor | https://codeberg.org/dnkl/tllist | MIT |
| Wiki de rendimiento | https://codeberg.org/dnkl/foot/wiki/Performance | — |
| Página del proyecto | https://www.pages.codeberg.org/dnkl/foot | — |
| Flathub | https://flathub.org/en/apps/page.codeberg.dnkl.foot | — |

Autor: **Daniel Eklöf** (`dnkl`). Escrito en C (97%), build con Meson.
La licencia MIT permite leer, copiar y reutilizar el código incluso en producto
propietario.

Copia local en `upstream/foot/` (commit `85655c7`) y `upstream/fcft/`.

### Documentación dentro del propio repo

```
upstream/foot/README.md                # features, modo servidor, atajos, OSCs
upstream/foot/INSTALL.md               # cómo compilar, incluido PGO
upstream/foot/doc/benchmark.md         # benchmarks vs Alacritty/URxvt/XTerm
upstream/foot/doc/foot.ini.5.scd       # TODAS las opciones de configuración
upstream/foot/doc/foot-ctlseqs.7.scd   # secuencias de escape soportadas
upstream/foot/doc/foot.1.scd           # man page
upstream/foot/pgo/                     # scripts de build con PGO
```

`foot.ini.5.scd` es la referencia definitiva para configurar tu foot actual.

---

## Terminales de referencia para el proyecto

| Proyecto | Lenguaje | Plataformas | Por qué mirarlo |
|---|---|---|---|
| **[Alacritty](https://github.com/alacritty/alacritty)** | Rust | Win/mac/Linux | **La mejor plantilla.** Su crate `alacritty_terminal` (motor, portable) separado del binario (UI) es exactamente la arquitectura a copiar. Minimalista como foot pero ya multiplataforma. |
| **[WezTerm](https://github.com/wez/wezterm)** | Rust | Win/mac/Linux | El más completo en Windows. Fuente del crate `portable-pty`. Buena referencia de cómo abstraer diferencias de SO. |
| **[Windows Terminal](https://github.com/microsoft/terminal)** | C++/WinUI | Windows | Referencia canónica de cómo se hace bien en Windows. El código de ConPTY vive aquí. MIT. |
| **[Ghostty](https://github.com/ghostty-org/ghostty)** | Zig | mac/Linux | Arquitectura moderna. **Sin Windows nativo** (solo WSL). Útil como diseño, no como plantilla de Windows. |
| **[Kitty](https://github.com/kovidgoyal/kitty)** | C/Python | mac/Linux | Origen de los subrayados con estilo y del protocolo de teclado extendido. |

---

## Splits y layout

- **[Using tree data structures to implement terminal split panes](https://warp.hashnode.dev/using-tree-data-structures-to-implement-terminal-split-panes-more-fun-than-it-sounds)**
  — Warp Engineering, **19 enero 2022**. *La* referencia. Árbol de ramas/hojas,
  flex ratios, complejidades, y el aviso sobre la dificultad de la navegación
  direccional.
- [Split panes — Warp docs](https://docs.warp.dev/terminal/windows/split-panes/) — comportamiento desde el punto de vista de usuario.
- [Warp overview](https://warpdotdev-warp.mintlify.app/terminal/overview) — describe su stack: Rust, render con `wgpu`, crates `warpui_core`/`warpui`, `grid_renderer` y `blockgrid_renderer`.

Nota histórica: los splits en terminal vienen de `screen` (1987) → `tmux` (2007) →
iTerm2 / Terminator / i3. Warp publicó su diseño **año y medio antes** de que
Ghostty se conociera públicamente.

---

## Windows: PTY

- **[Introducing the Windows Pseudo Console (ConPTY)](https://devblogs.microsoft.com/commandline/windows-command-line-introducing-the-windows-pseudo-console-conpty/)** — el artículo fundacional de Microsoft.
- **[Ejemplo oficial de app con pseudoconsole APIs](https://github.com/microsoft/terminal/commit/ac843745fa71728f6e385eed2e27805350d90c89)** — app mínima funcional. Punto de partida para la fase 0.
- [microsoft/terminal](https://github.com/microsoft/terminal) — implementación de producción.
- [XConPty](https://github.com/Biswa96/XConPty) — experimentos con la API.
- API central: `CreatePseudoConsole()`. Disponible desde Windows 10 (2018). Antes hacía falta WinPTY; ya no.

---

## libghostty — el atajo

- **[libghostty-vt docs](https://libghostty.tip.ghostty.org/)** — qué provee: parseo de escape sequences, estado del terminal, encoding de input (teclado/ratón/foco), scrollback, line wrapping, **resize reflow**, snapshots, parsers OSC/SGR, formateo a texto/VT/HTML.
- **[ghostling](https://github.com/ghostty-org/ghostling)** — terminal mínima viable construida sobre la API C de libghostty. El mejor punto de partida si eliges este camino.
- [awesome-libghostty](https://github.com/Uzaaft/awesome-libghostty) — proyectos que la usan.
- [libghostty C API Overview](https://ghostty-org-ghostty.mintlify.app/api/overview)
- [Libghostty Is Coming — Mitchell Hashimoto](https://mitchellh.com/writing/libghostty-is-coming)

**Estado:** `libghostty-vt` ya es usable en Zig y C, y es compatible con **macOS,
Linux, Windows y WebAssembly**.
⚠️ **La API no es estable**: *"Breaking changes are expected in future versions."*
La API completa de embedding se estabiliza en un release futuro.

---

## Crates de Rust relevantes

| Crate | Para qué |
|---|---|
| `alacritty_terminal` | Motor de terminal completo, sin UI. La capa 1 entera. |
| `vte` | Solo el parser de secuencias de escape (lo usa Alacritty). |
| `portable-pty` | Abstracción de PTY: ConPTY en Windows, `openpty` en Unix. De WezTerm. |
| `winit` | Ventana + input multiplataforma. |
| `wgpu` | Render GPU multiplataforma (lo usa Warp). |
| `cosmic-text` / `swash` | Shaping y caché de glifos. El análogo a `fcft`. |
| `tiny-skia` | Rasterizado 2D en CPU. El análogo a `pixman`. |

---

## Especificaciones y tests

- **Máquina de estados de Paul Williams para parsers DEC ANSI** — https://vt100.net/emu/dec_ansi_parser — el diagrama que siguen foot, Alacritty, Kitty, Ghostty y WezTerm. Los 16 primeros estados de `vt.c:28` son literalmente esto.
- [XTerm Control Sequences](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html) — la referencia histórica de secuencias de escape.
- [vtebench](https://github.com/alacritty/vtebench) — la suite de benchmarks que usa foot en `doc/benchmark.md`. Úsala para tus propias mediciones.
- [wraptest](https://github.com/mattiase/wraptest) — tests de las reglas de wrapping de línea. Citado en `terminal.h` a propósito del *Last Column Flag*.
- [Terminal WG: Synchronized Updates](https://gitlab.freedesktop.org/terminal-wg/specifications/-/merge_requests/2)
- [Kitty: subrayados con estilo](https://sw.kovidgoyal.net/kitty/underlines/)

---

## Contexto de este proyecto

- [Ghostty 1.0 has been summoned — LWN](https://lwn.net/Articles/1004377/) — 1.0 el 26 dic 2024, MIT.
- [Ghostty 1.0 is Coming — Mitchell Hashimoto](https://mitchellh.com/writing/ghostty-is-coming)
- [Ghostty: Reflecting on Reaching 1.0](https://mitchellh.com/writing/ghostty-1-0-reflection)
- [Niri](https://github.com/YaLTeR/niri) — el compositor. Rust, GPL-3.0, de YaLTeR. **Sin relación con foot.**
