cask "ayame" do
  arch arm: "aarch64", intel: "x86_64"

  version "0.7.1"
  sha256 arm:   "85fdba1e60809270d38547a3a1e493aa1e6eb81b65c88a6b8ee51e48bb34cbdf",
         intel: "ec5f695f27c720245fbb488565a0c1b69c443643174d7c221907b37191c699b2"

  url "https://github.com/hjosugi/ayame-editor/releases/download/v#{version}/ayame-v#{version}-macos-#{arch}.zip"
  name "Ayame Editor"
  desc "Desktop text editor for huge files"
  homepage "https://github.com/hjosugi/ayame-editor"

  app "Ayame.app"

  zap trash: [
    "~/Library/Caches/ayame",
  ]
end
