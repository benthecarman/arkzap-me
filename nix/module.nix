self:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.arkzap-me;
  inherit (lib)
    mkEnableOption
    mkIf
    mkOption
    types
    ;
in
{
  options.services.arkzap-me = {
    enable = mkEnableOption "arkzap-me LNURL server";

    package = mkOption {
      type = types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "self.packages.\${pkgs.stdenv.hostPlatform.system}.default";
      description = "The arkzap-me package to run.";
    };

    environmentFiles = mkOption {
      type = types.listOf types.str;
      default = [ ];
      example = [ "/run/secrets/arkzap-me.env" ];
      description = "Files containing environment variables, including secrets.";
    };

    environment = mkOption {
      type = types.attrsOf types.str;
      default = { };
      example = {
        LNURL_DOMAIN = "example.com";
        LNURL_BARKD_URL = "http://127.0.0.1:3535";
      };
      description = "Non-secret environment variables for arkzap-me.";
    };

    port = mkOption {
      type = types.port;
      default = 3000;
      description = "TCP port on which arkzap-me listens.";
    };

    openFirewall = mkOption {
      type = types.bool;
      default = false;
      description = "Whether to open the listening port in the firewall.";
    };

    user = mkOption {
      type = types.str;
      default = "arkzap-me";
      description = "User under which the service runs.";
    };

    group = mkOption {
      type = types.str;
      default = "arkzap-me";
      description = "Group under which the service runs.";
    };

  };

  config = mkIf cfg.enable {
    assertions = [
      {
        assertion =
          cfg.environmentFiles != [ ]
          || (
            cfg.environment ? LNURL_PG_URL && cfg.environment ? LNURL_NSEC && cfg.environment ? LNURL_BARKD_URL
          );
        message = "arkzap-me requires environmentFiles or LNURL_PG_URL, LNURL_NSEC, and LNURL_BARKD_URL in environment";
      }
    ];

    users.users = mkIf (cfg.user == "arkzap-me") {
      arkzap-me = {
        isSystemUser = true;
        group = cfg.group;
      };
    };
    users.groups = mkIf (cfg.group == "arkzap-me") { arkzap-me = { }; };

    networking.firewall.allowedTCPPorts = mkIf cfg.openFirewall [ cfg.port ];

    systemd.services.arkzap-me = {
      description = "arkzap-me LNURL server";
      wantedBy = [ "multi-user.target" ];
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];

      environment = {
        LNURL_BIND = "0.0.0.0";
        LNURL_PORT = toString cfg.port;
        RUST_LOG = "info";
      }
      // cfg.environment;

      serviceConfig = {
        ExecStart = lib.getExe cfg.package;
        EnvironmentFile = cfg.environmentFiles;
        User = cfg.user;
        Group = cfg.group;
        Restart = "on-failure";
        RestartSec = "5s";

        CapabilityBoundingSet = "";
        LockPersonality = true;
        NoNewPrivileges = true;
        PrivateDevices = true;
        PrivateTmp = true;
        ProtectClock = true;
        ProtectControlGroups = true;
        ProtectHome = true;
        ProtectHostname = true;
        ProtectKernelLogs = true;
        ProtectKernelModules = true;
        ProtectKernelTunables = true;
        ProtectSystem = "strict";
        RestrictAddressFamilies = [
          "AF_INET"
          "AF_INET6"
          "AF_UNIX"
        ];
        RestrictNamespaces = true;
        RestrictRealtime = true;
        SystemCallArchitectures = "native";
        UMask = "0077";
      };
    };
  };
}
