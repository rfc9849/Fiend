# Server

Needs Docker with Compose and a free TCP port 443. If a web server already uses port 443, put Fiend behind it, see [Behind a TLS proxy](#behind-a-tls-proxy).

```sh
mkdir fiend && cd fiend
curl -fsSLO https://raw.githubusercontent.com/rfc9849/Fiend/master/compose.yaml
docker compose up -d
docker compose logs panel
```

The panel listens on `127.0.0.1:38029` only, so scanners cannot find it. Open it through SSH from your computer:

```sh
ssh -N -L 8080:127.0.0.1:38029 <host>
```

The log has a setup link, `http://127.0.0.1:8080/setup/<token>`. It works once and expires in an hour. To get a new one:

```sh
docker compose exec panel fiend-panel renew-setup /etc/fiend/panel.toml
docker compose restart panel
```

The panel is plain HTTP. To open it over the internet, put an HTTPS reverse proxy in front of it on the same server and pass `X-Real-IP` and `X-Forwarded-Proto`. Do not open port 38029 to the internet.

## Settings

Put these into `.env` next to `compose.yaml`:

- `FIEND_NODE_PORT`: client port, 443 by default. Read on the first start only.
- `FIEND_NODE_LISTEN`: node address and port, `0.0.0.0:<FIEND_NODE_PORT>` by default. Read on the first start only.
- `FIEND_NODE_PROXY_PROTOCOL`: `true` if a proxy in front of the node sends a PROXY protocol header. Read on the first start only.
- `FIEND_NODE_FALLBACK`: `<host>=<address>`, the local address of the disguise site. Read on the first start only.
- `FIEND_NODE_FALLBACK_PROXY_PROTOCOL`: `true` to send a PROXY protocol header to the fallback address. Read on the first start only.
- `FIEND_PANEL_LISTEN`: panel address and port, `127.0.0.1:38029` by default. Read on the first start only.
- `FIEND_VERSION`: image tag, `latest` by default.

To change the panel address of an installed server, edit `listen` in `/etc/fiend/panel.toml` and restart the panel:

```sh
docker compose exec panel sed -i 's/^listen = .*/listen = "127.0.0.1:38029"/' /etc/fiend/panel.toml
docker compose restart panel
```

## Disguise

Use a TLS service that really runs on the same server, if you can. If the disguise site has a different IP address, scanners record its certificate on your IP address, and a database search finds your server. The panel shows a warning for this.

To make a `.bin` file from a client of the site, see [ClientHello capture](../capture.md).

## Behind a TLS proxy

nginx, HAProxy or a different TCP proxy can keep port 443 and send only the disguise site name to Fiend. Fiend does not need a port on the internet then.

The proxy reads the site name from the ClientHello and does not decrypt the connection. Fiend sends all connections without a valid key to the original site, so scanners and real visitors get the real site.

Set these in `/etc/fiend/node.toml`:

```toml
listen = "127.0.0.1:8443"   # the proxy connects here
public_port = 443           # the port in the client links
proxy_protocol = true       # the proxy sends the client address
```

If `proxy_protocol` is `true`, the node closes all connections without a PROXY protocol header. Turn it on in the proxy too, or set both to `false`. Do not open the listen address to the internet.

### Case 1: the disguise site runs on this server

Example: `site.example` runs on this server, and the disguise is a ClientHello of a real visitor of this site.

The disguise host points to this server, so Fiend cannot send probes to it by DNS. The probes come back to Fiend. Tell Fiend the local address of the site:

```toml
[[fallback]]
host = "site.example"        # the origin host of the disguise
address = "127.0.0.1:8444"   # the TLS server of the site
proxy_protocol = true        # send the visitor address to the site
```

The nginx config:

```nginx
stream {
    map $ssl_preread_server_name $backend {
        site.example  127.0.0.1:8443;   # Fiend
        default       127.0.0.1:8444;   # the sites
    }

    server {
        listen 443;
        listen [::]:443;
        ssl_preread on;
        proxy_protocol on;
        proxy_pass $backend;
    }
}

http {
    server {
        listen 127.0.0.1:8444 ssl proxy_protocol;
        http2 on;
        server_name site.example;
        ssl_certificate     /etc/letsencrypt/live/site.example/fullchain.pem;
        ssl_certificate_key /etc/letsencrypt/live/site.example/privkey.pem;
        set_real_ip_from 127.0.0.1;
        real_ip_header proxy_protocol;
        # the site config
    }
}
```

Visitors of `site.example` go through Fiend to `127.0.0.1:8444`. The site gets their real addresses from the PROXY protocol header. nginx accepts version 1 from the `stream` block and version 2 from Fiend on the same listen address.

Fiend selects the fallback from the site name in the ClientHello. Thus the proxy can send `site.example` to Fiend also when the disguise of the node is a different site.

The panel shows the fallback address in the disguise checks. If the check is red, the node has no fallback for the disguise host.

### Case 2: the disguise site runs on a different server

Example: the disguise is a capture of `captured-origin.example`, and your own sites run on this server.

Do not add a fallback. Fiend sends probes to the real `captured-origin.example` by DNS.

```nginx
stream {
    map $ssl_preread_server_name $backend {
        captured-origin.example  127.0.0.1:8443;   # Fiend
        default                  127.0.0.1:8444;   # your sites
    }

    server {
        listen 443;
        listen [::]:443;
        ssl_preread on;
        proxy_protocol on;
        proxy_pass $backend;
    }
}
```

The web server on `127.0.0.1:8444` must accept the PROXY protocol, for example `listen 127.0.0.1:8444 ssl proxy_protocol;`. The panel shows the same-server warning for this disguise.

### New server

Put these into `.env` before the first start:

```sh
FIEND_NODE_LISTEN=127.0.0.1:8443
FIEND_NODE_PROXY_PROTOCOL=true
# case 1 only:
FIEND_NODE_FALLBACK=site.example=127.0.0.1:8444
FIEND_NODE_FALLBACK_PROXY_PROTOCOL=true
```

`FIEND_NODE_PORT` stays 443, because the clients connect to the proxy on port 443.

### Installed server

Edit `/etc/fiend/node.toml`, check it, and restart the node:

```sh
docker compose exec node vi /etc/fiend/node.toml
docker compose exec node fiend-core check /etc/fiend/node.toml
docker compose restart node
```

The panel reads `public_port` from the local node and changes the port in the links. Send the new links to the users if the port changes.

The node uses the host network, so a proxy on the host can connect to `127.0.0.1:8443`. If the proxy runs in a Docker bridge network, set `listen` to the gateway address of this network, for example `172.18.0.1:8443`. Then let the network through the firewall, for example `ufw allow from 172.18.0.0/16 to any port 8443 proto tcp`. To let the node connect to a site in the proxy container, publish its port on the loopback address, for example `127.0.0.1:8444:8444`.

## Update

```sh
docker compose pull
docker compose up -d
```
