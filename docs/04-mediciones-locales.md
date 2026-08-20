# Mediciones

## Entorno

```
foot     1.16.2  +pgo +ime +graphemes -assertions   (paquete Ubuntu 1.16.2-2ubuntu0.1)
niri     26.04
Medido   2026-08-19
```

## Arranque (medido en esta máquina)

`/usr/bin/time` sobre un ciclo completo: arrancar, abrir ventana, ejecutar y salir.

```sh
foot -e true         →  0,13 s  |  0,07 s  |  0,16 s     (~12 MB RSS pico)
footclient -e true   →  0,04 s  |  0,03 s  |  0,03 s     (~1,2 MB RSS pico)
```

**`footclient` es 2-4× más rápido** porque el proceso servidor ya tiene las fuentes
cargadas y el caché de glifos caliente. El cliente solo serializa argv/cwd/entorno
por un socket UNIX (ver `docs/01`, sección 9).

## Memoria residente

```
foot --server (PID del servidor)   7,4 MB
ventana de foot                   17,6 MB
ventana de foot                   17,7 MB
```

## Tamaño en disco

```
/usr/bin/foot                                412 KB
/usr/bin/footclient                           27 KB
/opt/warpdotdev/warp-terminal                191 MB     ← ~460× el binario de foot
```

## Dependencias dinámicas

23 librerías compartidas, todas de bajo nivel:

```
libwayland-client  libwayland-cursor  libpixman-1  libfcft  libfreetype
libharfbuzz  libfontconfig  libxkbcommon  libutf8proc  libpng16  libz
libbz2  libbrotlidec  libgraphite2  libexpat  libffi  libglib-2.0  libpcre2-8
libm  libc  ld-linux  linux-vdso  libbrotlicommon
```

**Sin GTK, sin Qt, sin Cairo, sin Pango, sin Chromium.** `meson.build` declara 12
dependencias directas.

---

## Benchmarks oficiales de foot

De `upstream/foot/doc/benchmark.md`, con [vtebench](https://github.com/alacritty/vtebench).
Sistema: i9-9900, 64 GB RAM, RX 5500XT. Geometría 2040×1884, scrollback 10.000.
Fecha: 2022-05-12.

| Benchmark (ms, menos es mejor) | foot GCC+PGO 1.12.1 | foot 1.12.1 | Alacritty 0.10.1 | URxvt 9.26 | XTerm 372 |
|---|---:|---:|---:|---:|---:|
| cursor motion | **10,40** | 14,07 | 24,97 | 23,38 | 1622,86 |
| dense cells | **29,58** | 45,46 | 97,45 | 10828,00 | 2323,00 |
| light cells | **4,34** | 4,40 | 12,84 | 12,17 | 49,81 |
| scrolling | 135,31 | 116,35 | 121,69 | **108,30** | 4041,33 |
| scrolling bottom region | 118,19 | 109,70 | **105,26** | 118,80 | 3875,00 |
| scrolling bottom small region | 132,41 | **122,11** | 122,83 | 151,30 | 3839,67 |
| scrolling fullscreen | 5,70 | **5,66** | 10,92 | 12,09 | 124,25 |
| scrolling top region | 144,19 | **121,78** | 135,81 | 159,24 | 3858,33 |
| scrolling top small region | 135,95 | 119,01 | **115,46** | 216,55 | 3872,67 |
| unicode | 11,56 | **10,92** | 15,94 | 1012,27 | 4779,33 |

### Lectura honesta de esta tabla

Dos matices que conviene no barrer bajo la alfombra, porque afectan al diseño:

1. **foot NO gana en todo.** Domina con holgura en movimiento de cursor (2,4×
   mejor que Alacritty), celdas densas (3,3×), celdas ligeras (3×) y unicode. Pero
   en **scroll pierde o empata** contra Alacritty y URxvt. Es coherente con no usar
   GPU: el scroll masivo es justo donde la GPU ayuda.

2. **El build con PGO no siempre gana.** Es claramente mejor en cursor motion
   (10,40 vs 14,07) y dense cells (29,58 vs 45,46 = 1,5×), pero **peor en casi
   todos los benchmarks de scroll** (135,31 vs 116,35). La afirmación de que
   "PGO+GCC es más del doble de rápido que clang" **no se sostiene en esta tabla**;
   la mejora real está entre 1,3× y 1,5×, y solo en las rutas de parseo/render de
   celdas.

**Consecuencia para el diseño:** si el objetivo son latencia de tecleo, arranque y
RAM, el enfoque CPU de foot es el correcto. Si el objetivo fuera scroll de logs
masivos, la GPU tiene un argumento real. Ver `docs/02`, sección 4.

---

## Hallazgo de configuración en esta máquina

`~/.config/niri/config.kdl:172-173`:

```kdl
Mod+Return hotkey-overlay-title="Terminal" { spawn "foot"; }
Mod+T      hotkey-overlay-title="Terminal" { spawn "foot"; }
```

Pero `foot-server.service` y `foot-server.socket` **están activos** (systemd los
levantó el 2026-08-18 19:36). Es decir: **el servidor está corriendo y no se está
usando.** Cada `Mod+Return` arranca un proceso completo e independiente en vez de
pedirle una ventana al servidor que ya tiene las fuentes cargadas.

**Cambio propuesto:** `spawn "footclient"` en ambos binds.

- Ganancia: **~2-4× más rápido** al abrir (de ~100 ms a ~35 ms), y cada ventana
  comparte el caché de glifos en vez de duplicarlo (menos RAM).
- Coste, según el README de foot: en modo servidor el I/O de todas las ventanas se
  multiplexa en el mismo hilo (aunque **cada ventana conserva sus propios hilos de
  render**). Una ventana escupiendo output masivo puede afectar a las demás.
- Riesgo: *"should the server process crash, **all** windows will be gone."*

No aplicado — pendiente de decisión.
