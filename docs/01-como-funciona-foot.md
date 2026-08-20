# Cómo funciona foot por dentro

> Todo lo de este documento está verificado contra el código fuente en
> `upstream/foot/` (commit `85655c7`). Las referencias son `archivo:línea` reales;
> se pueden abrir directamente.

---

## 0. El mapa: 53.000 líneas de C, y dónde está cada cosa

```
render.c        5471   Render: hilos worker, damage, composición con pixman
terminal.c      4852   El objeto terminal: estado, PTY, resize, ciclo de vida
config.c        4378   Parseo de foot.ini
input.c         3687   Teclado, ratón, keybindings, codificación de teclas
box-drawing.c   3450   Dibujo vectorial de caracteres de caja (no usa la fuente)
selection.c     2940   Selección de texto
wayland.c       2864   Protocolo Wayland: superficies, seats, outputs
csi.c           2271   Secuencias CSI  (ESC [ ...)
sixel.c         2228   Protocolo de imágenes Sixel
osc.c           1735   Secuencias OSC  (ESC ] ...) — títulos, colores, URLs
grid.c          1676   Ring buffer del grid, resize con reflow
search.c        1512   Búsqueda en scrollback
vt.c            1133   LA MÁQUINA DE ESTADOS del parser
shm.c           1109   Buffers de memoria compartida (memfd + mmap)
main.c           727   Arranque
server.c         690   Modo servidor
key-binding.c    661   Resolución de atajos
client.c         605   El binario footclient
slave.c          573   fork + exec del shell hijo
fdm.c            496   EL BUCLE DE EVENTOS (epoll)
```

Dos archivos mandan sobre la arquitectura entera: **`fdm.c`** (cómo se organiza el
tiempo) y **`vt.c`** (cómo se interpretan los bytes). Todo lo demás cuelga de ahí.

---

## 1. El bucle de eventos: `fdm` (File Descriptor Multiplexer)

foot **no tiene un bucle de render**. No hay un `while(true) { render(); }` a 60 fps.
Tiene un bucle de *espera* sobre `epoll` que solo despierta cuando algo pasa.

`upstream/foot/fdm.h`:

```c
struct fdm *fdm_init(void);
bool fdm_add(struct fdm *fdm, int fd, int events, fdm_fd_handler_t handler, void *data);
bool fdm_signal_add(struct fdm *fdm, int signo, fdm_signal_handler_t handler, void *data);
bool fdm_hook_add(struct fdm *fdm, fdm_hook_t hook, void *data, enum fdm_hook_priority priority);
bool fdm_poll(struct fdm *fdm);
```

`upstream/foot/fdm.c:88`:

```c
struct fdm {
    int epoll_fd;
    bool is_polling;
    tll(struct fd_handler *) fds;
    tll(struct fd_handler *) deferred_delete;
    sigset_t sigmask;
    struct sig_handler *signal_handlers;
    ...
};
```

Todo en foot es un descriptor de archivo registrado aquí:

- el **PTY** del shell (`ptmx`)
- el socket de **Wayland**
- el socket del **servidor** (en modo daemon)
- los **timers** (`timerfd`): parpadeo del cursor, render diferido
- las **señales** (`signalfd`)

Un solo hilo, `epoll_wait`, cero polling activo. **Si no pasa nada, foot consume
exactamente 0% de CPU.** Esto suena obvio y es la mitad de por qué se siente ligera:
muchas terminales con render GPU mantienen un bucle de frames vivo.

El detalle de los *hooks* con prioridad (`FDM_HOOK_PRIORITY_HIGH/NORMAL/LOW`) es
cómo encaja el render: se ejecuta *después* de procesar todos los FDs listos en esa
vuelta del bucle, no en medio.

---

## 2. La celda: 12 bytes, y por qué eso importa tanto

Esta es la decisión de diseño más importante del proyecto, y el propio código lo
explica. `upstream/foot/terminal.h:36`:

```c
/*
 *  Note: we want the cells to be as small as possible. Larger cells
 *  means fewer scrollback lines (or performance drops due to cache
 *  misses)
 *
 * Note that the members are laid out optimized for x86
 */
struct attributes {
    bool bold:1;
    bool dim:1;
    bool italic:1;
    bool underline:1;
    bool strikethrough:1;
    bool blink:1;
    bool conceal:1;
    bool reverse:1;
    uint32_t fg:24;

    bool clean:1;
    enum color_source fg_src:2;
    enum color_source bg_src:2;
    bool confined:1;
    bool selected:1;
    bool url:1;
    uint32_t bg:24;
};
static_assert(sizeof(struct attributes) == 8, "VT attribute struct too large");

struct cell {
    char32_t wc;
    struct attributes attrs;
};
static_assert(sizeof(struct cell) == 12, "bad size");
```

Léelo con calma, porque está lleno de oficio:

- **Bitfields empaquetados a mano.** 8 flags booleanos + color de 24 bits caben
  *exactos* en 4 bytes. Dos de esos = 8 bytes de atributos.
- **`static_assert` en tiempo de compilación.** Si alguien añade un flag y rompe
  el tamaño, **el build falla**. El tamaño de la celda es un contrato, no una
  aspiración.
- **"laid out optimized for x86"** — el orden de los campos no es arbitrario.
- **12 bytes por celda.** Un scrollback de 10.000 líneas × 200 columnas = 24 MB.
  Si la celda fuera de 32 bytes serían 64 MB. Ahí se decide si tu terminal es
  ligera o no; no en el renderer.

El razonamiento del comentario es lo importante: celdas pequeñas → más líneas de
scrollback caben en caché → menos *cache misses* al recorrer el grid → render más
rápido. **El rendimiento se gana en el layout de memoria, no en el dibujado.**

### Lo raro va aparte

`upstream/foot/terminal.h`:

```c
struct row {
    struct cell *cells;
    struct row_data *extra;      /* NULL en el caso normal */
    bool dirty;
    bool linebreak;
    struct {
        bool prompt_marker;
        int cmd_start;
        int cmd_end;
    } shell_integration;
};
```

Los hipervínculos OSC-8 y los subrayados de colores son **raros**. En vez de
inflar cada celda con esos campos, viven en `row->extra`, que normalmente es
`NULL`. Y se libera con un fast path explícito:

```c
static inline void
grid_row_reset_extra(struct row *row)
{
    struct row_data *extra = row->extra;
    if (likely(extra == NULL))
        return;
    ...
}
```

Ese `likely()` es una pista al compilador: el caso común es que no haya nada extra.

---

## 3. El grid: un ring buffer con máscara de bits

`upstream/foot/terminal.h`:

```c
struct grid {
    int num_rows;      /* SIEMPRE potencia de 2 */
    int num_cols;
    int offset;        /* dónde empieza la pantalla dentro del anillo */
    int view;          /* dónde está mirando el usuario (scrollback) */
    struct cursor cursor;
    struct cursor saved_cursor;
    struct attributes saved_attrs;
    struct row **rows;
    struct row *cur_row;
    tll(struct damage) scroll_damage;
    tll(struct sixel) sixel_images;
    ...
};
```

Y el acceso, en `upstream/foot/grid.h`:

```c
static inline int
grid_row_absolute(const struct grid *grid, int row_no)
{
    return (grid->offset + row_no) & (grid->num_rows - 1);
}
```

Tres cosas en una línea:

1. **`&` en vez de `%`.** Como `num_rows` es potencia de dos, el módulo se
   convierte en un AND de bits. Esta función se llama millones de veces por
   segundo; ahorrarse una división entera ahí es significativo.
2. **Hacer scroll es `offset++`.** No se mueve ni un byte de memoria. Todo el
   scroll del terminal es aritmética sobre un índice.
3. **`view` es independiente de `offset`.** El usuario puede estar mirando
   scrollback mientras el shell sigue escribiendo abajo, sin copiar nada.

### Filas perezosas

`upstream/foot/grid.h`:

```c
static inline struct row *
_grid_row_maybe_alloc(struct grid *grid, int row_no, bool alloc_if_null)
{
    int real_row = grid_row_absolute(grid, row_no);
    struct row *row = grid->rows[real_row];

    if (row == NULL && alloc_if_null) {
        row = grid_row_alloc(grid->num_cols, false);
        grid->rows[real_row] = row;
    }
    return row;
}
```

Un scrollback configurado a 10.000 líneas **no reserva 10.000 filas al arrancar**.
Reserva el array de punteros (barato) y va materializando filas conforme se
escriben. Por eso una ventana recién abierta ocupa ~17 MB y no ~40 MB, aunque el
scrollback esté configurado alto.

### `grid_resize_and_reflow`

`upstream/foot/grid.h` declara la función más difícil del proyecto:

```c
void grid_resize_and_reflow(
    struct grid *grid, const struct terminal *term, int new_rows, int new_cols,
    int old_screen_rows, int new_screen_rows,
    size_t tracking_points_count,
    struct coord *const _tracking_points[static tracking_points_count]);
```

Reflow = al cambiar el ancho de la ventana, las líneas largas se re-parten. Fíjate
en los `tracking_points`: hay coordenadas (el cursor, los extremos de la selección,
los resultados de búsqueda) que deben *seguir apuntando a lo mismo* después del
reflow. Es el tipo de detalle que no se te ocurre hasta que lo sufres, y es de las
razones más fuertes para no escribir tu propio motor de terminal desde cero.

---

## 4. El parser VT: 22 estados

`upstream/foot/vt.c:28`:

```c
enum state {
    STATE_GROUND,
    STATE_ESCAPE,
    STATE_ESCAPE_INTERMEDIATE,

    STATE_CSI_ENTRY,
    STATE_CSI_PARAM,
    STATE_CSI_INTERMEDIATE,
    STATE_CSI_IGNORE,

    STATE_OSC_STRING,

    STATE_DCS_ENTRY,
    STATE_DCS_PARAM,
    STATE_DCS_INTERMEDIATE,
    STATE_DCS_IGNORE,
    STATE_DCS_PASSTHROUGH,

    STATE_SOS_PM_APC_STRING,

    STATE_UTF8_21,
    STATE_UTF8_31,
    STATE_UTF8_32,
    STATE_UTF8_41,
    STATE_UTF8_42,
    STATE_UTF8_43,
};
```

Los primeros 16 son la máquina de estados canónica de **Paul Williams** para
parsers DEC ANSI/VT — es *el* diagrama que sigue todo el mundo (Alacritty, Kitty,
Ghostty, WezTerm). No lo inventes: impleméntalo.

Lo propio de foot son los 6 últimos: la decodificación **UTF-8 integrada en la
misma máquina de estados**. `STATE_UTF8_31` = "estoy en el byte 1 de 3 de una
secuencia UTF-8". En vez de decodificar UTF-8 en una pasada previa y luego parsear
VT, hace ambos en un solo recorrido de los bytes. Una pasada, no dos.

El despacho está repartido por tipo de secuencia:

- `csi.c` (2271 líneas) — `ESC [` : movimiento de cursor, colores SGR, modos
- `osc.c` (1735 líneas) — `ESC ]` : título de ventana, portapapeles, URLs, paleta
- `dcs.c` (533 líneas) — `ESC P` : Sixel, XTGETTCAP

Esas ~4.500 líneas son solo el despacho de secuencias. Es el trabajo que
`libghostty-vt` te regalaría.

---

## 5. El render: paralelismo por filas

Aquí está lo bonito. `upstream/foot/render.c:2183`:

```c
static int
render_worker_thread(void *_ctx)
{
    struct render_worker_context *ctx = _ctx;
    struct terminal *term = ctx->term;
    const int my_id = ctx->my_id;
    free(ctx);

    sigset_t mask;
    sigfillset(&mask);
    pthread_sigmask(SIG_SETMASK, &mask, NULL);   /* los workers NO reciben señales */

    sem_t *start = &term->render.workers.start;
    sem_t *done  = &term->render.workers.done;
    mtx_t *lock  = &term->render.workers.lock;

    while (true) {
        sem_wait(start);
        struct buffer *buf = term->render.workers.buf;
        bool frame_done = false;

        while (!frame_done) {
            mtx_lock(lock);
            int row_no = tll_pop_front(term->render.workers.queue);
            mtx_unlock(lock);

            switch (row_no) {
            default: {
                struct row *row = grid_row_in_view(term->grid, row_no);
                int cursor_col = cursor.row == row_no ? cursor.col : -1;
                render_row(term, buf->pix[my_id], &buf->dirty[my_id],
                           row, row_no, cursor_col);
                break;
            }
            case -1:
                frame_done = true;
                sem_post(done);
                break;
            ...
            }
        }
    }
}
```

Y el lado del productor, `upstream/foot/render.c:3556`:

```c
if (term->render.workers.count > 0) {
    mtx_lock(&term->render.workers.lock);
    term->render.workers.buf = buf;
    for (size_t i = 0; i < term->render.workers.count; i++)
        sem_post(&term->render.workers.start);        /* ¡arrancad! */
    ...
}
/* se encolan los números de fila sucios */
tll_push_back(term->render.workers.queue, r);
...
/* Signal workers the frame is done */
for (size_t i = 0; i < term->render.workers.count; i++)
    tll_push_back(term->render.workers.queue, -1);    /* centinela, uno por worker */
mtx_unlock(&term->render.workers.lock);

for (size_t i = 0; i < term->render.workers.count; i++)
    sem_wait(&term->render.workers.done);             /* barrera de fin de frame */
```

El diseño, en tres ideas:

1. **La unidad de trabajo es una fila**, y la cola contiene simples `int`. Nada de
   estructuras, nada de asignaciones por tarea.
2. **`-1` es el centinela de "frame terminado"**, y se encola una vez por worker.
   Elegante: no hace falta un flag aparte ni un broadcast; cada worker consume su
   propio "fin".
3. **Cada worker escribe en `buf->pix[my_id]` y `buf->dirty[my_id]`** — su *propia*
   imagen pixman y su *propia* región de daño. **Durante el render real no hay
   ningún lock.** El mutex solo protege sacar un número de la cola.

El header lo confirma, `upstream/foot/shm.h`:

```c
struct buffer {
    ...
    pixman_image_t **pix;      /* una por worker */
    size_t pix_instances;

    /*
     * First item in the array is used to track frame-to-frame damage...
     * The remaining items are used to track surface damage. Each
     * worker thread adds its own cell damage to "its" region. When
     * the frame is done, all damage is converted to a single region,
     * which is then used in calls to wl_surface_damage_buffer().
     */
    pixman_region32_t *dirty;
};
```

Al final del frame se unen todas las regiones de daño en una sola y se le pasa al
compositor. **El compositor solo recompone los píxeles que cambiaron.**

---

## 6. El truco de rendimiento más importante: el render diferido

Esto es lo que separa una terminal que se siente instantánea de una que parpadea.
`upstream/foot/terminal.c`, en el handler del PTY:

```c
uint8_t buf[24 * 1024];
const size_t max_iterations = !hup ? 10 : SIZE_MAX;

for (size_t i = 0; i < max_iterations && pollin; i++) {
    ssize_t count = read(term->ptmx, buf, sizeof(buf));
    if (count < 0) {
        if (errno == EAGAIN || errno == EIO)
            break;
        ...
    } else if (count == 0)
        break;

    vt_from_slave(term, buf, count);      /* solo actualiza el grid en memoria */
}
```

Primer detalle: **hasta 10 lecturas de 24 KB por despertar de epoll**, y luego se
cede. Un `cat` de un archivo gigante no puede monopolizar el bucle de eventos —
importantísimo en modo servidor, donde ese bucle es compartido.

Segundo detalle, y el comentario del propio código lo explica mejor que yo:

```c
/*
 * We likely need to re-render. But, we don't want to do it
 * immediately. Often, a single client update is done through
 * multiple writes. This could lead to us rendering one frame with
 * "intermediate" state.
 *
 * For example, we might end up rendering a frame where the client
 * just erased a line, while in the next frame, the client wrote to
 * the same line. This causes screen "flickering".
 *
 * Mitigate by always incuring a small delay before rendering the
 * next frame. ...
 *
 * We incur this delay *every* time we receive input. To ensure we
 * don't delay rendering indefinitely, we start a second timer that
 * is only reset when we render.
 */
```

Es un **debounce con techo**, implementado con dos `timerfd`:

| Timer | Valor por defecto | Se reinicia | Función |
|---|---|---|---|
| `lower_fd` | **0,5 ms** (`config.c:3656`) | en **cada** llegada de datos | Espera a que el cliente termine de escribir. Evita renderizar estado intermedio. |
| `upper_fd` | **8,33 ms** = medio frame a 60 Hz (`config.c:3657`) | **solo al renderizar** | Techo absoluto. Garantiza que nunca se retrase el render indefinidamente. |

Traducido: *"cuando lleguen datos, espera 0,5 ms por si viene más; pero pase lo que
pase, renderiza como mucho 8,33 ms después del primer dato"*.

Y remata el comentario:

```c
/*
 * Note that when the client is producing data at a very high pace,
 * we're rate limited by the wayland compositor anyway. The delay we
 * introduce here only has any effect when the renderer is idle.
 */
```

Cuando el output es masivo, quien impone el ritmo es el `frame_callback` de Wayland
(`render.c:3678`): `wl_surface_frame()` hace que el compositor avise cuándo tiene
sentido pintar. **foot nunca renderiza más rápido que la pantalla.**

Este es el mecanismo que hay que copiar sí o sí. La mayoría del "lag" percibido en
terminales mal hechas es renderizar una vez por `read()`.

---

## 7. Buffers: memoria compartida, no GPU

`upstream/foot/shm.c` — el pipeline es:

1. `memfd_create()` — un "archivo" en RAM
2. `mmap()` — mapearlo en el proceso; los workers pintan ahí con pixman
3. `wl_shm_pool_create_buffer()` — pasarle el mismo fd al compositor
4. El compositor lo mapea también → **cero copias**

```c
static off_t max_pool_size = 512 * 1024 * 1024;
```

Hay además un caché de buffers con edad (`unsigned age` en `struct buffer`), porque
algunos compositores no liberan el buffer de inmediato y hay que hacer doble
buffering. El comentario en `shm.h` sobre "re-applying damage from the last frame"
es exactamente ese caso.

**No hay OpenGL. No hay Vulkan. No hay contexto gráfico.** De ahí viene el arranque
en 70 ms: crear un contexto GPU cuesta más que todo lo que hace foot al arrancar.

### La consecuencia de no tener GPU

Como no puede pedirle a la GPU que dibuje formas, foot **dibuja los caracteres de
caja a mano**: `box-drawing.c`, 3.450 líneas de geometría con pixman para ┌─┬─┐ y
los bloques de sombreado. La ventaja: salen perfectos a cualquier tamaño y sin
huecos entre celdas, cosa que con fuente casi nunca ocurre.

---

## 8. Las fuentes: `fcft`

Está clonada en `upstream/fcft/`. Es una librería aparte del mismo autor, escrita
específicamente porque **Cairo le resultó demasiado lento para foot**.

Envuelve FontConfig + FreeType + HarfBuzz y devuelve los glifos ya rasterizados
**como imágenes pixman**, que es exactamente lo que el renderer necesita — sin
conversión intermedia. Cachea agresivamente y soporta lookup desde varios hilos, que
es lo que permite a los workers de `render.c` trabajar en paralelo sin pelearse.

---

## 9. El modo servidor

`upstream/foot/server.c:481` — un socket UNIX normal:

```c
int fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC | SOCK_NONBLOCK, 0);
...
int client_fd = accept4(server->fd, ..., SOCK_CLOEXEC | SOCK_NONBLOCK);
```

`footclient` (605 líneas, binario de 27 KB) no es una terminal: es un **mensajero**.
Serializa `argv`, el `cwd`, el entorno y las opciones, los manda por el socket, y el
servidor llama a `term_init()` (`server.c:391`) creando una ventana más *dentro de
su propio proceso*.

Lo que se comparte entre todas las ventanas:
- el caché de glifos de fcft (la ganancia grande)
- las fuentes cargadas
- la conexión Wayland
- el bucle de eventos `fdm`

Lo que **no** se comparte — y esto lo tenía yo mal antes, el README lo corrige:

> The downside is a performance penalty; all windows' input and output
> are multiplexed in the same thread (**but each window will have its own
> set of rendering threads**). This means that if one window is very busy
> with, for example, producing output, then other windows will suffer.

O sea: el **I/O** es compartido (un solo `fdm`), pero **cada ventana tiene sus
propios hilos de render**. El cuello de botella potencial está en el parseo, no en
el dibujado. Y el riesgo real:

> And of course, should the server process crash, **all** windows will be gone.

Además soporta **activación por socket** de systemd: `foot --server` se arranca solo
la primera vez que se ejecuta `footclient`.

**Este es el plano de tu arquitectura multi-pane.** Un proceso, N terminales, caché
compartido. Solo falta el árbol de layout.

---

## 10. Dependencias: 12 en total

`upstream/foot/meson.build:146`:

```
threads  epoll-shim  pixman-1  wayland-protocols  wayland-client
wayland-cursor  xkbcommon  fontconfig  libutf8proc  tllist  fcft
wayland-scanner
```

Ni GTK, ni Qt, ni Cairo, ni Pango, ni glib directo. Comparado con GNOME Terminal
—que arranca GTK entero, lee temas, hace roundtrips a dconf y se registra en D-Bus
antes de mostrar un carácter— foot no hace nada de eso.

(`tllist` es una lista enlazada *typed* en macros, también de dnkl.)

---

## 11. El build con PGO

`upstream/foot/pgo/pgo.sh` — el proceso es: compilar instrumentado, ejecutar cargas
sintéticas (`pgo.c` genera output de terminal representativo), recompilar usando el
perfil.

```sh
CFLAGS="${CFLAGS-} -O3"

case $(${CC-cc} --version) in
    *Free\ Software\ Foundation*)
        compiler=gcc
        do_pgo=yes
        ;;
    *clang*)
        compiler=clang
        ...
```

Hay modos `full-headless-sway.sh` y `full-headless-cage.sh`: arrancan un compositor
Wayland **sin pantalla** para poder generar el perfil en un servidor de CI. Ese es
el nivel de compromiso con el rendimiento del proyecto.

Tu binario (`foot 1.16.2 +pgo`) está compilado así.

---

## 12. Las 8 lecciones transferibles

Resumen de lo que aquí importa para construir otra terminal, en orden de impacto:

1. **El tamaño de la celda es el presupuesto de rendimiento.** 12 bytes, con
   `static_assert` que rompe el build si crece. Empaqueta a bitfields. Manda lo
   raro a un `extra` que normalmente sea `NULL`.
2. **Debounce con techo antes de renderizar.** 0,5 ms de espera reiniciable + 8,33 ms
   de techo. Renderizar por `read()` es el error clásico.
3. **Ring buffer con máscara de bits para el grid.** Potencia de dos, `&` en vez de
   `%`, scroll = `offset++`. Nunca muevas memoria para hacer scroll.
4. **Paraleliza por filas, no por regiones.** Cola de `int`, un buffer de salida por
   worker, cero locks durante el dibujado, centinela `-1` por worker para cerrar el
   frame.
5. **Damage tracking real.** Región de daño por worker, unión al final, y solo eso
   se manda a componer.
6. **Filas perezosas.** Reserva punteros, materializa filas al escribirse.
7. **Acota el trabajo por vuelta del bucle.** 10 lecturas × 24 KB y cede. Sin esto,
   un pane ruidoso congela a los demás.
8. **Un solo bucle de eventos para todo** — PTY, ventana, timers, señales, socket.
   Sin bucle de frames, sin polling. Idle = 0% CPU.

Y la que no es de código: **foot es rápida porque hace poco**. Sin X11, sin tabs,
sin splits, sin ligaduras. El scope recortado *es* la característica.
