{
  description = "diesel-async development shell";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  inputs.nix-overlays.url = "github:nix-ocaml/nix-overlays";
  inputs.nix-overlays.inputs.nixpkgs.follows = "nixpkgs";

  outputs = { nixpkgs, ... }: {
    devShells = nixpkgs.lib.genAttrs nixpkgs.lib.systems.flakeExposed (system:
      let pkgs = import nixpkgs { inherit system; }; in {
        default = pkgs.mkShell {
          packages = with pkgs; [ cargo rustc rustfmt postgresql_14 pkg-config ];
          buildInputs = [ pkgs.openssl ];
        };
      });
  };
}
