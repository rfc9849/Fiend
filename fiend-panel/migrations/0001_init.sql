CREATE TABLE admins (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    totp_secret TEXT,
    created_at INTEGER NOT NULL
);

CREATE TABLE sessions (
    id_hash TEXT PRIMARY KEY,
    admin_id INTEGER NOT NULL REFERENCES admins(id) ON DELETE CASCADE,
    csrf TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_seen INTEGER NOT NULL
);

CREATE TABLE disguises (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    origin_host TEXT NOT NULL,
    origin_port INTEGER NOT NULL,
    template_raw BLOB NOT NULL,
    origin_profile TEXT NOT NULL DEFAULT '{}',
    probed_at INTEGER,
    probe_error TEXT,
    created_at INTEGER NOT NULL
);

CREATE TABLE nodes (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    address TEXT NOT NULL,
    port INTEGER NOT NULL,
    local INTEGER NOT NULL DEFAULT 0,
    arch TEXT,
    control_secret TEXT NOT NULL,
    service_secret TEXT NOT NULL,
    ssh_host_key TEXT,
    version TEXT,
    binary_sha256 TEXT,
    applied_version INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'unknown',
    last_error TEXT,
    last_seen INTEGER,
    started_at INTEGER,
    disguise_id INTEGER REFERENCES disguises(id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE users (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    secret TEXT NOT NULL UNIQUE,
    enabled INTEGER NOT NULL DEFAULT 1,
    note TEXT NOT NULL DEFAULT '',
    traffic_limit INTEGER,
    used_up INTEGER NOT NULL DEFAULT 0,
    used_down INTEGER NOT NULL DEFAULT 0,
    expire_at INTEGER,
    hold_days INTEGER,
    reset_period TEXT NOT NULL DEFAULT 'none',
    reset_at INTEGER,
    usage_epoch INTEGER NOT NULL DEFAULT 0,
    last_seen INTEGER,
    online INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);

CREATE TABLE user_nodes (
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    node_id INTEGER NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, node_id)
);

CREATE TABLE outbounds (
    id INTEGER PRIMARY KEY,
    node_id INTEGER REFERENCES nodes(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    config TEXT NOT NULL,
    udp_fallback TEXT,
    UNIQUE (node_id, name)
);

CREATE TABLE rules (
    id INTEGER PRIMARY KEY,
    node_id INTEGER REFERENCES nodes(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    pattern TEXT NOT NULL,
    outbound TEXT NOT NULL
);

CREATE TABLE warp (
    node_id INTEGER PRIMARY KEY REFERENCES nodes(id) ON DELETE CASCADE,
    enabled INTEGER NOT NULL DEFAULT 0,
    rotate_every INTEGER,
    status TEXT,
    exit_v4 TEXT,
    exit_v6 TEXT,
    rotated_at INTEGER,
    checked_at INTEGER
);

CREATE TABLE traffic_minute (
    ts INTEGER NOT NULL,
    node_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    up INTEGER NOT NULL,
    down INTEGER NOT NULL,
    PRIMARY KEY (ts, node_id, user_id)
);

CREATE TABLE traffic_hour (
    ts INTEGER NOT NULL,
    node_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    up INTEGER NOT NULL,
    down INTEGER NOT NULL,
    PRIMARY KEY (ts, node_id, user_id)
);

CREATE TABLE traffic_day (
    ts INTEGER NOT NULL,
    node_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    up INTEGER NOT NULL,
    down INTEGER NOT NULL,
    PRIMARY KEY (ts, node_id, user_id)
);

CREATE INDEX traffic_minute_user ON traffic_minute (user_id, ts);
CREATE INDEX traffic_hour_user ON traffic_hour (user_id, ts);
CREATE INDEX traffic_day_user ON traffic_day (user_id, ts);

CREATE TABLE node_metrics (
    ts INTEGER NOT NULL,
    node_id INTEGER NOT NULL,
    cpu REAL NOT NULL,
    memory_used INTEGER NOT NULL,
    memory_total INTEGER NOT NULL,
    rx INTEGER NOT NULL,
    tx INTEGER NOT NULL,
    handshakes_ok INTEGER NOT NULL,
    handshakes_origin INTEGER NOT NULL,
    PRIMARY KEY (ts, node_id)
);

CREATE TABLE bundles (
    id INTEGER PRIMARY KEY,
    version TEXT NOT NULL,
    arch TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    core_sha256 TEXT NOT NULL,
    path TEXT NOT NULL,
    uploaded_at INTEGER NOT NULL,
    UNIQUE (version, arch, sha256)
);

CREATE TABLE audit (
    id INTEGER PRIMARY KEY,
    ts INTEGER NOT NULL,
    admin_id INTEGER,
    action TEXT NOT NULL,
    target TEXT NOT NULL,
    detail TEXT NOT NULL DEFAULT ''
);

CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
