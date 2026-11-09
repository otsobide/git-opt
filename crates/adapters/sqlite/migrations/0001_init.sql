-- Key/value runtime settings (JSON values), edited from the web UI.
CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- A GitHub user/org (or "me": everything the token can access) whose
-- repositories are imported and kept in sync.
CREATE TABLE sources (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    kind             TEXT NOT NULL CHECK (kind IN ('user', 'org', 'me')),
    name             TEXT NOT NULL DEFAULT '',
    include_forks    INTEGER NOT NULL DEFAULT 0,
    include_archived INTEGER NOT NULL DEFAULT 0,
    auto_sync        INTEGER NOT NULL DEFAULT 1,
    repo_count       INTEGER NOT NULL DEFAULT 0,
    last_synced_at   TEXT,
    last_error       TEXT,
    created_at       TEXT NOT NULL,
    UNIQUE (kind, name)
);

CREATE TABLE repos (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    owner                 TEXT NOT NULL,
    name                  TEXT NOT NULL,
    source_id             INTEGER REFERENCES sources (id) ON DELETE SET NULL,
    -- 'direct': push to the repo itself; 'fork': contribute through a fork.
    mode                  TEXT NOT NULL DEFAULT 'direct' CHECK (mode IN ('direct', 'fork')),
    clone_url             TEXT NOT NULL,
    default_branch        TEXT NOT NULL DEFAULT 'main',
    fork_full_name        TEXT,
    fork_clone_url        TEXT,
    enabled               INTEGER NOT NULL DEFAULT 1,
    auto_pr               INTEGER NOT NULL DEFAULT 1,
    poll_interval_minutes INTEGER,
    ignore_globs          TEXT NOT NULL DEFAULT '',
    status                TEXT NOT NULL DEFAULT 'pending',
    last_error            TEXT,
    last_seen_sha         TEXT,
    last_checked_at       TEXT,
    next_check_at         TEXT,
    pending_commit        TEXT,
    pr_number             INTEGER,
    pr_url                TEXT,
    pr_state              TEXT,
    -- Bytes saved by this bot's PRs that ended up merged.
    bytes_saved           INTEGER NOT NULL DEFAULT 0,
    created_at            TEXT NOT NULL,
    UNIQUE (owner, name)
);

CREATE TABLE runs (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_id         INTEGER NOT NULL REFERENCES repos (id) ON DELETE CASCADE,
    trigger         TEXT NOT NULL,
    status          TEXT NOT NULL,
    started_at      TEXT NOT NULL,
    finished_at     TEXT,
    head_sha        TEXT,
    commit_sha      TEXT,
    files_scanned   INTEGER NOT NULL DEFAULT 0,
    files_optimized INTEGER NOT NULL DEFAULT 0,
    bytes_before    INTEGER NOT NULL DEFAULT 0,
    bytes_after     INTEGER NOT NULL DEFAULT 0,
    pr_number       INTEGER,
    pr_url          TEXT,
    message         TEXT
);
CREATE INDEX runs_repo ON runs (repo_id, id DESC);

CREATE TABLE run_files (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id        INTEGER NOT NULL REFERENCES runs (id) ON DELETE CASCADE,
    path          TEXT NOT NULL,
    optimizer     TEXT NOT NULL,
    original_blob TEXT NOT NULL,
    bytes_before  INTEGER NOT NULL,
    bytes_after   INTEGER NOT NULL
);
CREATE INDEX run_files_run ON run_files (run_id);

-- Content-addressed memory of what each optimizer profile did with a blob.
-- Global across repos: the same bytes give the same result everywhere.
CREATE TABLE blob_cache (
    blob_sha       TEXT NOT NULL,
    profile        TEXT NOT NULL,
    result         TEXT NOT NULL CHECK (result IN ('optimal', 'optimized', 'failed')),
    optimized_blob TEXT,
    bytes_before   INTEGER,
    bytes_after    INTEGER,
    detail         TEXT,
    updated_at     TEXT NOT NULL,
    PRIMARY KEY (blob_sha, profile)
);

-- Blobs a maintainer rejected (PR closed without merging): never propose again.
CREATE TABLE repo_ignored_blobs (
    repo_id  INTEGER NOT NULL REFERENCES repos (id) ON DELETE CASCADE,
    blob_sha TEXT NOT NULL,
    PRIMARY KEY (repo_id, blob_sha)
);
