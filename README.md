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
cargo run
```

The example `JACKETT_URL` asks one indexer. Replace `all` with an indexer id to search only that indexer.

## Development goals

- [x] Face-lift with CSS
- [x] Indicate which torrents are already grabbed
- [x] Indicate number of seeds/peer and other useful info
