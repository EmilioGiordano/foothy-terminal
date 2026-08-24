# Documento de diseño: terminal rápida y ligera con multiplexado

**Objetivo:** una terminal con el consumo y la latencia de foot, pero con splits y
tabs como Warp. Multiplataforma si es viable.

**Motivación explícita:** hartazgo del consumo de RAM de las apps de escritorio
modernas, y degradación progresiva del rendimiento de Warp (los splits tardan cada
vez más en abrir).

---

## 0. ¿Es esto realista? Lee esta sección antes que ninguna otra

Respuesta corta: **sí, pero solo si eliges bien el alcance.** El proyecto entero
vive o muere en esta decisión, así que va primero.

### Los datos duros de referencia

| Proyecto | Dev(s) | Tiempo hasta 1.0 | Plataformas hoy |
|---|---|---|---|
| **Ghostty** | Mitchell Hashimoto (creador de Vagrant/Terraform), a tiempo completo, con financiación y ~5.000 beta testers | 2021 → dic 2024 = **~3,5 años** | macOS, Linux. **Windows no.** |
| **Alacritty** | equipo, open source | 2016 → 1.0 en 2024 = **~8 años** | Win/mac/Linux |
| **foot** | dnkl, en solitario | ~2019 → hoy, 53k LOC | **Solo Wayland** |
| **WezTerm** | wez, en solitario | 2018 → hoy | Win/mac/Linux |

La lectura honesta: **Hashimoto, a tiempo completo y con recursos, tardó 3 años y
medio y aún no tiene Windows.** Si tu plan es "terminal completa, nativa, en tres
plataformas, desde cero", eso son años, no meses. No porque seas peor dev, sino
porque el problema tiene una cola de casos límite prácticamente infinita
(reflow, IME, ligaduras, emoji de anchura variable, ratón, portapapeles, mil
secuencias de escape con comportamientos históricos contradictorios).

### Pero eso NO significa que no puedas

Significa que tienes que separar dos cosas que la gente confunde:

- **"Una terminal que compite con Ghostty en features"** → años. Descártalo.
- **"Una terminal que hace exactamente lo que YO necesito, rapidísima y ligera"**
  → **3 a 6 meses a tiempo parcial**, y es completamente alcanzable.

foot **es** la segunda cosa. Ese es su secreto entero. Su autor no ganó a nadie en
features; recortó el problema hasta que cupo en una persona.

### Dónde está realmente el trabajo

Reparto aproximado del esfuerzo total, según lo que se ve en el código de foot:

| Parte | % del esfuerzo | ¿Portable entre SO? | ¿Se puede evitar? |
|---|---|---|---|
| Parser VT + grid + scrollback + reflow | **~45%** | **100% portable** | **SÍ** — `alacritty_terminal` o `libghostty-vt` |
| Render de texto (shaping, caché, atlas) | ~20% | parcial | parcialmente (crates existentes) |
| Capa de ventana/input/IME por plataforma | ~20% | **no, ×3** | no |
| PTY | ~5% | no, pero resuelto | **SÍ** — `portable-pty` |
| Layout tree (splits/tabs) | **~5%** | 100% portable | no, y es lo divertido |
| Config, temas, docs, empaquetado | ~5% | portable | no |

Dos conclusiones que cambian el proyecto:

1. **El 50% del trabajo se puede tomar prestado hoy.** El parser VT y el PTY son
   problema resuelto, con licencias permisivas.
2. **Los splits — lo que TÚ quieres — son el 5%.** Es la parte más pequeña y la
   mejor documentada. Warp publicó su diseño.

Lo que no se puede evitar es la capa de ventana/input por plataforma. **Ahí es
donde "multiplataforma" cuesta 3×**, y es exactamente donde Ghostty se atascó con
Windows.

### Sobre tu queja concreta con Warp

Que los splits de Warp se vuelvan cada vez más lentos **no es inherente a tener
splits**. Un pane son unos MB de grid. Las causas plausibles en Warp son otras:
el modelo de *blocks* (que guarda cada comando y su output como objeto vivo, y
crece sin techo), el renderer GPU propio con su framework de UI encima, y el
estado acumulado de sesión.

Esto es una buena noticia para ti: **el problema que quieres resolver es real y es
resoluble**, y no exige reinventar el emulador de terminal — exige no cometer los
errores de arquitectura que lo causan. Las reglas de la sección 3 son literalmente
la lista de esos errores.

### Veredicto

| Plan | Tiempo realista | Recomendación |
|---|---|---|
| A. Terminal completa nativa en Win+mac+Linux, desde cero | 2-4 años a tiempo parcial | ❌ No |
| B. **Una plataforma primero, scope recortado, core prestado** | **3-6 meses a tiempo parcial hasta uso diario** | ✅ **Este** |
| C. Contribuir splits a una terminal existente | semanas-meses | ✅ válido si el objetivo es el resultado, no el aprendizaje |

Y la clave que hace que B no cierre puertas: **si separas las capas desde el día 1
como se describe abajo, añadir macOS y Linux después es trabajo aditivo, no una
reescritura.** Multiplataforma se convierte en una decisión que tomas en el mes 6
con un producto funcionando, no en una apuesta que haces el día 1 sin nada.

---

## 1. Arquitectura

```
┌──────────────────────────────────────────────────────────┐
│  CAPA 3 — Plataforma                                     │
│  ventana, eventos de input, IME, portapapeles, DPI       │   ← ×N plataformas
│  Windows: Win32/WinUI  ·  macOS: AppKit  ·  Linux: Wayland│
├──────────────────────────────────────────────────────────┤
│  CAPA 2 — Layout + render                                │
│  árbol de panes (ramas/hojas + flex), damage, compositor │   ← portable
│  caché de glifos COMPARTIDO por todos los panes           │
├──────────────────────────────────────────────────────────┤
│  CAPA 1 — Core de terminal          × N instancias        │
│  parser VT · grid (ring buffer) · scrollback · PTY       │   ← 100% portable
│  SIN dependencias de UI. Testeable sin abrir una ventana. │
└──────────────────────────────────────────────────────────┘
```

**Las dos reglas invariantes:**

1. La **capa 1 no sabe que existen los splits**. Recibe bytes, mantiene un grid,
   emite eventos. Nada más. Una instancia por pane, sin comunicación entre ellas.
2. La **capa 2 no sabe renderizar glifos ni hablar con el SO**. Decide qué
   rectángulo ocupa cada pane y qué celdas están sucias.

Si mezclas capas, pierdes las tres cosas que quieres: el testing sin GUI, la
portabilidad, y la capacidad de optimizar después.

### Proceso único, N terminales

Como `foot --server` (ver `docs/01`, sección 9): **un proceso hospeda todos los
panes**, compartiendo caché de glifos, fuentes y bucle de eventos.

Con la corrección que trae el README de foot: el **I/O** puede ir en un bucle
compartido, pero **el render de cada pane debe tener sus propios hilos**. Así un
pane ruidoso no bloquea el dibujado de los demás.

Y la advertencia de foot aplica igual: si el proceso cae, caen todos los panes.
Vale la pena considerar persistencia de sesión desde temprano.

---

## 2. El árbol de panes

Modelo de Warp ([blog, 19-ene-2022](https://warp.hashnode.dev/using-tree-data-structures-to-implement-terminal-split-panes-more-fun-than-it-sounds)),
que es también el de i3, Ghostty y tmux:

```
                  Rama(vertical)
                  /            \
        Rama(horizontal)      Hoja(term C, flex 0.3)
          /          \
   Hoja(term A     Hoja(term B
    flex 0.5)       flex 0.5)
```

- **Nodo rama**: orientación (H/V) + lista de hijos
- **Nodo hoja**: una instancia del core de terminal (capa 1)
- **flex ratio**: proporción del hijo respecto a su rama → el resize sale gratis y
  las dimensiones globales se conservan solas

Operaciones, con la complejidad que reporta Warp:

| Operación | Cómo | Coste |
|---|---|---|
| Split | la hoja se convierte en rama con dos hijos | O(N) tiempo, O(1) espacio |
| Cerrar | quitar hoja; si la rama queda con un hijo, colapsarla | O(N) |
| Resize | ajustar flex ratios de los hermanos | O(hermanos) |
| Navegar ↑↓←→ | heurística: primer o último hijo según dirección | O(N) |

⚠️ **El aviso que da Warp**: lo difícil no es el árbol, es la **navegación con
flechas**. La jerarquía del árbol no coincide con la intuición espacial del
usuario. Su solución fue una heurística de enfocar el primer/último hijo según la
dirección del movimiento. Presupuesta tiempo para esto; parece trivial y no lo es.

**Recomendación fuerte:** construye y depura el árbol con **rectángulos de color**,
sin terminales reales dentro. Es el paso 2 del plan de la sección 6.

---

## 3. Las reglas que preservan el rendimiento

Estas salen directamente del código de foot (`docs/01`). Son las que evitan
convertirse en lo que te molesta de Warp.

| # | Regla | Por qué | De dónde sale |
|---|---|---|---|
| 1 | **Celda ≤ 16 bytes, con assert en compilación** | Es el presupuesto de RAM del scrollback. 12 B × 200 col × 10k líneas = 24 MB. A 32 B serían 64 MB — por pane. | `terminal.h:36`, `static_assert` |
| 2 | **Debounce con techo antes de renderizar** | ~0,5 ms reiniciable + ~8 ms de techo. Renderizar por `read()` es el error clásico y la causa de la mayoría del lag percibido. | `terminal.c`, `config.c:3656` |
| 3 | **Grid = ring buffer potencia de 2** | Scroll es `offset++`, sin mover memoria. `&` en vez de `%`. | `grid.h` |
| 4 | **Paralelizar por filas, buffer por worker** | Cero locks durante el dibujado. El mutex solo protege la cola. | `render.c:2183` |
| 5 | **Damage tracking por celda, unión al final** | Solo se recompone lo que cambió. Un pane inactivo cuesta 0. | `shm.h` |
| 6 | **Filas perezosas** | Scrollback de 10k líneas ≠ 10k filas reservadas. | `grid.h` |
| 7 | **Acotar trabajo por vuelta del bucle** | 10 lecturas × 24 KB y ceder. Sin esto un pane ruidoso congela los demás — crítico con splits. | `terminal.c` |
| 8 | **Una sola superficie para todos los panes** | Nunca una ventana/surface hija por pane. Ahí se muere en composición. | diseño |
| 9 | **Techo explícito al estado por pane** | Scrollback acotado y podado. **Esta es la regla que Warp parece no aplicar.** | — |
| 10 | **Idle = 0% CPU** | Bucle de eventos, no bucle de frames. Sin polling. | `fdm.c` |

La regla 9 merece énfasis: es la que ataca directamente tu queja. Todo lo que se
acumule por pane sin techo (scrollback, historial de comandos, "blocks", metadatos)
degrada el rendimiento monótonamente con el tiempo de sesión. **Todo estado por
pane necesita un límite superior definido, y hay que medir su coste.**

---

## 4. GPU o CPU: decidir a conciencia

No es obvio, y foot demuestra que la respuesta "obvia" está mal.

| | CPU (pixman/Direct2D a bitmap) | GPU (wgpu/D3D/Metal) |
|---|---|---|
| Arranque | **~70 ms** (no toca el driver) | +decenas de ms por contexto |
| Uso de RAM | menor | mayor (atlas en VRAM + buffers) |
| Scroll masivo | peor | **mejor** |
| Texto normal, escribir, moverse | **excelente** | excelente |
| Complejidad | menor | mayor |
| Portabilidad | alta | `wgpu` abstrae los 3 SO |

Los benchmarks oficiales de foot (ver `docs/04`) muestran que foot **gana con
holgura** en movimiento de cursor, celdas densas y unicode, y **pierde ligeramente**
contra Alacritty en scroll. O sea: la GPU te compra scroll masivo y poco más.

**Recomendación:** CPU primero. Es más simple, arranca antes, gasta menos RAM —tus
dos prioridades declaradas— y si luego el scroll te molesta, el renderer es
sustituible *si respetaste la separación de capas*. Con `wgpu` además tendrías las
3 plataformas de una sola vez, así que también es una salida razonable si eliges
GPU desde el principio.

---

## 5. Stack

### Recomendación: Rust

| Pieza | Elección | Por qué |
|---|---|---|
| Core VT + grid | **`alacritty_terminal`** (crate) | Probado en producción años. Ya separado del UI — exactamente la capa 1. |
| PTY | **`portable-pty`** (de WezTerm) | Envuelve ConPTY (Windows), `openpty` (Unix). Resuelve la capa menos interesante. |
| Ventana/input | `winit`, o nativo por plataforma | `winit` = 3 plataformas gratis, sensación menos nativa. |
| Render | `wgpu` (GPU) o `tiny-skia`/Direct2D (CPU) | Ver sección 4. |
| Fuentes | `swash` / `cosmic-text` | Shaping + caché. Análogo a `fcft`. |
| Layout tree | **propio**, ~800 líneas | Es tu proyecto. Sigue el post de Warp. |

**Por qué Rust y no C como foot:** el ecosistema ya tiene resueltas las piezas que
no quieres escribir, es memory-safe en código con hilos (y vas a tener hilos de
render), y compila a las tres plataformas sin drama. foot es C porque su autor
empezó en 2019 y quería control absoluto; tú no tienes esa restricción.

### Alternativa: C/Zig + `libghostty-vt`

Te regala el 45% más difícil: parser, estado, encoding de input, scrollback, line
wrapping, **resize reflow**, snapshots, parsers OSC/SGR.

Y el dato clave: **`libghostty-vt` ya funciona en macOS, Linux, Windows y WASM**,
aunque la app Ghostty no tenga Windows.

⚠️ Textual de su documentación: *"This library is currently in development and the
API is not yet stable. Breaking changes are expected in future versions."* Hoy la
usa sobre todo la app de macOS. Si la adoptas ahora, estás firmando por refactors
periódicos. Vigila [ghostling](https://github.com/ghostty-org/ghostling), que es
una terminal mínima construida sobre ella — literalmente un punto de partida.

### Descartado como referencia de Windows

**Ghostty la app**: solo macOS y Linux; en Windows únicamente vía WSL. Como
referencia de arquitectura y como fuente de `libghostty-vt`, excelente. Como
plantilla de Windows, no sirve.

---

## 6. Plan por fases

Ordenado para que **lo más arriesgado se descubra primero** y para tener algo
usable lo antes posible.

### Fase 0 — Corte vertical (1-2 semanas)
ConPTY + `alacritty_terminal` + render de un grid en pantalla, sin config, sin
temas, sin nada. Que se vea un `dir` funcionando.
**Objetivo real:** validar el stack antes de invertir. Si algo va a salir mal, sale
mal aquí y has perdido dos semanas, no seis meses.

### Fase 1 — Un pane usable a diario (1-2 meses)
Colores, selección, portapapeles, scrollback, resize con reflow, teclado completo.
**Criterio de salida: puedes usarla como tu terminal principal.** Este es el hito
psicológico que decide si el proyecto sobrevive; la mayoría muere antes.

### Fase 2 — El árbol de panes con rectángulos falsos (2-3 semanas)
Sin terminales reales. Splits, cierre, resize, navegación direccional. Depuras la
heurística de navegación en aislamiento, que es la parte que Warp señala como
difícil.

### Fase 3 — Unir 1 y 2 (2-4 semanas)
Aquí ya tienes tu terminal: multi-pane, ligera, rápida. **Fin del MVP.**

### Fase 4 — Endurecer (continuo)
Tabs, config, temas, búsqueda en scrollback, ligaduras, IME.

### Fase 5 — Segunda plataforma (opcional, decidir en el mes 6)
Solo la capa 3 se reescribe. Si las capas se respetaron, esto es aditivo.
**Decides esto con un producto funcionando en la mano, no antes.**

---

## 7. Presupuesto de rendimiento (criterios de aceptación)

Números concretos contra los que medir, derivados de las mediciones en `docs/04`:

| Métrica | Objetivo | Referencia |
|---|---|---|
| Arranque en frío, 1 pane | < 150 ms | foot: 70-160 ms |
| Abrir un pane nuevo | **< 50 ms, y constante con el nº de panes** | footclient: 30-40 ms |
| RAM, 1 pane, scrollback 10k | < 40 MB | foot: 17,6 MB |
| RAM, 10 panes | **< 150 MB** | Warp en disco: 191 MB |
| CPU en reposo | **0,0%** | foot: 0% |
| Latencia de tecla a píxel | < 1 frame + ~8 ms | debounce de foot |

**La métrica que define el proyecto es la segunda: constante con el número de
panes.** Es exactamente lo que Warp incumple y lo que te trajo aquí. Mídela desde
la fase 3, con un test automatizado, y trátala como regresión bloqueante.

---

## 8. Decisiones de scope: qué NO va a hacer

La lección más valiosa de foot no es técnica. Rellena esto **antes** de escribir
código, y defiéndelo:

| Decisión | Propuesta | Razón |
|---|---|---|
| ¿Modelo de *blocks* tipo Warp? | **No, al menos no en v1** | Es el rasgo distintivo real de Warp, y el principal sospechoso de su degradación. Es otro proyecto. |
| ¿IA integrada? | **No** | Fuera del objetivo "ligera". |
| ¿Ligaduras? | No en v1 | foot tardó años. |
| ¿Sixel / imágenes? | No en v1 | 2.228 líneas en foot. |
| ¿Multiplataforma? | **Una plataforma en v1** | Ver sección 0. |
| ¿Tabs además de splits? | Sí, fase 4 | Sale casi gratis del mismo árbol. |
| ¿Multiplexado remoto (tipo tmux)? | No | Cambia la arquitectura entera. |

Cada "sí" en esta tabla se paga en meses. Cada "no" es la razón de que foot arranque
en 70 ms.
