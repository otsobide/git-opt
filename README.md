# git-opt

Bot **autoalojado** al estilo de ImgBot: vigila tus repositorios de GitHub, busca imágenes que se pueden comprimir **sin pérdidas** y abre una pull request **a tu nombre** con el resultado. Backend en Rust, panel de gestión en Vue y todo empaquetado en **un único contenedor Docker**.

- Optimización **sin pérdidas** comprobada: cada imagen optimizada se decodifica y se compara píxel a píxel con la original; si no son idénticas, no se toca.
- Trabajo en **segundo plano**: cada repositorio se comprueba cada **20 min + un aleatorio de hasta 20 min** (configurable globalmente y por repo).
- Sabe lo que ya optimizó **leyendo el historial de commits**, aunque la PR se haya fusionado con *merge*, *rebase* o *squash*, y aunque borres su base de datos.
- PRs y commits **en nombre del desarrollador dueño de la máquina**: usa tu token de GitHub y tu `git config`.
- **Contribuciones a terceros**: para repos donde no tienes permiso de escritura crea un fork en tu cuenta y abre la PR contra el original.
- Importa **todos los repos de un usuario, de una organización o todos a los que tienes acceso**, y se mantiene sincronizado.
- Usa el **`git` del sistema** para clonar, hacer commit y push.
- Pensado para crecer: los optimizadores son *plugins* (`Optimizer`); hoy son imágenes, mañana pueden ser otros tipos de fichero.

## Cómo funciona

```
cada 20 min + rand(0..20 min), por repo
  │
  ├─ GitHub API: rama por defecto, estado de la PR anterior (¿fusionada? ¿cerrada?)
  ├─ git clone (la primera vez) / git fetch origin <rama por defecto>
  ├─ ¿HEAD igual que la última vez? → nada que hacer
  ├─ git ls-tree: lista de ficheros con optimizador (png, jpg/jpeg, gif)
  │    descarta: patrones ignorados, ficheros LFS, demasiado grandes,
  │              blobs escritos por commits del bot (historial),
  │              blobs rechazados en PRs cerradas sin fusionar,
  │              blobs que ya se analizaron y no mejoran (caché)
  ├─ optimiza → verifica píxel a píxel → escribe el fichero
  ├─ git commit (autor: tú) con el marcador  Git-Opt: lossless-optimization
  ├─ git push --force a la rama git-opt/optimize-images (del repo o de tu fork)
  └─ abre la PR, o actualiza la que ya estaba abierta
```

### Cómo reconoce su propio trabajo

Cada commit del bot termina con el *trailer* `Git-Opt: lossless-optimization` (y el cuerpo de la PR también lo incluye, para que sobreviva a un *squash* que use la descripción de la PR). En cada comprobación el bot ejecuta

```
git log --grep "Git-Opt: lossless-optimization" --raw <rama por defecto>
```

y recoge los **hashes de los blobs** que escribieron esos commits. Como se compara por contenido y no por id de commit, funciona igual con *merge*, *rebase* o *squash*. Si una imagen se modifica después, su blob cambia y se vuelve a evaluar.

Además guarda en SQLite una caché por contenido (`blob → resultado`) para no reprocesar imágenes que ya están óptimas, y recuerda los ficheros de PRs que los mantenedores **cerraron sin fusionar** para no volver a proponerlos (se puede olvidar desde la UI).

### Garantía "sin pérdidas"

| Formato | Herramienta | Qué cambia | Qué no cambia |
|---|---|---|---|
| PNG | [oxipng](https://github.com/shssoichiro/oxipng) (librería, integrada) | filtros, compresión deflate, reducción de tipo de color sin pérdida | píxeles; metadatos (salvo que actives *quitar metadatos*) |
| JPEG | `jpegtran -optimize [-progressive] -copy all` | tablas Huffman, modo progresivo si es más pequeño | coeficientes DCT y **todos** los metadatos (EXIF/orientación, ICC) |
| GIF | `gifsicle -O3` (nunca `--lossy`) | codificación de frames | cada frame compuesto y su duración |

Después de optimizar, **siempre** se decodifican original y resultado y se exige igualdad exacta de dimensiones y valores RGBA (en GIF, de cada frame y su retardo). Si la verificación falla o el formato no se puede decodificar, el fichero se deja como está. Los PNG animados (APNG) se omiten. Además, solo se acepta un resultado si ahorra al menos un 1 % y 256 bytes (configurable).

## Puesta en marcha con Docker (recomendado)

Un único contenedor con la API, el panel web, `git`, `jpegtran` y `gifsicle`.

```bash
export GITHUB_TOKEN=$(gh auth token)      # o un PAT (ver "Token de GitHub")
export GIT_OPT_ADMIN_TOKEN=$(openssl rand -hex 24)   # protege el panel
docker compose up -d --build
```

Abre <http://localhost:8484>. Los datos (SQLite + clones) viven en el volumen `git-opt-data`.

Para que los commits usen tu identidad de git, descomenta en `docker-compose.yml` el montaje de `~/.gitconfig`; si no, puedes fijar nombre y email en **Ajustes**, y si tampoco, se usa tu cuenta de GitHub (`nombre <id+login@users.noreply.github.com>`). Si tu `.gitconfig` firma commits (`commit.gpgsign`) y en el contenedor no hay clave, desactiva **Firmar commits** en Ajustes.

Sin compose:

```bash
docker build -t git-opt .
docker run -d --name git-opt -p 127.0.0.1:8484:8484 \
  -e GITHUB_TOKEN -e GIT_OPT_ADMIN_TOKEN \
  -v git-opt-data:/data git-opt
```

## Instalación desde el código

Requisitos: Rust ≥ 1.88, Node ≥ 20, `git` y, para JPEG/GIF, `jpegtran` (paquete `libjpeg-turbo`, `libjpeg-turbo-progs` en Debian) y `gifsicle`. PNG no necesita nada externo. Si falta una herramienta, ese formato aparece como *no disponible* en el panel y se ignora.

```bash
(cd frontend && npm ci && npm run build)
cargo build --release
./target/release/git-opt          # sirve API + panel en http://127.0.0.1:8484
```

Como servicio de usuario de systemd (así usa tu `git config` y tu sesión de `gh`):

```bash
install -Dm755 target/release/git-opt ~/.local/bin/git-opt
mkdir -p ~/.local/share/git-opt && cp -r frontend/dist ~/.local/share/git-opt/web
install -Dm644 contrib/git-opt.service ~/.config/systemd/user/git-opt.service
systemctl --user daemon-reload && systemctl --user enable --now git-opt
loginctl enable-linger "$USER"     # que siga funcionando sin sesión abierta
```

## Token de GitHub

Todas las llamadas a la API y los `push` se hacen con **tu** token, así que las PRs, los forks y los commits salen a tu nombre. Se busca en este orden:

1. El token guardado en **Ajustes** del panel (solo escritura, nunca se muestra).
2. Variables de entorno `GIT_OPT_GITHUB_TOKEN`, `GITHUB_TOKEN` o `GH_TOKEN`.
3. `gh auth token` (la sesión de GitHub CLI del usuario que ejecuta el servicio).

Permisos: el token de `gh auth login` ya vale. Si usas un PAT clásico, necesita `repo` y `workflow` (`workflow` hace falta para subir ramas a un fork cuando el original tiene GitHub Actions). El token nunca aparece en la línea de comandos: se inyecta a `git` como cabecera HTTP mediante variables `GIT_CONFIG_*`.

## Uso del panel

- **Panel**: totales, trabajos en curso y en cola con su fase, actividad reciente, con qué cuenta salen las PRs y qué optimizadores están disponibles.
- **Repositorios**: añade `owner/repo` o una URL. El modo se elige solo: **directo** si puedes hacer push, **fork (contribución)** si es de terceros. Puedes desactivar *PR automática* para revisar antes: el bot deja la rama preparada y la PR se abre con **Abrir PR**. Desde aquí también: comprobar ahora, forzar, pausar, borrar.
- **Detalle de repo**: intervalo propio, modo, patrones ignorados, historial de ejecuciones y "olvidar ficheros rechazados".
- **Fuentes**: añade un usuario, una organización o "mis repos" (todos a los que tiene acceso el token). Se resincronizan periódicamente para detectar repos nuevos.
- **Actividad**: cada ejecución con el detalle por fichero.
- **Ajustes**: intervalos, concurrencia, umbrales, nivel de oxipng, patrones ignorados globales, rama, título del commit/PR, autor, firma y token.

Los patrones ignorados siguen la sintaxis de `.gitignore` simplificada: `*.min.png` (a cualquier profundidad), `docs/` (un directorio), `assets/raw/*.jpg`. Las líneas con `#` son comentarios.

## Configuración

Variables de entorno (lo demás se configura desde el panel y se guarda en la base de datos):

| Variable | Por defecto | Descripción |
|---|---|---|
| `GIT_OPT_BIND` | `127.0.0.1:8484` (`0.0.0.0:8484` en Docker) | Dirección HTTP |
| `GIT_OPT_DATA_DIR` | `~/.local/share/git-opt` (`/data` en Docker) | SQLite y clones |
| `GIT_OPT_STATIC_DIR` | `frontend/dist` si existe | Panel web compilado |
| `GIT_OPT_ADMIN_TOKEN` | — | Si se define, la API exige `Authorization: Bearer <token>` |
| `GIT_OPT_GITHUB_API` | `https://api.github.com` | Para GitHub Enterprise: `https://<host>/api/v3` |
| `GIT_OPT_GITHUB_TOKEN` / `GITHUB_TOKEN` / `GH_TOKEN` | — | Token de GitHub |
| `RUST_LOG` | `git_opt=info,tower_http=warn` | Nivel de logs |

## Desarrollo

```bash
cargo test                            # tests del backend
cargo run                             # API en :8484
(cd frontend && npm run dev)          # Vite en :5173 con proxy de /api → :8484
scripts/e2e.sh                        # flujo completo contra un GitHub falso y repos locales
```

`scripts/e2e.sh` levanta un mock de la API de GitHub (`scripts/mock-github.py`) y repos *bare* locales, añade un repo, espera la PR y comprueba con ImageMagick que los píxeles son idénticos; no toca GitHub real.

### Estructura

```
backend/src/
  main.rs         arranque: config, BD, scheduler, servidor HTTP + panel estático
  api.rs          API JSON (axum)
  scheduler.rs    ticker de comprobaciones y despachador con concurrencia limitada
  jobs.rs         cola en memoria, deduplicación por repo y progreso en vivo
  pipeline.rs     lo que se hace con cada repo (fetch → candidatos → optimizar → commit → PR)
  git.rs          wrapper del git del sistema
  github.rs       cliente REST de GitHub y resolución del token
  optimizers/     trait Optimizer + oxipng, jpegtran, gifsicle y verificación por píxeles
  report.rs       mensaje de commit y cuerpo de la PR
  db.rs           todas las consultas SQLite;  migrations/ el esquema
frontend/         panel Vue 3 + Vite + TypeScript
```

### Añadir un nuevo tipo de fichero

Implementa `optimizers::Optimizer` (id, extensiones, `profile`, `probe`, `optimize` y, sobre todo, `verify`, que debe **demostrar** que el resultado es equivalente) y regístralo en `Registry::new`. Aparecerá automáticamente en el panel con su interruptor, y el pipeline, la caché y los informes no necesitan cambios.

## Ideas para más adelante

SVG (svgo con verificación por rasterizado), WebP sin pérdidas, GitLab/Gitea, webhooks en lugar de *polling*, y un fichero de configuración dentro de cada repo (`.git-opt.json`) para que los mantenedores ajusten el comportamiento.
