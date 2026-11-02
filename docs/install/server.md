# Server

Needs Docker with Compose and a free TCP port 443.

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
- `FIEND_PANEL_LISTEN`: panel address and port, `127.0.0.1:38029` by default. Read on the first start only.
- `FIEND_VERSION`: image tag, `latest` by default.

To change the panel address of an installed server, edit `listen` in `/etc/fiend/panel.toml` and restart the panel:

```sh
docker compose exec panel sed -i 's/^listen = .*/listen = "127.0.0.1:38029"/' /etc/fiend/panel.toml
docker compose restart panel
```

## Disguise

Use a TLS service that really runs on the same server, if you can. If the disguise site has a different IP address, scanners record its certificate on your IP address, and a database search finds your server. The panel shows a warning for this.

## Update

```sh
docker compose pull
docker compose up -d
```
