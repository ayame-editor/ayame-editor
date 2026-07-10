{
  description = "Ayame Editor development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  };

  outputs =
    { nixpkgs, ... }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      devShells = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
          linuxGuiPackages = pkgs.lib.optionals pkgs.stdenv.isLinux [
            pkgs.gtk3
            pkgs.webkitgtk_4_1
            pkgs.libayatana-appindicator
            pkgs.pkg-config
          ];
        in
        {
          default = pkgs.mkShell {
            packages =
              [
                pkgs.cargo
                pkgs.clippy
                pkgs.git
                pkgs.jq
                pkgs.mkdocs
                pkgs.nodejs_26
                pkgs.pnpm
                pkgs.python3
                pkgs.ruby
                pkgs.rustc
                pkgs.rustfmt
              ]
              ++ linuxGuiPackages;

            shellHook = ''
              export CARGO_TERM_COLOR=always
            '';
          };
        }
      );

      formatter = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        pkgs.nixfmt
      );
    };
}
