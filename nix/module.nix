{ config, lib, pkgs, ... }:

let
  cfg = config.services.anorak;
in
{
  options.services.anorak = {
    enable = lib.mkEnableOption "Anorak";

    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.callPackage ./package.nix { };
      defaultText = "pkgs.callPackage ./package.nix { }";
      description = "Anorak package. The default builds this repository.";
    };

    port = lib.mkOption {
      type = lib.types.port;
      default = 9341;
      description = "Port Anorak listens on, inside its network namespace when one is set.";
    };

    jackettUrl = lib.mkOption {
      type = lib.types.str;
      example = "http://127.0.0.1:3420/api/v2.0/indexers/all/results/torznab";
      description = "Torznab results URL. Lodestarr and Jackett both speak this API.";
    };

    jackettApiKey = lib.mkOption {
      type = lib.types.str;
      description = "API key sent to the Torznab server. Lodestarr accepts any value.";
    };

    rqbitUrl = lib.mkOption {
      type = lib.types.str;
      default = "http://127.0.0.1:9030";
      example = "http://127.0.0.1:9030";
      description = "rqbit HTTP API. Grabs are posted to /torrents on this URL.";
    };

    openFirewall = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Open the listen port on the host firewall. Leave this off when the service is inside a network namespace.";
    };

    networkNamespace = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "nordvpn";
      description = ''
        Join this existing network namespace (`/run/netns/<name>`).
        The process then uses only that namespace's routes and
        `/etc/netns/<name>/resolv.conf`. Create the namespace before this
        service starts.
      '';
    };

    namespaceService = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "nordvpn-netns.service";
      description = "systemd unit that creates `networkNamespace`. This service waits for it and stops with it.";
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.services.anorak = {
      description = "Anorak";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ] ++ lib.optional (cfg.namespaceService != null) cfg.namespaceService;
      bindsTo = lib.optional (cfg.namespaceService != null) cfg.namespaceService;
      environment = {
        JACKETT_URL = cfg.jackettUrl;
        JACKETT_APIKEY = cfg.jackettApiKey;
        RQBIT_URL = cfg.rqbitUrl;
        ANORAK_PORT = toString cfg.port;
        RUST_LOG = "info";
      };
      serviceConfig = {
        ExecStart = lib.getExe cfg.package;
        WorkingDirectory = "${cfg.package}/share/anorak";
        Restart = "on-failure";
        RestartSec = "10s";
      } // lib.optionalAttrs (cfg.networkNamespace != null) {
        NetworkNamespacePath = "/run/netns/${cfg.networkNamespace}";
        BindReadOnlyPaths = [
          "/etc/netns/${cfg.networkNamespace}/resolv.conf:/etc/resolv.conf:norbind"
          "/etc/netns/${cfg.networkNamespace}/nsswitch.conf:/etc/nsswitch.conf:norbind"
        ];
        InaccessiblePaths = [ "-/run/nscd/socket" ];
      };
    };

    networking.firewall.allowedTCPPorts = lib.mkIf cfg.openFirewall [ cfg.port ];
  };
}
