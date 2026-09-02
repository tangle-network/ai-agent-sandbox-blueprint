{ pkgs }:

let
  lib = pkgs.lib;

  mkNpmCli = {
    name,
    version,
    npmPackage,
    entry,
    lockfileDir,
    npmDepsHash,
    binary ? name,
    src ? lockfileDir,
    installSourceTree ? false,
    restorePackageJsonFrom ? null,
    makeCacheWritable ? false,
    fetcherVersion ? 1,
    wrapperArgs ? "",
    license ? lib.licenses.unfree,
  }: pkgs.stdenv.mkDerivation {
    pname = name;
    inherit version src makeCacheWritable;

    nativeBuildInputs = [
      pkgs.nodejs_24
      pkgs.npmHooks.npmConfigHook
      pkgs.makeWrapper
    ];

    npmFlags = [ "--ignore-scripts" ];
    npmInstallFlags = [ "--ignore-scripts" ];
    dontNpmBuild = true;
    dontBuild = true;
    NIX_NPM_FETCHER_VERSION = toString fetcherVersion;

    npmDeps = pkgs.fetchNpmDeps {
      name = "${name}-npm-deps";
      src = lockfileDir;
      hash = npmDepsHash;
      inherit fetcherVersion;
    };

    installPhase = ''
      runHook preInstall

      ${if installSourceTree then ''
        runtime_root="$out/lib/${name}"
        mkdir -p "$runtime_root" "$out/bin"
        cp -r . "$runtime_root"
        ${lib.optionalString (restorePackageJsonFrom != null) ''
          mv "$runtime_root/${restorePackageJsonFrom}" "$runtime_root/package.json"
        ''}
      '' else ''
        mkdir -p "$out/lib" "$out/bin"
        cp -r node_modules "$out/lib/node_modules"
        runtime_root="$out/lib/node_modules/${npmPackage}"
      ''}

      chmod -R u+w,go-w "$out/lib"
      makeWrapper ${pkgs.nodejs_24}/bin/node "$out/bin/${binary}" \
        --add-flags "$runtime_root/${entry}" \
        ${wrapperArgs}

      runHook postInstall
    '';

    meta = {
      mainProgram = binary;
      inherit license;
      platforms = [ "x86_64-linux" ];
    };
  };

  openclawVersion = "2026.7.1-2";
  openclawSource = pkgs.stdenvNoCC.mkDerivation {
    pname = "openclaw-runtime-source";
    version = openclawVersion;
    src = pkgs.fetchurl {
      url = "https://registry.npmjs.org/openclaw/-/openclaw-${openclawVersion}.tgz";
      hash = "sha512-ycF3yPcbjN6bUPeaUx6Mh6vze1hQWoD3CT/wWcmD7a8xaHHHRUaAlaq+lFxMHf1ssEgODVAwjlzYqp2twkYZ7g==";
    };
    dontUnpack = true;
    installPhase = ''
      mkdir -p "$out"
      tar -xzf "$src" --strip-components=1 -C "$out"
      mv "$out/package.json" "$out/package.published.json"
      cp ${../npm-locks/openclaw/package.json} "$out/package.json"
      cp ${../npm-locks/openclaw/npm-shrinkwrap.json} "$out/npm-shrinkwrap.json"
    '';
  };
in {
  pi = mkNpmCli {
    name = "pi";
    version = "0.84.1";
    npmPackage = "@earendil-works/pi-coding-agent";
    entry = "dist/cli.js";
    lockfileDir = ../npm-locks/pi;
    npmDepsHash = "sha256-8qXEB429r954k2TG2PKQqtYCFtLQ+4CjV0cL+0e0Cdc=";
    makeCacheWritable = true;
    fetcherVersion = 2;
    license = lib.licenses.mit;
  };

  prime = mkNpmCli {
    name = "prime-agent";
    version = "0.7.2";
    npmPackage = "prime-agent";
    entry = "dist/bundle/cli.js";
    lockfileDir = ../npm-locks/prime;
    npmDepsHash = "sha256-pWRXFDNrmPAideo62eqUzC/17Z1y5jR9vcMIjKLpOv4=";
    wrapperArgs = ''
      --set-default PRIME_AGENT_INSTALL_UV 1 \
      --prefix PATH : ${lib.makeBinPath [ pkgs.uv pkgs.python313 ]}
    '';
    license = lib.licenses.mit;
  };

  openclaw = mkNpmCli {
    name = "openclaw";
    version = openclawVersion;
    npmPackage = "openclaw";
    entry = "openclaw.mjs";
    lockfileDir = ../npm-locks/openclaw;
    npmDepsHash = "sha256-iKFMyt8uchivDZn8pjbmUbnhsWecQJqakUpvUhkL41A=";
    src = openclawSource;
    installSourceTree = true;
    restorePackageJsonFrom = "package.published.json";
    license = lib.licenses.mit;
  };
}
