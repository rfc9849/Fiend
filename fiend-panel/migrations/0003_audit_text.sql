UPDATE audit
SET detail = REPLACE(REPLACE(REPLACE(REPLACE(detail, 'Some("', ''), '")', ''), 'None', 'none'), ' -> ', ' → ')
WHERE action = 'warp new ip';
UPDATE audit SET detail = 'IPv4 ' || detail WHERE action = 'warp new ip' AND detail NOT LIKE 'IPv4 %';
UPDATE audit SET detail = REPLACE(detail, ', ', ', IPv6 ') WHERE action = 'warp new ip' AND detail NOT LIKE '%IPv6%';
UPDATE audit SET action = 'WARP new IP' WHERE action = 'warp new ip';
UPDATE audit SET action = 'WARP registered' WHERE action = 'warp registered';
UPDATE audit SET action = 'WARP registration failed' WHERE action = 'warp register failed';
UPDATE audit SET action = 'WARP account deleted' WHERE action = 'warp deleted';
UPDATE audit SET action = 'WARP settings' WHERE action = 'warp settings';
UPDATE audit SET target = 'node ' || (SELECT name FROM nodes WHERE 'node ' || nodes.id = audit.target)
WHERE action LIKE 'WARP %' AND EXISTS (SELECT 1 FROM nodes WHERE 'node ' || nodes.id = audit.target);
