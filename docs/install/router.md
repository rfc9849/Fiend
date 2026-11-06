# Router (OpenWrt)

Fiend runs on OpenWrt 24.10 with firewall4 and dnsmasq, the defaults of OpenWrt. Files are in the [latest release](https://github.com/rfc9849/Fiend/releases/latest). Replace `0.1.7` below with its version.

## How it works

Only the sites in your lists go through Fiend. All other traffic stays in the kernel at full speed, with flow offloading.

dnsmasq sends its DNS queries to Fiend. When a site is in a Tunnel or DPI bypass list, Fiend puts the addresses from the answer into an nftables set before it sends the answer. For a site in a Block list, Fiend answers that the site does not exist. The kernel then gives the traffic to these addresses, and to the networks in your lists, to Fiend with TPROXY. The addresses are real, so a stop of Fiend does not break the sites: the rules go away and the traffic goes directly again.

## Install

1. Find the package architecture of the router. It is the last line of:

   ```sh
   opkg print-architecture
   ```

   Packages are for `aarch64_*`, `arm_cortex-a*` and `x86_64`. MIPS routers are not supported.

2. Copy `fiend_0.1.7-r1_<architecture>.ipk` and `luci-app-fiend_0.1.7-r1_all.ipk` to `/tmp` on the router, then install them:

   ```sh
   opkg update
   opkg install /tmp/fiend_0.1.7-r1_*.ipk /tmp/luci-app-fiend_0.1.7-r1_all.ipk
   ```

   `opkg update` is necessary, because Fiend needs the `kmod-nft-tproxy` and `kmod-nft-socket` kernel modules.

3. In LuCI, open **Services** → **Fiend** and paste your `fiend://` link. Fiend connects, and connects again after each restart of the router.

4. In **Routing**, add your lists. A list is a sing-box rule set (`.srs`) or a text list at a URL. For each list, select what Fiend does with its sites:

   - **Tunnel**: send through the Fiend server
   - **DPI bypass**: connect directly and split the TLS greeting
   - **Direct**: connect directly
   - **Block**: refuse the connection

   Fiend downloads the lists again each day.

## Settings

**Traffic that is not in a list** is **Direct** on a router. Select **Tunnel** to send all traffic through the tunnel, except the Direct lists. Then all traffic of the LAN goes through Fiend, which is slower.

In **Settings**:

- **Networks**: the networks whose devices use Fiend. The default is `lan`.
- **Catch all DNS queries**: devices that use a different DNS server, for example `8.8.8.8`, also send their queries to the router. Keep it on, or lists of sites do not work for such devices.
- **DNS server**: the server for the queries that do not go through the tunnel. Empty: the server that dnsmasq used before Fiend.

The same settings are in `/etc/config/fiend`. Profiles and lists are in `/etc/fiend`.

## Command line

```sh
fiend status
fiend subscriptions add https://example.com/list.srs --kind proxy
fiend logs -f
```

## Limits

- Lists of sites do not work for a browser with secure DNS (DNS over HTTPS), because its queries do not go to the router. Turn off secure DNS in the browser, or add the addresses of the site to a list.
- Changes to the lists apply after you reconnect. The **Overview** page shows a **Reconnect** button.
- OpenWrt 25 uses apk packages, which are not available yet.

## Remove

```sh
opkg remove luci-app-fiend fiend
```

The removal stops Fiend and restores dnsmasq. Your profiles stay in `/etc/fiend`.
