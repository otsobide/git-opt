# AGENTS.md

Guía para agentes (y personas) que trabajen en este repositorio. Visión general y uso: `README.md`.

## Qué es

`git-opt` es un bot autoalojado tipo ImgBot: vigila repos de GitHub, optimiza imágenes **sin pérdidas** y abre PRs con el token y la identidad git del desarrollador dueño de la máquina. Backend Rust (`backend/`), panel Vue (`frontend/`), una sola imagen Docker.

## Comandos

```bash
cargo build                         # desde la raíz (workspace)
cargo test                          # tests unitarios del backend
cargo clippy --all-targets          # lint
scripts/e2e.sh                      # flujo completo con GitHub falso; necesita magick y jpegtran
cd frontend && npm ci && npm run build   # type-check (vue-tsc) + build a frontend/dist
cd frontend && npm run dev          # Vite :5173, proxy /api -> 127.0.0.1:8484
docker compose up -d --build        # imagen única (API + panel + git/jpegtran/gifsicle)
```

Antes de dar algo por terminado: `cargo test` y `cargo clippy` sin warnings; si tocaste `pipeline.rs`, `git.rs`, `github.rs` o `api.rs`, ejecuta también `scripts/e2e.sh`; si tocaste `frontend/`, `npm run build`.

## Mapa del backend

| Fichero | Responsabilidad |
|---|---|
| `main.rs` | arranque, servidor axum, panel estático con fallback SPA, apagado con SIGTERM |
| `config.rs` | variables de entorno (`GIT_OPT_*`); nada que se edite en caliente |
| `settings.rs` | ajustes editables desde la UI (JSON en la tabla `settings`); `sanitized()` acota valores |
| `db.rs` + `migrations/` | **todas** las consultas SQL. No escribas SQL fuera de `db.rs` |
| `api.rs` | rutas `/api/*`; errores como `{ "error": "..." }`; mensajes al usuario en español |
| `jobs.rs` | cola en memoria: deduplica por repo, nunca dos trabajos del mismo repo a la vez, progreso en vivo |
| `scheduler.rs` | ticker (15 s) que encola lo vencido y despachador con concurrencia leída en cada vuelta |
| `pipeline.rs` | el trabajo por repo: fetch → candidatos → optimizar → commit → push → PR; alta de repos y sync de fuentes |
| `git.rs` | **único** sitio que ejecuta `git`. Constante `BOT_MARKER` |
| `github.rs` | cliente REST y resolución del token (UI → env → `gh auth token`) |
| `optimizers/` | trait `Optimizer`, registro, oxipng/jpegtran/gifsicle, verificación por píxeles |
| `report.rs` | mensaje de commit y cuerpo de PR (ambos llevan `BOT_MARKER`) |

## Invariantes que no se pueden romper

- **Sin pérdidas de verdad.** Todo optimizador implementa `verify` y debe demostrar equivalencia (hoy: decodificar y comparar píxeles/frames). Si no se puede verificar, no se toca el fichero. Nunca actives opciones con pérdida (`--lossy`, `optimize_alpha`, `scale_16`, `-copy none` en JPEG…).
- **El marcador** `Git-Opt: lossless-optimization` va en cada commit y PR del bot. La detección de "ya optimizado" se basa en los blobs que introducen esos commits (`Git::blobs_from_bot_commits`); cambiar el texto rompe el reconocimiento del historial existente.
- **Identidad del desarrollador.** Commits con `git config` del sistema (o lo fijado en Ajustes, o la cuenta de GitHub); API y push con su token. No hay cuenta "bot".
- **Secretos.** El token nunca va en argumentos ni URLs: se pasa a git con `GIT_CONFIG_COUNT/KEY/VALUE` (`GitAuth`). No lo registres en logs ni lo devuelvas por la API (`PUT /api/settings/github-token` es de solo escritura).
- **El clon es del bot.** Vive en `$GIT_OPT_DATA_DIR/repos/<owner>/<name>`; se puede hacer `checkout --force`/`clean` sin miedo. Commits con `--no-verify` y `core.hooksPath=/dev/null`.
- **Nada de ruido en el historial**: las comprobaciones programadas sin cambios no crean filas en `runs`; los errores repetidos tampoco.

## Añadir un optimizador / tipo de fichero

1. Nuevo módulo en `backend/src/optimizers/` que implemente `Optimizer` (`id` estable, `extensions`, `profile` con todo lo que altera la salida, `probe`, `optimize`, `verify`).
2. Registrarlo en `Registry::new`.
3. Si es una herramienta externa, usar `external::run_on_bytes` y añadirla al `Dockerfile` (etapa runtime).
4. Tests en `optimizers/mod.rs`: que reduce tamaño, que `verify` rechaza un cambio mínimo y que un fichero ya óptimo se deja igual.
5. Añadir su línea en el bloque "How" de `report::pr_body` y en la tabla del README.

El pipeline, la caché (`blob_cache` con clave `blob + profile`), la UI y los informes no necesitan cambios.

## Esquema y migraciones

SQLite vía `sqlx` con `sqlx::migrate!`. Para cambiar el esquema añade `backend/migrations/000N_descripcion.sql`; **no edites migraciones ya publicadas**. Las fechas son `DateTime<Utc>` (texto RFC 3339 comparable lexicográficamente).

## Frontend

Vue 3 + Vite + TypeScript + vue-router, CSS propio sin frameworks de UI. Textos en español. El contrato de la API está en los tipos de `frontend/src/types.ts`; si cambias un modelo en `backend/src/models.rs` o una respuesta en `api.rs`, actualiza ambos lados.

## Idiomas

- Código, identificadores, comentarios y logs: inglés.
- UI y mensajes de error de la API pensados para el usuario: español.
- Commits y PRs que genera el bot: inglés (van a repos de terceros).

## Commits de este repo

Commits pequeños y frecuentes con mensajes cortos de una línea en inglés, en imperativo ("Add fork mode retry"). Sin descripciones largas y **sin trailers `Co-Authored-By` ni ninguna atribución a asistentes de IA**.

## Pruebas sin tocar GitHub

`scripts/mock-github.py` imita los endpoints que usamos (repos, forks, pulls, `/user`) con repos *bare* locales como `clone_url` (`file://…`; sin `https` no se inyecta auth). `scripts/e2e.sh` lo usa para probar: alta → PR → píxeles idénticos (ImageMagick) → merge → nada pendiente → repo de terceros en modo fork con aprobación manual. Nunca pruebes contra repos reales sin permiso explícito: crear forks y PRs es visible para otras personas.
