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
    mkIf
    mkOption
    nameValuePair
    types
    ;
in
{
  options.services.arkzap-me.instances = mkOption {
    type = types.attrsOf (
      types.submodule (
        { name, ... }:
        {
          options = {
            package = mkOption {
              type = types.package;
              default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
              defaultText =
                lib.literalExpression "self.packages.\${pkgs.stdenv.hostPlatform.system}.default";
              description = "The arkzap-me package to run.";
            };

            bind = mkOption {
              type = types.str;
              default = "0.0.0.0";
              description = "Address on which arkzap-me listens.";
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
              default = "arkzap-me-${name}";
              description = "User under which the service runs.";
            };

            group = mkOption {
              type = types.str;
              default = "arkzap-me-${name}";
              description = "Group under which the service runs.";
            };
          };
        }
      )
    );
    default = { };
    description = "Named arkzap-me LNURL server instances.";
  };

  config = mkIf (cfg.instances != { }) {
    assertions = lib.mapAttrsToList (
      name: instance: {
        assertion =
          instance.environmentFiles != [ ]
          || (
            instance.environment ? LNURL_PG_URL
            && instance.environment ? LNURL_NSEC
            && instance.environment ? LNURL_BARKD_URL
          );
        message =
          "arkzap-me instance ${name} requires environmentFiles or LNURL_PG_URL, LNURL_NSEC, "
          + "and LNURL_BARKD_URL in environment";
      }
    ) cfg.instances;

    users.users = lib.mkMerge (
      lib.mapAttrsToList (
        name: instance:
        mkIf (instance.user == "arkzap-me-${name}") {
          ${instance.user} = {
            isSystemUser = true;
            group = instance.group;
          };
        }
      ) cfg.instances
    );

    users.groups = lib.mkMerge (
      lib.mapAttrsToList (
        name: instance:
        mkIf (instance.group == "arkzap-me-${name}") { ${instance.group} = { }; }
      ) cfg.instances
    );

    networking.firewall.allowedTCPPorts = lib.mapAttrsToList (
      _: instance: instance.port
    ) (lib.filterAttrs (_: instance: instance.openFirewall) cfg.instances);

    systemd.services = lib.mapAttrs' (
      name: instance:
      nameValuePair "arkzap-me-${name}" {
        description = "arkzap-me LNURL server";
        wantedBy = [ "multi-user.target" ];
        after = [ "network-online.target" ];
        wants = [ "network-online.target" ];

        environment = {
          LNURL_BIND = instance.bind;
          LNURL_PORT = toString instance.port;
          RUST_LOG = "info";
        }
        // instance.environment;

        serviceConfig = {
          ExecStart = lib.getExe instance.package;
          EnvironmentFile = instance.environmentFiles;
          User = instance.user;
          Group = instance.group;
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
      }
    ) cfg.instances;
  };
}
