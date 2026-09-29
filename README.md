# Anorak

Anorak is a self-hosted app for searching a Torznab indexer and sending the result to rqbit.
It is a general-purpose torrent grabber, rather than a show or movie manager.

It is written with Rust, Axum, MiniJinja, and HTMX.

## NixOS

Anorak does not need Docker. Build it from this repository and run it as a systemd service.

```nix
imports = [ /path/to/anorak/nix/module.nix ];

services.anorak = {
  enable = true;
  jackettUrl = "http://127.0.0.1:3420/api/v2.0/indexers/all/results/torznab";
  jackettApiKey = "lodestarr";
  rqbitUrl = "http://127.0.0.1:9030";
};
```

`jackettUrl` is the Torznab results URL. Point it at Lodestarr.
`jackettApiKey` is sent with each search. Lodestarr accepts any value.
`rqbitUrl` is rqbit's HTTP API. A grab is posted to `{rqbitUrl}/torrents`.
Sending never fails just because a torrent's metadata hasn't arrived yet. Links are checked first: a magnet needs `xt=urn:btih:` with a 40-hex or 32-base32 info hash, anything else must be an http(s) `.torrent` URL, and a bad link is reported as invalid with what's wrong. Magnets are posted with `defer_metadata=true` and a one-year `magnet_timeout_secs`, so rqbit queues them at once and keeps looking for peers; the button then shows "Queued in rqbit, waiting for peers…". If rqbit takes longer than about 8 seconds to answer, `POST /send-to-rqbit/` answers `202` with a job id and the page polls `GET /api/send/{job}` for as long as it takes; if rqbit's add request times out, anorak adds it again. Only a bad link is shown as an error (red); rqbit being unreachable or refusing for its own reasons is shown in amber with a retry. A second send of the same info hash while one is in flight joins it instead of adding it again.

### 1337x and Cloudflare

Every 1337x mirror is behind Cloudflare's browser check, which Lodestarr can't pass (it gets `403` and reports no results). Set `flaresolverrUrl` (env `FLARESOLVERR_URL`) to a [FlareSolverr](https://github.com/FlareSolverr/FlareSolverr) in the same network namespace and anorak searches 1337x itself: FlareSolverr's headless Chromium loads the search page (sorted by seeders, up to two pages) and anorak parses it. Results carry their details-page link; the magnet on that page is fetched, again through FlareSolverr, only when the result is sent or its files are listed. One FlareSolverr session is kept so only the first request pays for the challenge (about 12 s); later pages take 1-2 s, searches are cached for 10 minutes, and the session (and its Chromium) is closed after 30 idle minutes. If Cloudflare still wins (FlareSolverr times out after 40 s), the session is closed, the next mirror is not tried, and for 3 minutes 1337x is reported as blocked at once instead of holding every search up. `X1337X_MIRRORS` (comma-separated base URLs) overrides the mirror order; otherwise Lodestarr's list is used and the last mirror that worked is remembered.

A source that can't be searched is listed under "No answer from …" with the reason, e.g. "1337x (blocked by Cloudflare)" or "1337x (FlareSolverr not running)", instead of silently showing nothing. Lodestarr answers a blocked site with an empty list, so when a Lodestarr source returns nothing anorak fetches that site's home page once (cached 10 minutes) and reports "blocked by Cloudflare" or "site unreachable" when that's what it finds.

```nix
services.flaresolverr.enable = true;   # nixpkgs module; join it to the same namespace
services.anorak.flaresolverrUrl = "http://127.0.0.1:8191";
```

The service listens on port 9341. Set `openFirewall = true` only when it should be reachable on the host's own network.

To build the package by itself:

```bash
nix-build -E 'with import <nixpkgs> {}; callPackage ./nix/package.nix {}'
```

The pages are loaded from `assets/` in the process working directory. The NixOS module sets that directory to the package's `$out/share/anorak`.

### Only this service uses a VPN namespace

Create a network namespace that has no route except the VPN, with `/etc/netns/<name>/resolv.conf` pointing at resolvers reached through that tunnel. Then:

```nix
services.anorak = {
  enable = true;
  networkNamespace = "nordvpn";
  namespaceService = "nordvpn-netns.service";
  jackettUrl = "http://127.0.0.1:3420/api/v2.0/indexers/all/results/torznab";
  jackettApiKey = "lodestarr";
  rqbitUrl = "http://127.0.0.1:9030";
};
```

`namespaceService` is the unit that creates the namespace. Anorak waits for it and stops with it. Other programs on the machine keep the normal route. From the host, reach the UI at the namespace's address, for example `http://10.200.200.2:9341` when that address is the namespace end of a veth pair.

## Run from a checkout

```bash
export JACKETT_URL=http://127.0.0.1:3420/api/v2.0/indexers/all/results/torznab
export JACKETT_APIKEY=lodestarr
export RQBIT_URL=http://127.0.0.1:9030
export FLARESOLVERR_URL=http://127.0.0.1:8191   # optional, for 1337x
cargo run
```

The example `JACKETT_URL` asks one indexer. Replace `all` with an indexer id to search only that indexer.

## Development goals

- [x] Face-lift with CSS
- [x] Indicate which torrents are already grabbed
- [x] Indicate number of seeds/peer and other useful info
