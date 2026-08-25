# Comportamiento, ergonomía y accesibilidad

**Qué resuelve este documento:** la lista de decisiones de *comportamiento* que hay
que cerrar antes de escribir código, en el mismo espíritu que la sección 8 de
`docs/02` ("rellena esto **antes**, y defiéndelo").

Elaborado el 2026-08-20. Las mediciones de PSReadLine son de esta máquina.

---

## 0. La distinción que ordena todo lo demás

Antes de listar features hay que separar tres cosas que se confunden
constantemente, porque **la mayoría de lo que uno pide a "la terminal" no lo hace
la terminal**:

```
┌─ TÚ escribes ───────────────────────────────────────────────┐
│  teclas  →  [ TERMINAL ]  →  bytes  →  [ ConPTY ]  →  shell  │
│                                                              │
│  La terminal decide: QUÉ BYTES manda.                        │
│  El shell decide:    QUÉ HACE con esos bytes.                │
└──────────────────────────────────────────────────────────────┘
```

| Capa | De quién es el trabajo | Ejemplos |
|---|---|---|
| **A. Emulador (tu código)** | **tuyo, hay que decidirlo** | Selección y portapapeles, encoding de teclas, scrollback, zoom, colores, cursor, campana, splits |
| **B. Shell (PSReadLine, bash, fish)** | **de otro — solo hay que no estorbar** | Historial de comandos, Ctrl+R, autocompletado, Ctrl+Backspace, edición de línea |
| **C. Sistema operativo** | del SO, con enganches tuyos | Lector de pantalla (UIA), alto contraste, escalado DPI, IME |

Esta separación no es pedantería: **decide qué entra en el presupuesto de trabajo
y qué es gratis.** Dos de tus tres ejemplos caen en la capa B, o sea que ya
funcionan y no cuestan nada.

---

## 1. Tus tres preguntas, respondidas

### 1.1 Historial de comandos: ¿por sesión o global?

**No es una decisión tuya. Ya está tomada, y es global.** Medido aquí:

```
PSReadLine                       2.0.0   (Windows PowerShell 5.1; pwsh 7 no instalado)
HistorySaveStyle                 SaveIncrementally
HistorySavePath                  %APPDATA%\Microsoft\Windows\PowerShell\PSReadLine\
                                 ConsoleHost_history.txt
MaximumHistoryCount              4096
HistoryNoDuplicates              True
UpArrow                       →  PreviousHistory
Ctrl+r                        →  ReverseSearchHistory
```

Un **único archivo compartido por todas las sesiones**, escrito incrementalmente
(cada comando se persiste al ejecutarse, no al cerrar), con tope de 4096 entradas
y sin duplicados. Si quisieras historial por sesión, se cambia con
`Set-PSReadLineOption -HistorySaveStyle SaveNothing` en tu `$PROFILE` — una línea
de config, cero líneas de código en tu terminal.

Y hay un argumento de arquitectura para dejarlo así, no solo de pereza: la
**regla 9** de `docs/02` §3 nombra explícitamente el *"historial de comandos"*
como estado por pane que crece sin techo. Si el historial vive en el shell, tu
terminal transporta **cero** de ese estado: 10 panes son 10 clientes del mismo
archivo, no 10 historiales en tu RAM. Es exactamente lo que Warp hace mal con los
*blocks*.

> **Decisión: la terminal no implementa historial. Ni por sesión ni global.**
> Coste: 0. Riesgo de regresión de rendimiento: 0.

### 1.2 Ctrl+Backspace: ¿palabra o carácter?

**Palabra — y ya lo hace, en el shell.** Medido aquí:

```
Backspace        →  BackwardDeleteChar     (un carácter)
Ctrl+Backspace   →  BackwardKillWord       (una palabra)   ← ya está
Delete           →  DeleteChar
Ctrl+Delete      →  KillWord
```

Pero aquí sí hay una decisión tuya, y es más fina de lo que parece. **Tu terminal
no decide el significado; decide el encoding** — y el encoding heredado es
ambiguo:

| Tecla | Byte tradicional | Colisión |
|---|---|---|
| `Backspace` | `0x7F` (DEL) | — |
| `Ctrl+H` | `0x08` (BS) | — |
| `Ctrl+Backspace` | **no tiene byte propio** | indistinguible de `Ctrl+H` |

Ese es el problema real: en el protocolo VT clásico **no hay forma de expresar
"Ctrl + Backspace"**. Cada terminal inventa su parche (unos mandan `0x08`, otros
mandan `0x17` = `Ctrl+W` para disparar el kill-word de readline). Por eso esta
tecla se comporta distinto en cada terminal que has usado.

Hay dos soluciones modernas, y en Windows la interesante **no** es la famosa:

| Protocolo | Qué resuelve | Quién lo entiende |
|---|---|---|
| **`win32-input-mode`** (`CSI ?9001h`) | Pasa el evento Win32 **completo** por el PTY: virtual-key, scan code, modificadores, key-up | ConPTY / apps de consola Windows (PSReadLine) |
| **Kitty keyboard protocol** (`CSI > flags u`) | Desambigua todas las combinaciones, multiplataforma | kitty, foot, WezTerm, Ghostty, apps Unix modernas |

Como tu v1 es Windows y tu shell es PowerShell, **`win32-input-mode` es el que te
da la ganancia**; el de Kitty importa cuando quieras que herramientas de WSL/Unix
se porten bien, o cuando llegues a la fase 5.

> **Decisión: negociar `win32-input-mode` con ConPTY; tabla legacy como fallback;
> protocolo Kitty diferido a fase 4.**
> ⚠️ **Validar en la fase 0.** Cómo viaja exactamente `Ctrl+Backspace` desde tu
> ventana hasta PSReadLine atravesando ConPTY es justo el tipo de detalle que el
> corte vertical existe para descubrir. Si falla, quieres saberlo en la semana 2.

### 1.3 Copiar y pegar

**Esta sí es 100% tuya**, y es la única de las tres que cuesta trabajo real.

| Sub-decisión | Propuesta | Por qué |
|---|---|---|
| Atajo de copiar | `Ctrl+Shift+C`, **y** `Ctrl+C` si hay selección | `Ctrl+C` sin selección **debe** seguir mandando SIGINT. Romper eso es romper la terminal. |
| Atajo de pegar | `Ctrl+Shift+V` y `Ctrl+V` | En Windows `Ctrl+V` no colisiona con nada crítico. |
| Copia automática al seleccionar | **No** por defecto, opcional | Es idioma Unix; sorprende en Windows y pisa el portapapeles sin querer. |
| Selección rectangular | Sí, con `Alt` | Barata y muy útil para output en columnas. |
| Doble/triple clic | palabra / línea | Esperado. Separadores de palabra configurables. |
| **Bracketed paste** | **Sí, obligatorio** | Envuelve el pegado en `ESC[200~…ESC[201~` para que el shell sepa que es texto pegado y no lo ejecute. |
| **Guarda de pegado multilínea** | **Sí** | Si el texto pegado trae saltos de línea, confirmar antes. Es el mecanismo antipegado-accidental-de-`rm -rf`. |
| Pegar sin formato | siempre | Una terminal solo tiene texto plano. |

**Buena noticia de stack:** `alacritty_terminal` **ya trae el módulo `selection`**
(y también `vi_mode`). Módulos que expone hoy el crate:

```
event · event_loop · grid · index · selection · sync · term · thread · tty · vi_mode
```

O sea: el modelo de selección —con sus casos límite de wrapping y scroll— te lo
regalan. Lo que escribes tú es el puente al portapapeles de Win32 y los atajos.

---

## 2. Corrección al doc de diseño: falta la capa de input

Revisando `docs/02` aparece un hueco que conviene arreglar antes de la fase 0.

La tabla de esfuerzo de §0 acredita a `libghostty-vt` con *"encoding de input"*,
y luego §5 recomienda `alacritty_terminal`. **No son intercambiables en eso.**
La lista de módulos de arriba lo confirma: `alacritty_terminal` es la ruta de
**salida** (parser, grid, scrollback, tty). **No hay ningún módulo de teclado.**
En Alacritty, el mapeo tecla→bytes vive en el binario, no en el crate.

**Consecuencia práctica:**

| | Doc 02 asume | Realidad |
|---|---|---|
| Encoding de teclas | prestado | **código tuyo** |
| Tamaño | — | ~500-900 líneas, más una tabla de casos límite |
| Cuándo | — | **fase 0/1, no es opcional** |

No es una mala noticia grande —es la parte que decide cómo se siente escribir en
tu terminal, y probablemente quieras controlarla— pero **hay que presupuestarla**.
Sugerencia: añadir una fila "Encoding de input" a la tabla de §5 marcada como
*propio*, junto a "Layout tree".

---

## 3. Accesibilidad de verdad (capa C)

Aquí está lo que **no** se puede añadir después, y por eso va ahora aunque no lo
construyas en v1.

### 3.1 La única decisión arquitectónica urgente

Un lector de pantalla (NVDA, JAWS, Narrador) no lee píxeles: pide el texto por
**UI Automation**, con un `ITextProvider` que sepa devolver rangos, la posición
del cursor y **notificar cambios**. Windows Terminal tiene una implementación
sustancial de esto.

Implementar UIA en v1 sería un error de scope. Pero hay una decisión de **una
línea** que sí hay que tomar hoy:

> **La capa 1 debe exponer una vista de solo lectura, direccionable por rangos, del
> grid + scrollback, más notificaciones de cambio.**

El contrato de capa 1 en `docs/02` §1 dice *"recibe bytes, mantiene un grid, emite
eventos"* — eso describe solo la ruta de **escritura**. No dice nada sobre lectura
estructurada. Añadir ese requisito ahora es **casi gratis**; añadirlo en el año 2,
cuando la capa 2 lea el grid por punteros y offsets, es una refactorización.

Con esa API, UIA es aditivo. Sin ella, es una reescritura. Ese es el trato.

### 3.2 Las filas que faltaban en tu lista

Estas son las que interactúan con los **splits**, que son el punto entero del
proyecto:

| # | Comportamiento | Propuesta | Coste | Fase |
|---|---|---|---|---|
| 1 | **Todo operable sin ratón** | Splits, cierre, resize y navegación con teclado. Sin excepciones. | bajo | 2 |
| 2 | **Foco de pane no solo por color** | Borde + otra señal (grosor/atenuación de los demás). El color solo excluye a daltónicos. | bajo | 3 |
| 3 | **Zoom de fuente** `Ctrl +/-/0` | Re-layout, no escalado de bitmap. | medio | 1 |
| 4 | **Contraste mínimo garantizado** | Ajustar automáticamente colores indistinguibles del fondo. Los temas de 256 colores producen texto ilegible constantemente. | medio | 4 |
| 5 | **Parpadeo de cursor desactivable** | Y respetar la preferencia del SO de reducir animación. Es un problema vestibular y de fotosensibilidad real. | bajo | 1 |
| 6 | **Campana visual, no sonora, por defecto** | Configurable a: nada / visual / sonido. | bajo | 1 |
| 7 | **Escalado DPI correcto** | Per-monitor v2. Nada de texto borroso al mover entre monitores. | medio | 1 |
| 8 | **Confirmar cierre con proceso vivo** | Cerrar un pane con algo corriendo debe preguntar. Un `Ctrl+W` de más no debería matar un build. | bajo | 3 |
| 9 | **Navegación de scrollback por teclado** | `vi_mode` de `alacritty_terminal` sale casi gratis. | bajo | 1 |
| 10 | **Búsqueda en scrollback** | `Ctrl+Shift+F`, con resaltado. | medio | 4 |
| 11 | **Alto contraste del SO** | Detectar el modo y no pelearse con él. | bajo | 4 |
| 12 | **IME** | Escritura CJK/acentos con composición. Caro y aislable. | **alto** | 4 |

La regla que resume 1, 2, 5 y 8: **ninguna información importante debe transmitirse
solo por color, solo por sonido, o solo por movimiento.**

---

## 4. Tabla de decisiones (formato §8 de `docs/02`)

Rellenada, para defenderla:

| Decisión | Propuesta | Razón |
|---|---|---|
| ¿Historial de comandos propio? | **No, nunca** | Es del shell. Cero coste, y respeta la regla 9. |
| ¿Ctrl+Backspace borra palabra? | **Sí — vía encoding, no vía lógica** | El shell ya lo hace. Tú solo mandas los bytes correctos. |
| ¿`win32-input-mode`? | **Sí, v1** | Es la respuesta de Windows a la ambigüedad del teclado. Validar en fase 0. |
| ¿Protocolo de teclado Kitty? | Fase 4 | Importa para apps Unix/WSL, no para PowerShell. |
| ¿Bracketed paste? | **Sí, v1, no negociable** | Sin esto, pegar es peligroso. |
| ¿Guarda de pegado multilínea? | **Sí, v1** | Barato y evita el accidente clásico. |
| ¿`Ctrl+C` copia? | Solo con selección; si no, SIGINT | Romper SIGINT rompe la terminal. |
| ¿Copiar al seleccionar? | No por defecto | Sorprende en Windows. |
| ¿API de lectura por rangos en capa 1? | **Sí, v1** | **La única cosa de este doc que no se puede añadir después.** |
| ¿Lector de pantalla (UIA)? | **No en v1** — pero API lista | Caro. Aditivo si se respeta la fila anterior. |
| ¿Zoom, campana visual, cursor sin parpadeo? | Sí, fase 1 | Baratos y de uso diario. |
| ¿Contraste mínimo automático? | Fase 4 | Medio coste, gran ganancia con temas de terceros. |
| ¿IME? | Fase 4 | Alto coste, aislado en capa 3. |
| ¿Configurable por el usuario? | Archivo de texto, sin GUI de settings | Como foot. La GUI de config es otro proyecto. |

---

## 5. Qué cambiar en `docs/02`

1. **§1**, contrato de capa 1: añadir *"y expone una vista de solo lectura
   direccionable por rangos, con notificación de cambios"*.
2. **§5**, tabla de stack: añadir fila **"Encoding de input → propio, ~500-900
   líneas"**. Hoy el doc lo da por prestado y no lo es.
3. **§6**, fase 0: añadir al criterio de salida *"`Ctrl+Backspace` borra una
   palabra en PSReadLine"*. Es una prueba de una línea que ejercita toda la
   cadena teclado → encoding → ConPTY → shell.
4. **§6**, fase 1: los ítems 3, 5, 6, 9 de la tabla §3.2 caben aquí y son baratos.

---

## 6. Resumen

- **Dos de tus tres ejemplos ya funcionan**, y funcionan en el shell. La terminal
  no debe tocarlos: historial global de 4096 entradas y `Ctrl+Backspace` →
  `BackwardKillWord`, ambos verificados en esta máquina.
- **El trabajo real de tu lista es copiar/pegar**, y `alacritty_terminal` te
  regala la mitad (`selection`).
- **El hueco no listado es el encoding de teclado**: `alacritty_terminal` no lo
  trae, `docs/02` asume que sí. Presupuestarlo.
- **Lo único urgente de accesibilidad es una decisión de API**, no una feature:
  que la capa 1 se pueda *leer* por rangos. Todo lo demás es aditivo.
