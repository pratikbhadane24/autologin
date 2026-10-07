-- Accounts are identified by tenant + broker + client_id, so the same broker
-- account can be linked to two Cirrus tenants. Secrets are NOT stored here;
-- they live in the OS keychain (see store/secrets.rs).
CREATE TABLE accounts (
    id          INTEGER PRIMARY KEY,
    tenant_id   TEXT    NOT NULL,
    broker_id   TEXT    NOT NULL,
    client_id   TEXT    NOT NULL,
    -- Non-secret manifest fields (api_key, mobile_number, ...) as a JSON object.
    fields      TEXT    NOT NULL DEFAULT '{}',
    -- Names of secret fields saved in the keychain, as a JSON array.
    secret_keys TEXT    NOT NULL DEFAULT '[]',
    status      TEXT    NOT NULL DEFAULT 'logged_out'
                        CHECK (status IN ('logged_out', 'logged_in', 'failed')),
    last_login  TEXT,   -- RFC 3339, UTC
    last_error  TEXT,
    added_on    TEXT    NOT NULL,
    UNIQUE (tenant_id, broker_id, client_id)
);

CREATE TABLE runs (
    id          INTEGER PRIMARY KEY,
    trigger     TEXT    NOT NULL CHECK (trigger IN ('manual', 'scheduled', 'retry')),
    started_at  TEXT    NOT NULL,
    finished_at TEXT,
    succeeded   INTEGER NOT NULL DEFAULT 0,
    failed      INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL      -- JSON
);
