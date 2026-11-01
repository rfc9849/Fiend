# Server

Needs Docker with Compose and free TCP ports 443 and 38029.

```sh
mkdir fiend && cd fiend
curl -fsSLO https://raw.githubusercontent.com/rfc9849/Fiend/master/compose.yaml
docker compose up -d
docker compose logs panel
```

The log has a setup link, `http://<host>:38029/setup/<token>`. It works once and expires in an hour. To get a new one:

```sh
docker compose exec panel fiend-panel renew-setup /etc/fiend/panel.toml
docker compose restart panel
```

The panel is plain HTTP. For access over the internet, put an HTTPS reverse proxy in front of it on the same server and pass `X-Real-IP` and `X-Forwarded-Proto`.

## Settings

Put these into `.env` next to `compose.yaml`:

- `FIEND_NODE_PORT`: client port, 443 by default. Read on the first start only.
- `FIEND_VERSION`: image tag, `latest` by default.

## Update

```sh
docker compose pull
docker compose up -d
```
