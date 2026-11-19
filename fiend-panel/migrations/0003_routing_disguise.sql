ALTER TABLE rules ADD COLUMN udp_fallback TEXT;
ALTER TABLE nodes ADD COLUMN disguise_version INTEGER NOT NULL DEFAULT 0;
ALTER TABLE nodes ADD COLUMN disguise_changed_at INTEGER;

UPDATE rules AS r
SET
    outbound = '@' || (
        SELECT o.kind FROM outbounds AS o
        WHERE o.name = r.outbound AND (o.node_id IS NULL OR o.node_id IS r.node_id)
        ORDER BY o.node_id IS NOT NULL DESC, o.id DESC LIMIT 1
    ),
    udp_fallback = (
        SELECT o.udp_fallback FROM outbounds AS o
        WHERE o.name = r.outbound AND (o.node_id IS NULL OR o.node_id IS r.node_id)
        ORDER BY o.node_id IS NOT NULL DESC, o.id DESC LIMIT 1
    )
WHERE (
    SELECT o.kind = 'warp' OR (o.kind = 'block' AND o.udp_fallback IS NULL)
    FROM outbounds AS o
    WHERE o.name = r.outbound AND (o.node_id IS NULL OR o.node_id IS r.node_id)
    ORDER BY o.node_id IS NOT NULL DESC, o.id DESC LIMIT 1
) AND (
    r.node_id IS NOT NULL
    OR NOT EXISTS (
        SELECT 1 FROM outbounds AS o
        WHERE o.name = r.outbound AND o.node_id IS NOT NULL
    )
);

DELETE FROM outbounds AS o
WHERE o.kind IN ('warp', 'block')
    AND NOT EXISTS (SELECT 1 FROM rules AS r WHERE r.outbound = o.name)
    AND NOT EXISTS (SELECT 1 FROM outbounds AS other WHERE other.udp_fallback = o.name)
    AND NOT EXISTS (SELECT 1 FROM rules AS r WHERE r.udp_fallback = o.name);

UPDATE settings SET value = CAST(value AS INTEGER) + 1 WHERE key = 'state_version';
