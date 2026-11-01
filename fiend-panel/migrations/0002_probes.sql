CREATE TABLE probes (
    id INTEGER PRIMARY KEY,
    ts INTEGER NOT NULL,
    node_id INTEGER NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    peer TEXT NOT NULL,
    reason TEXT NOT NULL,
    sni TEXT,
    size INTEGER NOT NULL,
    head BLOB NOT NULL
);

CREATE INDEX probes_ts ON probes (ts);
CREATE INDEX probes_peer ON probes (peer, ts);
